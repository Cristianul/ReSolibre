//! Metadata-independent identity for original encoded audio, without decoding or playback processing.

mod mp4;

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use symphonia::core::{
    common::Limit,
    formats::{FormatOptions, TrackType, probe::Hint},
    io::{MediaSource, MediaSourceStream},
    meta::{MetadataOptions, StandardTag},
};

const MAX_BYTES: u64 = 512 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(2);

/// A content key plus descriptive tags. Tags never contribute to the digest.
#[derive(Clone, Debug)]
pub struct AudioIdentity {
    pub digest: [u8; 32],
    pub artist: String,
    pub title: String,
}

struct BoundedSource<'a> {
    file: File,
    cancel: &'a AtomicBool,
    started: Instant,
    read_bytes: u64,
    phase_bytes: &'a AtomicU64,
}

impl BoundedSource<'_> {
    /// Stops identity work independently of analysis so unsupported/large inputs can still play.
    fn check(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed)
            || self.started.elapsed() >= DEADLINE
            || self.read_bytes > MAX_BYTES
            || self.phase_bytes.load(Ordering::Relaxed) >= 16 * 1024 * 1024
        {
            return Err(io::Error::other("Audio identity limit"));
        }
        Ok(())
    }
}

impl Read for BoundedSource<'_> {
    /// Bounds actual bytes read, including repeated probes; never grants writes to the source.
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        let limit = bytes.len().min(64 * 1024);
        let count = self.file.read(&mut bytes[..limit])?;
        self.read_bytes += count as u64;
        self.phase_bytes.fetch_add(count as u64, Ordering::Relaxed);
        Ok(count)
    }
}

impl Seek for BoundedSource<'_> {
    /// Keeps cancellation active during demuxer seeks as well as reads.
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.check()?;
        self.file.seek(position)
    }
}

impl MediaSource for BoundedSource<'_> {
    /// Only regular, seekable local files enter this adapter.
    fn is_seekable(&self) -> bool {
        true
    }

    /// Supplies the retained file size without guessing from an extension or external URL.
    fn byte_len(&self) -> Option<u64> {
        self.file.metadata().ok().map(|metadata| metadata.len())
    }
}

/// Hashes one complete encoded audio stream and timing/codec interpretation, excluding tags/artwork.
/// The caller must supply an independent cursor and revalidate source identity before using the result.
/// Unsupported, multi-audio-stream and over-budget inputs return None, never a partial digest.
pub fn identify(file: File, cancel: &AtomicBool) -> Option<AudioIdentity> {
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return None;
    }
    let phase_bytes = AtomicU64::new(0);
    let mut source = BoundedSource {
        file,
        cancel,
        started: Instant::now(),
        read_bytes: 0,
        phase_bytes: &phase_bytes,
    };
    let (is_mp4, container_timing) = mp4::read(&mut source)?;
    source.seek(SeekFrom::Start(0)).ok()?;
    let (artist, title) = riff_labels(&mut source).unwrap_or_default();
    source.seek(SeekFrom::Start(0)).ok()?;
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let options = MetadataOptions::default()
        .limit_tag_bytes(Limit::Maximum(4096))
        .limit_visual_bytes(Limit::Maximum(0));
    let mut format = symphonia::default::get_probe()
        .probe(&Hint::new(), stream, FormatOptions::default(), options)
        .ok()?;
    if format.format_info().short_name == "isomp4" && !is_mp4 {
        return None;
    }
    let audio_tracks: Vec<_> = format
        .tracks()
        .iter()
        .filter(|track| track.track_type() == Some(TrackType::Audio))
        .collect();
    // Native and portable demuxers do not share stream IDs. Never guess their mapping.
    if audio_tracks.len() != 1 {
        return None;
    }
    let track = audio_tracks[0];
    let track_id = track.id;
    let mut hash = Sha256::new();
    hash.update(b"ReSolibre encoded audio identity 1\0");
    hash.update((container_timing.len() as u64).to_le_bytes());
    hash.update(container_timing);
    // Only technical fields; language, title, flags, cover and other descriptive metadata are excluded.
    hash.update(format!(
        "{:?}/{:?}/{:?}/{:?}",
        track.time_base, track.start_ts, track.delay, track.padding
    ));
    let mut identity = AudioIdentity {
        digest: [0; 32],
        artist,
        title,
    };
    let mut packets = 0_u64;
    let mut metadata_seen = false;
    loop {
        read_metadata(
            &mut format.metadata(),
            &mut identity,
            &mut metadata_seen,
            track_id,
            &mut hash,
        )?;
        // Bound progressive parser allocations even before a complete packet is returned.
        phase_bytes.store(0, Ordering::Relaxed);
        let packet = format.next_packet().ok()?;
        let Some(packet) = packet else { break };
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        if packet.track_id != track_id {
            continue;
        }
        if packet.data.len() > 16 * 1024 * 1024 {
            return None;
        }
        hash.update(packet.pts.get().to_le_bytes());
        hash.update(packet.dur.get().to_le_bytes());
        hash.update(packet.trim_start.get().to_le_bytes());
        hash.update(packet.trim_end.get().to_le_bytes());
        hash.update((packet.data.len() as u64).to_le_bytes());
        hash.update(&packet.data);
        packets += 1;
    }
    if packets == 0 || cancel.load(Ordering::Relaxed) {
        return None;
    }
    // Some formats discover codec setup while reading initial packets. Hash the final technical setup.
    let mut parameters = format
        .tracks()
        .iter()
        .find(|track| track.id == track_id)?
        .codec_params
        .as_ref()?
        .audio()?
        .clone();
    if parameters
        .extra_data
        .as_ref()
        .is_some_and(|data| data.len() > 1024 * 1024)
    {
        return None;
    }
    let extra = parameters.extra_data.take().unwrap_or_default();
    hash.update(format!("{parameters:?}"));
    hash.update((extra.len() as u64).to_le_bytes());
    hash.update(extra);
    hash.update(packets.to_le_bytes());
    identity.digest = hash.finalize().into();
    Some(identity)
}

