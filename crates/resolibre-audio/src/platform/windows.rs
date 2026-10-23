//! Windows handle ownership and bounded pipe polling; no sockets or shell commands.

use std::fs::{File, Metadata};
use std::io;
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path, Prefix};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_SHARE_READ, ReOpenFile};

mod pipe;
pub use pipe::PipeReader;
pub(crate) use pipe::{named_pipe, output_pipe};

/// Rejects device namespaces, alternate streams and DOS device names before Windows opens them.
pub(crate) fn validate_media_path(path: &Path) -> io::Result<()> {
    for component in path.components() {
        if let Component::Prefix(prefix) = component {
            let ordinary = match prefix.kind() {
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) => true,
                Prefix::UNC(server, _) | Prefix::VerbatimUNC(server, _) => {
                    server != "." && server != "?"
                }
                _ => false,
            };
            if !ordinary {
                return Err(io::Error::other("Device paths are not media"));
            }
        }
        if let Component::Normal(name) = component {
            let name = name.to_string_lossy();
            let stem = name
                .split('.')
                .next()
                .unwrap_or("")
                .trim_end_matches([' ', '.'])
                .to_ascii_uppercase();
            let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.chars().count() == 4
                && stem
                    .chars()
                    .last()
                    .is_some_and(|value| "123456789¹²³".contains(value));
            if name.contains(':')
                || matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                )
                || numbered
            {
                return Err(io::Error::other("Device paths are not media"));
            }
        }
    }
    Ok(())
}

/// Detects every reparse point, including junctions that are not Rust symbolic links.
pub fn is_reparse(metadata: &Metadata) -> bool {
    metadata.file_attributes() & 0x400 != 0
}

/// Obtains a fresh cursor to an existing read-only source without a path-replacement race.
pub(crate) fn independent_file(file: &File) -> io::Result<File> {
    // SAFETY: the source handle remains owned, and successful ReOpenFile transfers one new handle.
    let handle = unsafe { ReOpenFile(file.as_raw_handle(), FILE_GENERIC_READ, FILE_SHARE_READ, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this new handle has no other owner and File closes it exactly once.
    Ok(unsafe { File::from_raw_handle(handle) })
}
