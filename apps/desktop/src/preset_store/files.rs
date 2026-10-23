//! Filesystem adapter for small portable presets; no names become filesystem paths.

use super::{Completion, Entry, MAX_PRESETS, legacy};
use crate::profile_storage;
use resolibre_core::equalizer::preset::{MAX_BYTES, Preset};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const EXTENSION: &str = "resolibre-eq";
const NO_FOLLOW_NONBLOCK: i32 = 0x20000 | 0x800;

/// Validates the profile root before entering its private catalog and serializes writers.
pub(super) fn load(directory: &Path) -> Result<Completion, &'static str> {
    let legacy = profile_storage::read(directory, "equalizer-v1.txt", MAX_BYTES as u64)?;
    let folder = directory.join("eq-presets");
    let _lease = profile_storage::lock_session(&folder)?;
    let mut skipped = false;
    if let Some(bytes) = legacy {
        match legacy::decode(&bytes) {
            Ok(slots) => {
                for (index, settings) in slots.into_iter().enumerate() {
                    if let Some(settings) = settings {
                        let file = format!("legacy-{}.resolibre-eq", index + 1);
                        if folder.join(&file).symlink_metadata().is_err() {
                            let preset = Preset::new(&format!("Saved {}", index + 1), settings)?;
                            profile_storage::write(&folder, &file, preset.encode().as_bytes())?;
                        }
                    }
                }
            }
            Err(_) => skipped = true,
        }
    }
    let (entries, invalid) = scan(&folder)?;
    Ok(Completion::Catalog {
        entries,
        selected: None,
        skipped: skipped || invalid,
    })
}

/// Enumerates a bounded catalog; malformed files do not hide valid presets.
fn scan(folder: &Path) -> Result<(Vec<Entry>, bool), &'static str> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut skipped = false;
    for (index, item) in fs::read_dir(folder)
        .map_err(|_| "Cannot read preset folder")?
        .enumerate()
    {
        if index >= 1024 {
            return Err("Preset folder contains too many files");
        }
        let item = item.map_err(|_| "Cannot read preset folder")?;
        let path = item.path();
        if path.extension().is_none_or(|value| value != EXTENSION) {
            continue;
        }
        let Some(file) = item.file_name().to_str().map(str::to_owned) else {
            skipped = true;
            continue;
        };
        match read_preset(&path) {
            Ok(preset) => {
                if entries
                    .iter()
                    .any(|entry| entry.preset.name.to_lowercase() == preset.name.to_lowercase())
                {
                    skipped = true;
                    continue;
                }
                if entries.len() >= MAX_PRESETS {
                    return Err("Preset limit reached (256)");
                }
                entries.push(Entry { file, preset });
            }
            Err(_) => skipped = true,
        }
    }
    entries.sort_by_key(|entry| entry.preset.name.to_lowercase());
    Ok((entries, skipped))
}

/// Rechecks disk under the catalog lease, so stale UI state cannot silently replace a preset.
pub(super) fn save(
    directory: &Path,
    preset: Preset,
    replace: bool,
) -> Result<Completion, &'static str> {
    // Recheck the configured root before following the catalog child directory.
    profile_storage::read(directory, "equalizer-v1.txt", MAX_BYTES as u64)?;
    let folder = directory.join("eq-presets");
    let _lease = profile_storage::lock_session(&folder)?;
    let (mut entries, skipped) = scan(&folder)?;
    let existing = entries
        .iter()
        .position(|entry| entry.preset.name.to_lowercase() == preset.name.to_lowercase());
    let file = if let Some(index) = existing {
        if !replace {
            return Err("A preset with this name exists; choose Replace or another name");
        }
        entries[index].file.clone()
    } else {
        if entries.len() >= MAX_PRESETS {
            return Err("Preset limit reached (256)");
        }
        (1..=1024)
            .map(|index| format!("preset-{index}.resolibre-eq"))
            .find(|file| folder.join(file).symlink_metadata().is_err())
            .ok_or("No free preset filename")?
    };
    profile_storage::write(&folder, &file, preset.encode().as_bytes())?;
    let selected = Some(preset.name.clone());
    let entry = Entry { file, preset };
    if let Some(index) = existing {
        entries[index] = entry;
    } else {
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.preset.name.to_lowercase());
    Ok(Completion::Catalog {
        entries,
        selected,
        skipped,
    })
}

/// Bounds imported bytes and rejects symlinks, FIFOs, devices and malformed numeric settings.
pub(super) fn read_preset(path: &Path) -> Result<Preset, &'static str> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW_NONBLOCK)
        .open(path)
        .map_err(|_| "Cannot open EQ preset")?;
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect EQ preset")?
        .is_file()
    {
        return Err("EQ preset must be a regular file");
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read EQ preset")?;
    Preset::decode(&bytes)
}

/// Creates a new shareable file without replacing media, links or existing presets.
pub(super) fn export(path: &Path, preset: &Preset) -> Result<(), &'static str> {
    let mut path = path.to_path_buf();
    if path.extension().is_none() {
        path.set_extension(EXTENSION);
    }
    if path.extension().is_none_or(|value| value != EXTENSION) {
        return Err("Use the .resolibre-eq extension for exported presets");
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(NO_FOLLOW_NONBLOCK)
        .open(&path)
        .map_err(|_| "Cannot create export; choose a new filename")?;
    // Only a file created by this operation can be cleaned up on failure.
    let result = (|| {
        file.write_all(preset.encode().as_bytes())
            .map_err(|_| "Cannot write exported preset")?;
        file.sync_all()
            .map_err(|_| "Cannot flush exported preset")?;
        if let Some(parent) = path.parent() {
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|_| "Cannot flush export folder")?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result
}
