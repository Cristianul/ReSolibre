//! Bounded local-session intent shared by the worker and storage adapter.

use super::validate_selection;
use resolibre_core::equalizer::Equalizer;
use resolibre_core::playback_edges::PlaybackEdges;
use resolibre_core::queue::RepeatMode;
use resolibre_core::session::Volume;
use std::path::PathBuf;
use std::sync::Arc;

/// Local intent only: shared path storage changes on queue edits, never on playback ticks.
#[derive(Clone, Default)]
pub struct SavedSession {
    pub paths: Arc<[PathBuf]>,
    pub current: Option<usize>,
    pub volume: Volume,
    pub equalizer: Equalizer,
    pub edges: PlaybackEdges,
    pub mix_enabled: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

impl SavedSession {
    /// Validates local references without touching media; missing files remain queue entries.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.paths.is_empty() {
            validate_selection(&self.paths)?;
        }
        if self.current.is_some_and(|index| index >= self.paths.len()) {
            return Err("Invalid saved selection");
        }
        for path in self.paths.iter() {
            if !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0) {
                return Err("Invalid saved local reference");
            }
        }
        Ok(())
    }
}

impl PartialEq for SavedSession {
    /// Compares intent with a constant-time path check when only playback timing changes.
    fn eq(&self, other: &Self) -> bool {
        self.current == other.current
            && self.volume == other.volume
            && self.equalizer == other.equalizer
            && self.edges == other.edges
            && self.mix_enabled == other.mix_enabled
            && self.shuffle == other.shuffle
            && self.repeat == other.repeat
            && (Arc::ptr_eq(&self.paths, &other.paths) || self.paths == other.paths)
    }
}

impl std::fmt::Debug for SavedSession {
    /// Keeps private source paths out of snapshot diagnostics and assertion failures.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SavedSession")
            .field("track_count", &self.paths.len())
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}
