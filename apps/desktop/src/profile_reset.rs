//! Allowlisted reset of application-owned data; never follows links or reads music locations.

#[cfg(unix)]
use std::fs::{self, File, OpenOptions};
#[cfg(unix)]
use std::os::{
    fd::AsRawFd,
    unix::fs::{MetadataExt, OpenOptionsExt},
};
#[cfg(unix)]
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::clear;

const INVALID: &str = "Unexpected file in application data; reset cancelled";

/// Pins a real directory descriptor so later parent-path replacement cannot redirect deletion.
#[cfg(unix)]
fn directory(path: &Path) -> Result<File, &'static str> {
    OpenOptions::new()
        .read(true)
        .custom_flags(0x10000 | 0x20000 | 0x800)
        .open(path)
        .map_err(|_| "Cannot open application data directory")
}

/// Refers only to a retained application directory; entry names come from its bounded enumeration.
#[cfg(unix)]
fn anchored(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

/// Validates all known data before deleting any; retained empty directories and lock files carry no settings.
/// The caller must hold exclusive profile access and stop every application worker first.
#[cfg(unix)]
pub fn clear(path: &Path) -> Result<(), &'static str> {
    let root = directory(path)?;
    let mut directories = vec![root];
    let mut files = Vec::new();
    collect(&directories[0], "root", &mut files)?;
    for name in ["analysis", "eq-presets"] {
        let child = anchored(&directories[0]).join(name);
        match child.symlink_metadata() {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            _ => return Err(INVALID),
        }
        let retained = directory(&child)?;
        collect(&retained, name, &mut files)?;
        directories.push(retained);
    }
    for path in files {
        fs::remove_file(path).map_err(|_| "Cannot finish application reset")?;
    }
    for directory in directories {
        directory
            .sync_all()
            .map_err(|_| "Cannot flush application reset")?;
    }
    Ok(())
}

/// Restricts deletion to the current storage formats; unknown content, links and excessive entries fail closed.
#[cfg(unix)]
fn collect(directory: &File, kind: &str, files: &mut Vec<PathBuf>) -> Result<(), &'static str> {
    for (index, item) in fs::read_dir(anchored(directory))
        .map_err(|_| INVALID)?
        .enumerate()
    {
        if index >= 1024 {
            return Err(INVALID);
        }
        let entry = item.map_err(|_| INVALID)?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(INVALID)?;
        if kind == "root" && matches!(name, "analysis" | "eq-presets") {
            continue;
        }
        let metadata = entry.path().symlink_metadata().map_err(|_| INVALID)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() != 1 {
            return Err(INVALID);
        }
        if matches!(name, "session.lock" | "profile.lock") {
            continue;
        }
        let known = match kind {
            "root" => matches!(name, "session-v1.bin" | "equalizer-v1.txt"),
            "analysis" => matches!(
                name,
                "analysis.sqlite3"
                    | "analysis.sqlite3-journal"
                    | "analysis.sqlite3-wal"
                    | "analysis.sqlite3-shm"
            ),
            "eq-presets" => name.ends_with(".resolibre-eq"),
            _ => false,
        };
        if !known && !transaction_file(name, kind) {
            return Err(INVALID);
        }
        files.push(entry.path());
    }
    Ok(())
}

#[cfg(test)]
mod tests;

/// Recognizes only atomic-write leftovers from the existing settings/preset storage adapters.
fn transaction_file(name: &str, kind: &str) -> bool {
    let Some(stem) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((target, process)) = stem.rsplit_once('-') else {
        return false;
    };
    if process.is_empty() || !process.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    match kind {
        "root" => matches!(target, "session-v1.bin" | "equalizer-v1.txt"),
        "eq-presets" => target.ends_with(".resolibre-eq"),
        _ => false,
    }
}
