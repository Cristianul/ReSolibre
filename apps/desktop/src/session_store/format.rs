//! Strict versioned local-session format. Platform-tagged paths retain native units, not display labels.

use super::StoredSession;
use crate::interface_state::InterfaceState;
use crate::path_encoding;
use resolibre_audio::session::SavedSession;
use resolibre_core::equalizer::Equalizer;
use resolibre_core::playback_edges::{FadeMode, PlaybackEdges};
use resolibre_core::queue::{MAX_QUEUE_ITEMS, RepeatMode};
use resolibre_core::session::Volume;
use std::path::PathBuf;

pub(super) const MAX_BYTES: u64 = (path_encoding::MAX_SELECTION_BYTES
    + MAX_QUEUE_ITEMS * 4
    + 133
    + if cfg!(windows) { 8 } else { 0 }) as u64;
#[cfg(unix)]
const MAGIC: &[u8] = b"ReSolibre-Session-4\n";
#[cfg(windows)]
const MAGIC: &[u8] = b"ReSolibre-Windows-Session-4\n";
#[cfg(unix)]
const V3_MAGIC: &[u8] = b"ReSolibre-Session-3\n";
#[cfg(windows)]
const V3_MAGIC: &[u8] = b"ReSolibre-Windows-Session-3\n";
#[cfg(unix)]
const V2_MAGIC: &[u8] = b"ReSolibre-Session-2\n";
#[cfg(windows)]
const V2_MAGIC: &[u8] = b"ReSolibre-Windows-Session-2\n";
#[cfg(unix)]
const LEGACY_MAGIC: &[u8] = b"ReSolibre-Session-1\n";
#[cfg(windows)]
const LEGACY_MAGIC: &[u8] = b"ReSolibre-Windows-Session-1\n";
const INVALID: &str = "Invalid saved session; previous file preserved";

/// Encodes validated intent with length-prefixed paths; never serializes native filter syntax.
pub(super) fn encode(stored: &StoredSession) -> Result<Vec<u8>, &'static str> {
    let saved = &stored.audio;
    saved.validate()?;
    let mut bytes = MAGIC.to_vec();
    bytes.extend(saved.volume.percent().to_le_bytes());
    bytes.push(u8::from(saved.equalizer.enabled()));
    bytes.extend(saved.equalizer.preamp().to_le_bytes());
    for band in saved.equalizer.bands() {
        bytes.extend(band.to_le_bytes());
    }
    bytes.push(u8::from(saved.shuffle));
    bytes.push(match saved.repeat {
        RepeatMode::Off => 0,
        RepeatMode::All => 1,
        RepeatMode::One => 2,
    });
    bytes.extend(
        saved
            .current
            .map(|index| index as u32)
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    bytes.extend((saved.paths.len() as u32).to_le_bytes());
    for path in saved.paths.iter() {
        let path = path_encoding::encode(path.as_os_str());
        bytes.extend((path.len() as u32).to_le_bytes());
        bytes.extend(path);
    }
    bytes.push(u8::from(saved.edges.trim));
    bytes.push(match saved.edges.fade {
        FadeMode::Off => 0,
        FadeMode::In => 1,
        FadeMode::Out => 2,
        FadeMode::Both => 3,
    });
    bytes.push(u8::from(stored.interface.spectrum));
    bytes.push(u8::from(stored.interface.equalizer_visible));
    bytes.push(u8::from(stored.interface.library_visible));
    bytes.push(u8::from(saved.mix_enabled));
    Ok(bytes)
}

