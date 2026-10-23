//! Local media candidates shared by chooser/scanner and the native container boundary.

use std::ffi::CString;
use std::path::Path;

/// Candidate extensions only: decoding validates content and requires a usable audio stream.
pub const EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "wave", "ogg", "oga", "opus", "aac", "m4a", "m4b", "aif", "aiff", "ape",
    "wv", "wma", "mp4", "m4v", "mov", "mkv", "mka", "webm", "avi", "wmv", "asf", "mpg", "mpeg",
    "ts", "m2ts", "flv", "3gp", "3g2",
];

// FFmpeg demuxer names, including aliases for the supported container families.
// Deliberately excludes playlists, image demuxers and reference/stream manifests.
pub(crate) const DEMUXERS: &str = "wav,flac,mp3,ogg,aac,mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,asf,mpeg,mpegts,flv,aiff,ape,wv";

/// Matches local filename extensions without probing content or performing filesystem I/O.
pub fn is_candidate(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    EXTENSIONS
        .iter()
        .any(|known| extension.eq_ignore_ascii_case(known))
}

/// Quotes the complete comma-separated allowlist for mpv; nested protocols remain disabled.
pub(crate) fn demuxer_options() -> CString {
    CString::new(format!(
        "format_whitelist=%{}%{DEMUXERS},protocol_whitelist=none",
        DEMUXERS.len()
    ))
    .expect("static demuxer names contain no NUL")
}
