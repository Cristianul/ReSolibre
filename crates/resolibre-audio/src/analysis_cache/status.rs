//! Restores Library badges progressively without starting decoders or retaining spectrograms.

use super::{CacheConfig, unchanged};
use crate::original_audio::SourceKey;
use resolibre_storage::{AnalysisCache, audio_identity};
use std::{
    fs::File,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
};

pub(crate) struct StatusJob {
    cancel: Arc<AtomicBool>,
    completed: Receiver<u64>,
    thread: Option<JoinHandle<()>>,
}

impl StatusJob {
    /// Takes at most one bounded queue snapshot; all file/hash/database work stays on this owned worker.
    pub(crate) fn start(
        entries: Vec<(u64, PathBuf)>,
        config: Arc<CacheConfig>,
    ) -> Result<Self, ()> {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        // At most one small message per occurrence; the queue's own size bounds channel memory.
        let (sender, completed) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("resolibre-analysis-status".into())
            .spawn(move || {
                let Some(mut database) = config.database() else {
                    return;
                };
                match database.has_tracks() {
                    Ok(false) => return,
                    Ok(true) => {}
                    Err(_) => {
                        config.warn();
                        return;
                    }
                }
                for (id, path) in entries {
                    if worker_cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    if cached_track(&path, &mut database, &config, &worker_cancel)
                        && sender.send(id).is_err()
                    {
                        return;
                    }
                }
            })
            .map_err(|_| ())?;
        Ok(Self {
            cancel,
            completed,
            thread: Some(thread),
        })
    }

    /// Reaps only finished work, then drains small observations without blocking the transport owner.
    pub(crate) fn poll(&mut self) -> Vec<u64> {
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
            && let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("warn: cached analysis status unavailable");
        }
        self.completed.try_iter().collect()
    }

    /// Reports completion only after the last observations can be drained by poll.
    pub(crate) fn finished(&self) -> bool {
        self.thread.is_none()
    }
}

impl Drop for StatusJob {
    /// Shutdown cancels and reaps hashing; normal polling never joins an active worker.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Revalidates read access and file facts; legacy caches are indexed once with their original audio hash.
fn cached_track(
    path: &Path,
    database: &mut AnalysisCache,
    config: &CacheConfig,
    cancel: &AtomicBool,
) -> bool {
    let Ok(file) = File::options()
        .read(true)
        .custom_flags(crate::OPEN_NONBLOCK | 0o400000)
        .open(path)
    else {
        return false;
    };
    let Ok(key) = SourceKey::read(&file, 0) else {
        return false;
    };
    let facts = key.metadata_key();
    let identity = match database.track_source(&facts) {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            let Ok(input) = file.try_clone() else {
                return false;
            };
            let Some(identity) = audio_identity::identify(input, cancel) else {
                return false;
            };
            if !unchanged(&file, key, cancel) {
                return false;
            }
            if database.save_track_source(&facts, &identity).is_err() {
                config.warn();
            }
            identity
        }
        Err(_) => {
            config.warn();
            return false;
        }
    };
    let complete = match database.has_track(&identity) {
        Ok(complete) => complete,
        Err(_) => {
            config.warn();
            false
        }
    };
    complete && unchanged(&file, key, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cached badges recheck file facts/access; links, replacements and cancelled reads cannot reuse a hint.
    #[test]
    fn cached_badge_rejects_changed_missing_linked_and_cancelled_sources() {
        let directory =
            std::env::temp_dir().join(format!("resolibre-badge-facts-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("source.flac");
        std::fs::write(
            &path,
            include_bytes!("../../tests/fixtures/durationless.flac"),
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        let identity = audio_identity::identify(File::open(&path).unwrap(), &cancel).unwrap();
        let config = CacheConfig::new(directory.join("cache"));
        let mut database = config.database().unwrap();
        let mut analyzer = resolibre_core::analysis::TrackAnalyzer::new(8000, 1).unwrap();
        analyzer.push(&[0.0; 8000]).unwrap();
        database
            .save_track(&identity, &analyzer.finish().unwrap())
            .unwrap();

        assert!(
            cached_track(&path, &mut database, &config, &cancel),
            "older reports are indexed"
        );
        let key = SourceKey::read(&File::open(&path).unwrap(), 0).unwrap();
        assert_eq!(
            database
                .track_source(&key.metadata_key())
                .unwrap()
                .unwrap()
                .digest,
            identity.digest
        );
        assert!(
            cached_track(&path, &mut database, &config, &cancel),
            "unchanged indexed files restore"
        );
        let link = directory.join("link.flac");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(!cached_track(&link, &mut database, &config, &cancel));
        cancel.store(true, Ordering::Relaxed);
        assert!(!cached_track(&path, &mut database, &config, &cancel));
        cancel.store(false, Ordering::Relaxed);
        std::fs::write(&path, b"replaced source").unwrap();
        assert!(!cached_track(&path, &mut database, &config, &cancel));
        std::fs::remove_file(&path).unwrap();
        assert!(!cached_track(&path, &mut database, &config, &cancel));
        drop(database);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