/// Rejects truncation, unknown versions, extra data and invalid values before restoring any state.
pub(super) fn decode(bytes: &[u8]) -> Result<StoredSession, &'static str> {
    if bytes.len() as u64 > MAX_BYTES
        || !(bytes.starts_with(MAGIC)
            || bytes.starts_with(V3_MAGIC)
            || bytes.starts_with(V2_MAGIC)
            || bytes.starts_with(LEGACY_MAGIC))
    {
        return Err(INVALID);
    }
    let mut reader = Reader {
        remaining: &bytes[MAGIC.len()..],
    };
    let volume = Volume::from_percent(reader.number()?).ok_or(INVALID)?;
    let enabled = reader.boolean()?;
    let preamp = reader.number()?;
    let mut bands = [0.0; 10];
    for band in &mut bands {
        *band = reader.number()?;
    }
    let equalizer = if bytes.starts_with(LEGACY_MAGIC) {
        Equalizer::from_saved_v1(enabled, bands, preamp)
    } else {
        Equalizer::new(enabled, bands, preamp)
    }
    .map_err(|_| INVALID)?;
    let shuffle = reader.boolean()?;
    let repeat = match reader.take(1)?[0] {
        0 => RepeatMode::Off,
        1 => RepeatMode::All,
        2 => RepeatMode::One,
        _ => return Err(INVALID),
    };
    let selected = reader.integer()?;
    let current = if selected == u32::MAX {
        None
    } else {
        Some(selected as usize)
    };
    let count = reader.integer()? as usize;
    if count > MAX_QUEUE_ITEMS {
        return Err(INVALID);
    }
    let mut paths = Vec::with_capacity(count);
    let mut total = 0_usize;
    for _ in 0..count {
        let length = reader.integer()? as usize;
        total = total.saturating_add(length);
        if length == 0
            || length > path_encoding::MAX_PATH_BYTES
            || total > path_encoding::MAX_SELECTION_BYTES
        {
            return Err(INVALID);
        }
        let path = path_encoding::decode(reader.take(length)?).ok_or(INVALID)?;
        paths.push(PathBuf::from(path));
    }
    let edges = if bytes.starts_with(LEGACY_MAGIC) {
        PlaybackEdges::default()
    } else {
        let trim = reader.boolean()?;
        let fade = match reader.take(1)?[0] {
            0 => FadeMode::Off,
            1 => FadeMode::In,
            2 => FadeMode::Out,
            3 => FadeMode::Both,
            _ => return Err(INVALID),
        };
        PlaybackEdges { trim, fade }
    };
    let interface = if bytes.starts_with(MAGIC) || bytes.starts_with(V3_MAGIC) {
        InterfaceState {
            spectrum: reader.boolean()?,
            equalizer_visible: reader.boolean()?,
            library_visible: reader.boolean()?,
        }
    } else {
        InterfaceState::default()
    };
    let mix_enabled = if bytes.starts_with(MAGIC) {
        reader.boolean()?
    } else {
        false
    };
    if !reader.remaining.is_empty() {
        return Err(INVALID);
    }
    let saved = SavedSession {
        paths: paths.into(),
        current,
        volume,
        equalizer,
        edges,
        mix_enabled,
        shuffle,
        repeat,
    };
    saved.validate().map_err(|_| INVALID)?;
    Ok(StoredSession {
        audio: saved,
        interface,
    })
}

struct Reader<'a> {
    remaining: &'a [u8],
}

impl<'a> Reader<'a> {
    /// Consumes only bytes present in the bounded input, returning a safe error on truncation.
    fn take(&mut self, count: usize) -> Result<&'a [u8], &'static str> {
        let (value, rest) = self.remaining.split_at_checked(count).ok_or(INVALID)?;
        self.remaining = rest;
        Ok(value)
    }
    /// Reads a fixed-width numeric gain; its domain constructor enforces range and finiteness.
    fn number(&mut self) -> Result<f64, &'static str> {
        Ok(f64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| INVALID)?,
        ))
    }
    /// Reads one bounded length/index field without string parsing or lossy path conversion.
    fn integer(&mut self) -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| INVALID)?,
        ))
    }
    /// Accepts exactly the two versioned boolean values.
    fn boolean(&mut self) -> Result<bool, &'static str> {
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(INVALID),
        }
    }
}
