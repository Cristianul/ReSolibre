//! Windows reset retains directory and file handles before marking allowlisted files for deletion.

use super::{INVALID, transaction_file};
use crate::profile_files;
use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO, FileDispositionInfo,
    GetFileInformationByHandle, SetFileInformationByHandle,
};

/// Pins each ancestor against rename/deletion and rejects reparse points before enumerating profile data.
fn pin_directories(path: &Path) -> Result<Vec<File>, &'static str> {
    let mut pins = Vec::new();
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let file = OpenOptions::new()
            .read(true)
            .access_mode(0x80)
            .share_mode(3)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(ancestor)
            .map_err(|_| INVALID)?;
        let metadata = file.metadata().map_err(|_| INVALID)?;
        if !metadata.is_dir() || profile_files::linked(&metadata) {
            return Err(INVALID);
        }
        pins.push(file);
    }
    Ok(pins)
}

/// Deletes only validated application files by handle after checking the entire bounded profile.
pub fn clear(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() {
        return Err(INVALID);
    }
    let mut directories = pin_directories(path)?;
    let mut files = Vec::new();
    collect(path, "root", &mut files)?;
    for name in ["analysis", "eq-presets"] {
        let child = path.join(name);
        match child.symlink_metadata() {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Ok(_) => {}
            Err(_) => return Err(INVALID),
        }
        directories.extend(pin_directories(&child)?);
        collect(&child, name, &mut files)?;
    }
    for file in files {
        let mut disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: this validated unique handle has DELETE access; no pathname lookup occurs during deletion.
        let success = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfo,
                (&mut disposition as *mut FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
        if success == 0 {
            return Err("Cannot finish application reset");
        }
    }
    Ok(())
}

/// Retains leaf objects with DELETE access and no write/delete sharing; links and hardlinks fail closed.
fn collect(directory: &Path, kind: &str, files: &mut Vec<File>) -> Result<(), &'static str> {
    for (index, entry) in fs::read_dir(directory).map_err(|_| INVALID)?.enumerate() {
        if index >= 1024 {
            return Err(INVALID);
        }
        let entry = entry.map_err(|_| INVALID)?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(INVALID)?;
        if kind == "root" && matches!(name, "analysis" | "eq-presets") {
            continue;
        }
        let metadata = entry.path().symlink_metadata().map_err(|_| INVALID)?;
        if !metadata.is_file() || profile_files::linked(&metadata) {
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
        let file = OpenOptions::new()
            .access_mode(0x0001_0000 | 0x80)
            .share_mode(1)
            .custom_flags(0x0020_0000)
            .open(entry.path())
            .map_err(|_| INVALID)?;
        let mut facts = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the retained file handle and fixed-size output are valid for this synchronous query.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut facts) } == 0
            || facts.nNumberOfLinks != 1
            || !profile_files::regular(&file).map_err(|_| INVALID)?
        {
            return Err(INVALID);
        }
        files.push(file);
    }
    Ok(())
}
