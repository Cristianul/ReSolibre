//! Read-only, bounded directory expansion outside the UI and playback threads.

use resolibre_audio::session::{MAX_SELECTION_BYTES, validate_selection};
use resolibre_core::queue::MAX_QUEUE_ITEMS;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const MAX_ENTRIES: usize = 16_384;
const MAX_TRAVERSAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_DEPTH: usize = 32;

pub struct Selection {
    pub paths: Vec<PathBuf>,
}

struct Scan<'a> {
    cancel: &'a AtomicBool,
    started: Instant,
    entries: usize,
    traversal_bytes: usize,
    selected_bytes: usize,
    selection: Selection,
}

/// Expands roots in drop order and directory children in lexical order; no symlinks are followed.
pub fn expand(roots: Vec<PathBuf>, cancel: &AtomicBool) -> Result<Selection, &'static str> {
    validate_selection(&roots)?;
    let mut scan = Scan {
        cancel,
        started: Instant::now(),
        entries: 0,
        traversal_bytes: 0,
        selected_bytes: 0,
        selection: Selection { paths: Vec::new() },
    };
    for root in roots {
        scan.visit(root, 0)?;
    }
    Ok(scan.selection)
}

impl Scan<'_> {
    /// Checks cancellation and cooperative time bounds between filesystem operations.
    fn check(&self) -> Result<(), &'static str> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err("Import cancelled");
        }
        if self.started.elapsed() > Duration::from_secs(10) {
            return Err("Folder import took too long; choose a smaller folder");
        }
        Ok(())
    }

    /// Bounds names retained during sorting, including unsupported entries.
    fn account(&mut self, path: &Path) -> Result<(), &'static str> {
        self.check()?;
        self.entries += 1;
        self.traversal_bytes = self.traversal_bytes.saturating_add(path.as_os_str().len());
        if self.entries > MAX_ENTRIES || self.traversal_bytes > MAX_TRAVERSAL_BYTES {
            return Err("Folder is too large; choose a smaller folder");
        }
        if !path.is_absolute() || path.as_os_str().len() > 4096 {
            return Err("Invalid local file path");
        }
        Ok(())
    }

    /// Reads metadata only; unsupported files, links and special devices are skipped.
    fn visit(&mut self, path: PathBuf, depth: usize) -> Result<(), &'static str> {
        self.check()?;
        if depth > MAX_DEPTH {
            return Err("Folder nesting is too deep");
        }
        if depth == 0 {
            self.account(&path)?;
        }
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|_| "Cannot read a dropped file or folder")?;
        if metadata.is_dir() {
            let directory = std::fs::read_dir(&path).map_err(|_| "Cannot read a dropped folder")?;
            let mut children = Vec::new();
            for entry in directory {
                let child = entry.map_err(|_| "Cannot read a folder entry")?.path();
                self.account(&child)?;
                children.push(child);
            }
            children.sort();
            for child in children {
                self.visit(child, depth + 1)?;
            }
            return Ok(());
        }

        if !metadata.is_file() || !resolibre_audio::media_formats::is_candidate(&path) {
            return Ok(());
        }
        self.selected_bytes = self.selected_bytes.saturating_add(path.as_os_str().len());
        if self.selection.paths.len() >= MAX_QUEUE_ITEMS
            || self.selected_bytes > MAX_SELECTION_BYTES
        {
            return Err("Track limit reached (1024 files / 1 MiB paths)");
        }
        self.selection.paths.push(path);
        Ok(())
    }
}
