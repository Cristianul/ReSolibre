//! Automatic rhythm excerpts, independent of the optional full-track spectrogram.

use crate::original_audio::{self, SourceKey};
use resolibre_core::analysis::{rhythm::RhythmAnalyzer, tempo::TempoReport};
use std::fs::File;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(10);
const EXCERPT_SECONDS: u32 = 64;

pub(crate) struct RhythmJob {
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(SourceKey, TempoReport), ()>>>,
}

impl RhythmJob {
    /// Starts one bounded retained-file excerpt without constructing a spectrogram or touching audio output.
    pub(crate) fn start(
        file: File,
        key: SourceKey,
        whole: bool,
        cache_config: Option<Arc<crate::analysis_cache::CacheConfig>>,
    ) -> Result<Self, ()> {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let thread = thread::Builder::new()
            .name("resolibre-rhythm".into())
            .spawn(move || {
                let retained = file.try_clone().map_err(|_| ())?;
                let mut cached = crate::analysis_cache::CachedInput::open(
                    cache_config.as_deref(),
                    &file,
                    key,
                    &worker_cancel,
                );
                let cached_report = cached.as_ref().and_then(|cached| {
                    match cached.database.rhythm(&cached.identity, whole) {
                        Ok(report) => report,
                        Err(_) => {
                            if let Some(config) = &cache_config {
                                config.warn();
                            }
                            None
                        }
                    }
                });
                if !crate::analysis_cache::unchanged(&file, key, &worker_cancel) {
                    return Err(());
                }
                if key.stream != u32::MAX
                    && let Some(report) = cached_report
                {
                    return Ok((key, report));
                }
                let result = original_audio::native::decode_prefix(
                    file,
                    key,
                    &worker_cancel,
                    DEADLINE,
                    |reader, started| {
                        let (rate, channels) = reader.expected_format();
                        let selected_key =
                            SourceKey::read(&retained, reader.selected_stream()).map_err(|_| ())?;
                        if let Some(report) = cached_report {
                            return Ok((selected_key, report));
                        }
                        let mut analyzer = RhythmAnalyzer::new(rate, channels).map_err(|_| ())?;
                        original_audio::pcm::stream_prefix(
                            reader,
                            &worker_cancel,
                            started,
                            DEADLINE,
                            if whole { None } else { Some(EXCERPT_SECONDS) },
                            |decoded_rate, decoded_channels, samples| {
                                if (decoded_rate, decoded_channels) != (rate, channels) {
                                    return Err(());
                                }
                                analyzer.push(samples).map_err(|_| ())
                            },
                        )?;
                        if worker_cancel.load(Ordering::Relaxed) {
                            return Err(());
                        }
                        Ok((selected_key, analyzer.finish()))
                    },
                )
                .map_err(|_| ())?;
                if !crate::analysis_cache::unchanged(&retained, key, &worker_cancel) {
                    return Err(());
                }
                if let Some(cached) = &mut cached
                    && cached
                        .database
                        .save_rhythm(&cached.identity, whole, &result.1)
                        .is_err()
                    && let Some(config) = &cache_config
                {
                    config.warn();
                }
                if !crate::analysis_cache::unchanged(&retained, key, &worker_cancel) {
                    return Err(());
                }
                Ok(result)
            })
            .map_err(|_| ())?;
        Ok(Self {
            cancel,
            thread: Some(thread),
        })
    }

    /// Interrupts pipe reads without blocking the playback thread.
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Reaps only a finished worker; obsolete results are rejected by the session owner.
    pub(crate) fn poll(&mut self) -> Option<Result<(SourceKey, TempoReport), ()>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        Some(self.thread.take()?.join().unwrap_or(Err(())))
    }
}

impl Drop for RhythmJob {
    /// Shutdown cancels and reaps owned native decoding; routine replacement polls first.
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
