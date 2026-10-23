//! Optional persistent cache orchestration, exclusively inside owned background jobs.

pub(crate) mod status;

use crate::original_audio::SourceKey;
use resolibre_storage::{
    AnalysisCache,
    audio_identity::{self, AudioIdentity},
};
use std::{
    fs::File,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

/// One explicit profile location, shared by workers; default/test sessions never discover user storage.
pub(crate) struct CacheConfig {
    directory: PathBuf,
    warned: AtomicBool,
}

impl CacheConfig {
    /// Records platform-provided storage intent without filesystem I/O on the playback thread.
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            warned: AtomicBool::new(false),
        }
    }

    /// Opens profile storage only on a background worker, sharing the session's warning boundary.
    pub(crate) fn database(&self) -> Option<AnalysisCache> {
        match AnalysisCache::open(&self.directory) {
            Ok(database) => Some(database),
            Err(_) => {
                self.warn();
                None
            }
        }
    }

    /// Opens bounded metadata storage on the metadata worker, sharing the private profile boundary.
    pub(crate) fn metadata_database(&self) -> Option<AnalysisCache> {
        let result = AnalysisCache::open(&self.directory).and_then(|database| {
            database.prepare_metadata()?;
            Ok(database)
        });
        match result {
            Ok(database) => Some(database),
            Err(_) => {
                self.warn();
                None
            }
        }
    }

    /// Reports a recoverable cache failure once per session without private paths or tags.
    pub(crate) fn warn(&self) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            eprintln!("warn: persistent media cache unavailable; using fresh measurements");
        }
    }
}

pub(crate) struct CachedInput {
    pub(crate) database: AnalysisCache,
    pub(crate) identity: AudioIdentity,
}

impl CachedInput {
    /// Computes a complete audio-only key from an independent cursor, then opens only application storage.
    /// Cancellation or changed input never grants a usable cache entry; unsupported media simply bypass it.
    pub(crate) fn open(
        config: Option<&CacheConfig>,
        file: &File,
        key: SourceKey,
        cancel: &AtomicBool,
    ) -> Option<Self> {
        let config = config?;
        if !unchanged(file, key, cancel) {
            return None;
        }
        let input = crate::platform::independent_file(file).ok()?;
        let identity = audio_identity::identify(input, cancel)?;
        if !unchanged(file, key, cancel) {
            return None;
        }
        match AnalysisCache::open(&config.directory) {
            Ok(database) => Some(Self { database, identity }),
            Err(_) => {
                config.warn();
                None
            }
        }
    }
}

/// Revalidates access to the retained original even on a cache hit; cached data is not authorization.
pub(crate) fn unchanged(file: &File, key: SourceKey, cancel: &AtomicBool) -> bool {
    !cancel.load(Ordering::Relaxed) && SourceKey::read(file, key.stream).ok() == Some(key)
}