/// Keeps identifying text bounded and safe to display; labels carry no lookup authority.
fn label(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect()
}

/// Reads small RIFF INFO labels because Symphonia 0.6.1 discards its parsed WAV INFO log.
/// This fallback is descriptive only: chunk bytes and labels never enter content hashing.
fn riff_labels(source: &mut BoundedSource<'_>) -> Option<(String, String)> {
    let mut header = [0; 12];
    source.read_exact(&mut header).ok()?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return None;
    }
    let end =
        (u64::from(u32::from_le_bytes(header[4..8].try_into().ok()?)) + 8).min(source.byte_len()?);
    let mut artist = String::new();
    let mut title = String::new();
    for _ in 0..128 {
        let position = source.stream_position().ok()?;
        if position + 8 > end {
            break;
        }
        let mut chunk = [0; 8];
        source.read_exact(&mut chunk).ok()?;
        let size = u64::from(u32::from_le_bytes(chunk[4..].try_into().ok()?));
        let next = position + 8 + size + size % 2;
        if next > end {
            return None;
        }
        if &chunk[..4] == b"LIST" && (4..=65536).contains(&size) {
            let mut bytes = vec![0; size as usize];
            source.read_exact(&mut bytes).ok()?;
            if &bytes[..4] == b"INFO" {
                let mut offset = 4;
                while offset + 8 <= bytes.len() {
                    let length =
                        u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
                    let value_start = offset + 8;
                    let value_end = value_start.checked_add(length)?;
                    let value = bytes.get(value_start..value_end)?;
                    if length <= 4096 {
                        if &bytes[offset..offset + 4] == b"IART" {
                            artist = label(&String::from_utf8_lossy(value));
                        }
                        if &bytes[offset..offset + 4] == b"INAM" {
                            title = label(&String::from_utf8_lossy(value));
                        }
                    }
                    offset = value_end + length % 2;
                }
            }
        }
        source.seek(SeekFrom::Start(next)).ok()?;
    }
    Some((artist, title))
}

/// Processes each metadata revision once; artist/title stay separate from timing-relevant trim tags.
fn read_metadata(
    metadata: &mut symphonia::core::meta::Metadata<'_>,
    identity: &mut AudioIdentity,
    seen: &mut bool,
    track_id: u32,
    hash: &mut Sha256,
) -> Option<()> {
    loop {
        if !*seen && let Some(revision) = metadata.current() {
            *seen = true;
            read_tags(&revision.media.tags, identity, hash)?;
            for entry in &revision.per_track {
                if entry.track_id == u64::from(track_id) {
                    read_tags(&entry.metadata.tags, identity, hash)?;
                }
            }
        }
        if metadata.pop().is_none() {
            break;
        }
        *seen = false;
    }
    Some(())
}

/// Copies bounded descriptive labels and hashes only a known gapless-timing tag that native decoding may honor.
fn read_tags(
    tags: &[symphonia::core::meta::Tag],
    identity: &mut AudioIdentity,
    hash: &mut Sha256,
) -> Option<()> {
    for tag in tags {
        if tag.raw.key.to_ascii_lowercase().contains("itunsmpb") {
            let trim = tag.raw.value.to_string();
            if trim.len() > 4096 {
                return None;
            }
            hash.update(b"iTunSMPB");
            hash.update((trim.len() as u64).to_le_bytes());
            hash.update(trim);
        }
        match &tag.std {
            Some(StandardTag::Artist(value)) => identity.artist = label(value.as_str()),
            Some(StandardTag::TrackTitle(value)) => identity.title = label(value.as_str()),
            _ => {}
        }
    }
    Some(())
}
