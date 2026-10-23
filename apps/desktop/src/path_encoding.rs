//! Platform-tagged session/chooser paths preserve Unix bytes or Windows UTF-16.

use std::ffi::{OsStr, OsString};

// UTF-16 can take twice as many bytes as the core's native encoded path length.
pub const BYTE_FACTOR: usize = if cfg!(windows) { 2 } else { 1 };
pub const MAX_PATH_BYTES: usize = 4096 * BYTE_FACTOR;
pub const MAX_SELECTION_BYTES: usize = resolibre_audio::session::MAX_SELECTION_BYTES * BYTE_FACTOR;

/// Serializes native path units without lossy Unicode conversion.
pub fn encode(path: &OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.encode_wide().flat_map(u16::to_le_bytes).collect()
    }
}

/// Rejects NULs and incomplete native units before constructing a path.
pub fn decode(bytes: &[u8]) -> Option<OsString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        if bytes.contains(&0) {
            return None;
        }
        Some(OsString::from_vec(bytes.to_vec()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let (pairs, remainder) = bytes.as_chunks::<2>();
        if !remainder.is_empty() {
            return None;
        }
        let units: Vec<_> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        if units.contains(&0) {
            return None;
        }
        Some(OsString::from_wide(&units))
    }
}
