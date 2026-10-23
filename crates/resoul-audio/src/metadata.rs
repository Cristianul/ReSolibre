//! Read-only tag and embedded-cover probing, isolated from playback and UI threads.

mod process;
#[cfg(test)]
mod tests;

use crate::edge_analysis::SourceKey;
use serde_json::Value;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Instant;

pub const COVER_SIDE: u32 = 512;
const COVER_BYTES: usize = COVER_SIDE as usize * COVER_SIDE as usize * 4;

/// Validated square RGBA thumbnail; no path, encoded image or native handle reaches the UI.
#[derive(Clone, Debug)]
pub struct Cover {
    pub rgba: Arc<[u8]>,
}

pub(crate) struct Details {
    pub title: Option<String>,
    pub search_text: String,
    pub duration_seconds: Option<u32>,
    pub cover: Option<Arc<Cover>>,
}

pub(crate) struct Job<T = Details> {
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<T, ()>>>,
}

impl Job {
    /// Probes one local source; one session owns at most one job, with no detached work.
    #[cfg(test)]
    pub(crate) fn start(path: PathBuf, cover: bool) -> Result<Self, ()> {
        Self::start_cached(path, cover, None)
    }

    /// Reuses unchanged metadata in a background job; only a current cover requires image probing.
    pub(crate) fn start_cached(
        path: PathBuf,
        cover: bool,
        cache: Option<Arc<crate::analysis_cache::CacheConfig>>,
    ) -> Result<Self, ()> {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let thread = thread::Builder::new()
            .name("resolibre-metadata".into())
            .spawn(move || inspect_cached(path, cover, &worker_cancel, cache.as_deref()))
            .map_err(|_| ())?;
        Ok(Self {
            cancel,
            thread: Some(thread),
        })
    }
}

impl<T> Job<T> {
    /// Returns a completed result once; callers never wait for decoding here.
    pub(crate) fn poll(&mut self) -> Option<Result<T, ()>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        Some(self.thread.take()?.join().unwrap_or(Err(())))
    }
}

impl<T> Drop for Job<T> {
    /// Cancels bounded subprocess I/O and reaps the owned worker before releasing it.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Revalidates regular-file facts on cache hits and after probes; storage failure falls back safely.
fn inspect_cached(
    path: PathBuf,
    want_cover: bool,
    cancel: &AtomicBool,
    config: Option<&crate::analysis_cache::CacheConfig>,
) -> Result<Details, ()> {
    let started = Instant::now();
    // Linux O_NONBLOCK/O_NOFOLLOW: never wait on a FIFO or follow an imported symlink.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(crate::OPEN_NONBLOCK | 0o400000)
        .open(path)
        .map_err(|_| ())?;
    let key = SourceKey::read(&file, 0).map_err(|_| ())?;
    let mut database = config.and_then(|config| config.metadata_database());
    if !want_cover && let Some(database) = &database {
        match database.metadata(&key.metadata_key()) {
            Ok(Some(record)) => {
                if !crate::analysis_cache::unchanged(&file, key, cancel) {
                    return Err(());
                }
                return Ok(Details {
                    title: record.title,
                    search_text: record.search_text,
                    duration_seconds: record.duration_seconds,
                    cover: None,
                });
            }
            Ok(None) => {}
            Err(_) => {
                if let Some(config) = config {
                    config.warn();
                }
            }
        }
    }
    let bytes = process::probe(&file, cancel, started)?;
    let json: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
    let title = track_title(&json);
    let search_text = search_tags(&json);
    let cover = if want_cover {
        cover_stream(&json).and_then(|stream| {
            let rgba = process::cover(&file, stream, cancel, started).ok()?;
            if rgba.len() != COVER_BYTES {
                return None;
            }
            Some(Arc::new(Cover { rgba: rgba.into() }))
        })
    } else {
        None
    };
    if cancel.load(Ordering::Relaxed) || SourceKey::read(&file, 0).map_err(|_| ())? != key {
        return Err(());
    }
    if let Some(database) = &mut database {
        let record = resolibre_storage::metadata::Metadata {
            title: title.clone(),
            search_text: search_text.clone(),
            duration_seconds: track_duration(&json),
        };
        if database
            .save_metadata(&key.metadata_key(), &record)
            .is_err()
            && let Some(config) = config
        {
            config.warn();
        }
    }
    Ok(Details {
        title,
        search_text,
        duration_seconds: track_duration(&json),
        cover,
    })
}

/// Prefers first-audio-stream timing, then container timing; ignores individual video streams.
fn track_duration(json: &Value) -> Option<u32> {
    let audio = json["streams"]
        .as_array()?
        .iter()
        .find(|stream| stream["codec_type"] == "audio")?;
    duration_seconds(&audio["duration"]).or_else(|| duration_seconds(&json["format"]["duration"]))
}

/// Accepts finite nonnegative seconds within u32 bounds; display precision is whole seconds.
fn duration_seconds(value: &Value) -> Option<u32> {
    let seconds = match value.as_str() {
        Some(text) => text.parse::<f64>().ok()?,
        None => value.as_f64()?,
    };
    if !seconds.is_finite() || seconds < 0.0 || seconds > f64::from(u32::MAX) {
        return None;
    }
    Some(seconds.floor() as u32)
}

/// Uses container tags, falling back to the first audio stream; title-less artist tags use filename.
fn track_title(json: &Value) -> Option<String> {
    let format = &json["format"]["tags"];
    let audio = json["streams"]
        .as_array()?
        .iter()
        .find(|stream| stream["codec_type"] == "audio")?;
    let title = tag(format, "title").or_else(|| tag(&audio["tags"], "title"))?;
    let artist = tag(format, "artist").or_else(|| tag(&audio["tags"], "artist"));
    Some(match artist {
        Some(artist) => format!("{artist} – {title}"),
        None => title,
    })
}

/// Collects only searchable music tags, with container precedence and the existing per-field bounds.
fn search_tags(json: &Value) -> String {
    let audio = json["streams"].as_array().and_then(|streams| {
        streams
            .iter()
            .find(|stream| stream["codec_type"] == "audio")
    });
    let Some(audio) = audio else {
        return String::new();
    };
    let mut fields = Vec::new();
    for name in [
        "artist",
        "title",
        "album",
        "album_artist",
        "date",
        "year",
        "disc",
        "discnumber",
    ] {
        if let Some(value) =
            tag(&json["format"]["tags"], name).or_else(|| tag(&audio["tags"], name))
        {
            fields.push(value);
        }
    }
    fields.join("\n").to_lowercase()
}

/// Bounds Unicode text and neutralizes control/bidi characters before plain-text presentation.
fn tag(tags: &Value, name: &str) -> Option<String> {
    let value = tags
        .as_object()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))?
        .1
        .as_str()?;
    let value: String = value
        .chars()
        .take(160)
        .map(|character| {
            if character.is_control()
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                ' '
            } else {
                character
            }
        })
        .collect();
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

