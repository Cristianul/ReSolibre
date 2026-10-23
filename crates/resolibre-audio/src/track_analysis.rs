//! One cancellable original-stream analysis job; no source path or native diagnostic is published.

use crate::original_audio::{self, SourceKey};
use resolibre_core::analysis::{TrackAnalysis, TrackAnalyzer};
use std::fs::File;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(300);
const UNAVAILABLE: &str = "Track analysis unavailable or resource limit reached";

pub(crate) enum TrackInput {
    Retained(File, SourceKey),
    Path(std::path::PathBuf),
}

pub(crate) struct TrackJob {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    thread: Option<JoinHandle<Result<TrackAnalysis, &'static str>>>,
}

impl TrackJob {
    /// Starts one retained-file decoder; source identity is revalidated before returning results.
    pub(crate) fn start(
        file: File,
        key: SourceKey,
        cache_config: Option<Arc<crate::analysis_cache::CacheConfig>>,
    ) -> Result<Self, &'static str> {
        Self::start_input(TrackInput::Retained(file, key), cache_config, false)
    }

    /// Opens library sources on the background thread; explicit retries bypass stored measurements.
    pub(crate) fn start_input(
        input: TrackInput,
        cache_config: Option<Arc<crate::analysis_cache::CacheConfig>>,
        force: bool,
    ) -> Result<Self, &'static str> {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let worker_cancel = Arc::clone(&cancel);
        let worker_progress = Arc::clone(&progress);
        let thread = thread::Builder::new()
            .name("resolibre-analysis".into())
            .spawn(move || {
                let (file, key) = match input {
                    TrackInput::Retained(file, key) => (file, key),
                    TrackInput::Path(path) => {
                        use std::os::unix::fs::OpenOptionsExt;
                        let file = File::options()
                            .read(true)
                            .custom_flags(crate::OPEN_NONBLOCK)
                            .open(path)
                            .map_err(|_| UNAVAILABLE)?;
                        let key = SourceKey::read(&file, u32::MAX).map_err(|_| UNAVAILABLE)?;
                        (file, key)
                    }
                };
                let retained = file.try_clone().map_err(|_| UNAVAILABLE)?;
                let mut cached = crate::analysis_cache::CachedInput::open(
                    cache_config.as_deref(),
                    &file,
                    key,
                    &worker_cancel,
                );
                if let Some(cached) = &mut cached
                    && crate::analysis_cache::unchanged(&file, key, &worker_cancel)
                    && cached
                        .database
                        .save_track_source(&key.metadata_key(), &cached.identity)
                        .is_err()
                    && let Some(config) = &cache_config
                {
                    config.warn();
                }
                if !force && let Some(cached) = &cached {
                    match cached.database.track(&cached.identity) {
                        Ok(Some(report)) => {
                            if crate::analysis_cache::unchanged(&file, key, &worker_cancel) {
                                return Ok(report);
                            }
                            return Err(UNAVAILABLE);
                        }
                        Ok(None) => {}
                        Err(_) => {
                            if let Some(config) = &cache_config {
                                config.warn();
                            }
                        }
                    }
                }
                if !crate::analysis_cache::unchanged(&file, key, &worker_cancel) {
                    return Err(UNAVAILABLE);
                }
                let mut analyzer = None;
                original_audio::native::decode(
                    file,
                    key,
                    &worker_cancel,
                    DEADLINE,
                    |reader, started| {
                        let expected = reader.expected_format();
                        original_audio::pcm::stream(
                            reader,
                            &worker_cancel,
                            started,
                            DEADLINE,
                            |rate, channels, samples| {
                                if (rate, channels) != expected {
                                    return Err(());
                                }
                                if analyzer.is_none() {
                                    analyzer =
                                        Some(TrackAnalyzer::new(rate, channels).map_err(|_| ())?);
                                }
                                let analyzer = analyzer.as_mut().ok_or(())?;
                                let work_started = std::time::Instant::now();
                                analyzer.push(samples).map_err(|_| ())?;
                                worker_progress.store(
                                    (analyzer.decoded_seconds() * 1000.0) as u64,
                                    Ordering::Relaxed,
                                );
                                // Give playback/control threads regular scheduling opportunities during a full decode.
                                thread::sleep(
                                    (work_started.elapsed() * 3).clamp(
                                        Duration::from_micros(200),
                                        Duration::from_millis(25),
                                    ),
                                );
                                Ok(())
                            },
                        )?;
                        Ok(())
                    },
                )
                .map_err(|_| UNAVAILABLE)?;
                if worker_cancel.load(Ordering::Relaxed) {
                    return Err(UNAVAILABLE);
                }
                let report = analyzer
                    .ok_or(UNAVAILABLE)?
                    .finish()
                    .map_err(|_| UNAVAILABLE)?;
                if !crate::analysis_cache::unchanged(&retained, key, &worker_cancel) {
                    return Err(UNAVAILABLE);
                }
                if let Some(cached) = &mut cached
                    && cached
                        .database
                        .save_track(&cached.identity, &report)
                        .is_err()
                    && let Some(config) = &cache_config
                {
                    config.warn();
                }
                if !crate::analysis_cache::unchanged(&retained, key, &worker_cancel) {
                    return Err(UNAVAILABLE);
                }
                Ok(report)
            })
            .map_err(|_| UNAVAILABLE)?;
        Ok(Self {
            cancel,
            progress,
            thread: Some(thread),
        })
    }

    /// Signals cancellation without joining on the playback/control thread.
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Returns decoded source seconds, independent of inaccurate container duration estimates.
    pub(crate) fn progress(&self) -> f64 {
        self.progress.load(Ordering::Relaxed) as f64 / 1000.0
    }

    /// Joins only an already-finished worker; cancellation/replacement remains responsive.
    pub(crate) fn poll(&mut self) -> Option<Result<TrackAnalysis, &'static str>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        Some(self.thread.take()?.join().unwrap_or(Err(UNAVAILABLE)))
    }
}

impl Drop for TrackJob {
    /// Final shutdown reaps owned work; ordinary cancellation polls completion before dropping the job.
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests;