/// Selects an actual attached picture, never a video frame; reject oversized source dimensions.
fn cover_stream(json: &Value) -> Option<u32> {
    json["streams"].as_array()?.iter().find_map(|stream| {
        if stream["disposition"]["attached_pic"].as_u64()? != 1 {
            return None;
        }
        let width = stream["width"].as_u64()?;
        let height = stream["height"].as_u64()?;
        if width == 0 || height == 0 || width > 4096 || height > 4096 || width * height > 4_194_304
        {
            return None;
        }
        u32::try_from(stream["index"].as_u64()?).ok()
    })
}

/// Small startup records, loaded together so cached rows do not wait one transport tick each.
pub(crate) type WarmMetadata = Vec<(u64, Details)>;

impl Job<WarmMetadata> {
    /// Restores only unchanged metadata; never hashes, probes or decodes music during warmup.
    pub(crate) fn warm(
        entries: Vec<(u64, PathBuf)>,
        config: Arc<crate::analysis_cache::CacheConfig>,
    ) -> Result<Self, ()> {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let thread = thread::Builder::new()
            .name("resolibre-metadata-load".into())
            .spawn(move || {
                let Some(database) = config.metadata_database() else {
                    return Ok(Vec::new());
                };
                let mut records = Vec::new();
                for (id, path) in entries {
                    if worker_cancel.load(Ordering::Relaxed) {
                        return Err(());
                    }
                    let Ok(file) = OpenOptions::new()
                        .read(true)
                        .custom_flags(crate::OPEN_NONBLOCK | 0o400000)
                        .open(path)
                    else {
                        continue;
                    };
                    let Ok(key) = SourceKey::read(&file, 0) else {
                        continue;
                    };
                    match database.metadata(&key.metadata_key()) {
                        Ok(Some(record))
                            if crate::analysis_cache::unchanged(&file, key, &worker_cancel) =>
                        {
                            records.push((
                                id,
                                Details {
                                    title: record.title,
                                    search_text: record.search_text,
                                    duration_seconds: record.duration_seconds,
                                    cover: None,
                                },
                            ));
                        }
                        Err(_) => {
                            config.warn();
                        }
                        _ => {}
                    }
                }
                Ok(records)
            })
            .map_err(|_| ())?;
        Ok(Self {
            cancel,
            thread: Some(thread),
        })
    }
}
