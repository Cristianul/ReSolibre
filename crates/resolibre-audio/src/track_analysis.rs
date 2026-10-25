//! One cancellable original-stream analysis job; no source path or native diagnostic is published.

mod provenance;

use crate::original_audio::{self, SourceKey};
use resolibre_core::analysis::{TrackAnalysis, TrackAnalyzer};
use std::fs::File;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc::{self, Receiver},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(300);
const UNAVAILABLE: &str = "Track analysis unavailable or resource limit reached";

/// Actual worker stages; decoding completion is not overall analysis completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisPhase {
    Opening,
    LoadingSpectrogram,
    Decoding,
    ResolvingTempo,
    DetectingSource { completed: usize, total: usize },
    Saving,
}

pub(crate) enum TrackInput {
    Retained(File, SourceKey),
    Path(std::path::PathBuf),
}

type CompletedTrack = (TrackAnalysis, Option<[u8; 32]>);

pub(crate) struct TrackJob {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    phase: Arc<AtomicUsize>,
    digest: Option<[u8; 32]>,
    preview_receiver: Receiver<Arc<TrackAnalysis>>,
    preview: Option<Arc<TrackAnalysis>>,
    thread: Option<JoinHandle<Result<CompletedTrack, &'static str>>>,
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
        let phase = Arc::new(AtomicUsize::new(0));
        let worker_phase = Arc::clone(&phase);
        let worker_cancel = Arc::clone(&cancel);
        let worker_progress = Arc::clone(&progress);
        let (preview_sender, preview_receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("resolibre-analysis".into())
            .spawn(move || {
                let (file, key) = match input {
                    TrackInput::Retained(file, key) => (file, key),
                    TrackInput::Path(path) => {
                        let file =
                            crate::platform::open_media(&path, false).map_err(|_| UNAVAILABLE)?;
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
                                return Ok((report, Some(cached.identity.digest)));
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
                worker_phase.store(1, Ordering::Relaxed);
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
                worker_phase.store(2, Ordering::Relaxed);
                let mut report = analyzer
                    .ok_or(UNAVAILABLE)?
                    .finish()
                    .map_err(|_| UNAVAILABLE)?;
                if !crate::analysis_cache::unchanged(&retained, key, &worker_cancel) {
                    return Err(UNAVAILABLE);
                }
                // Publish one immutable, source-checked plot before the slower detector runs.
                // Its bounded matrix is copied once so the final verdict can be written independently.
                let _ = preview_sender.try_send(Arc::new(report.clone()));
                report.provenance = provenance::analyze(&retained, key, &report, &worker_cancel, &worker_phase);
                if !crate::analysis_cache::unchanged(&retained, key, &worker_cancel) {
                    return Err(UNAVAILABLE);
                }
                if report.provenance == resolibre_core::analysis::provenance::ProvenanceReport::Unavailable {
                    eprintln!("warn: lossy-source detection unavailable; retaining spectral and tempo analysis");
                }
                worker_phase.store(3, Ordering::Relaxed);
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
                let digest = cached.as_ref().map(|cached| cached.identity.digest);
                Ok((report, digest))
            })
            .map_err(|_| UNAVAILABLE)?;
        Ok(Self {
            cancel,
            progress,
            phase,
            digest: None,
            preview_receiver,
            preview: None,
            thread: Some(thread),
        })
    }

    /// Receives at most one complete spectral/tempo preview without waiting on the detector or transport.
    pub(crate) fn preview(&mut self) -> Option<Arc<TrackAnalysis>> {
        if let Ok(report) = self.preview_receiver.try_recv() {
            self.preview = Some(report);
        }
        self.preview.as_ref().map(Arc::clone)
    }

    /// Returns only a completed worker's validated original-audio key for historical report inspection.
    pub(crate) fn digest(&self) -> Option<[u8; 32]> {
        self.digest
    }

    /// Signals cancellation without joining on the playback/control thread.
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Returns decoded source seconds, independent of inaccurate container duration estimates.
    pub(crate) fn progress(&self) -> f64 {
        self.progress.load(Ordering::Relaxed) as f64 / 1000.0
    }

    /// Reads one atomic stage/counter snapshot, avoiding inconsistent excerpt totals during polling.
    pub(crate) fn phase(&self) -> AnalysisPhase {
        match self.phase.load(Ordering::Relaxed) {
            0 => AnalysisPhase::Opening,
            1 => AnalysisPhase::Decoding,
            2 => AnalysisPhase::ResolvingTempo,
            3 => AnalysisPhase::Saving,
            value => AnalysisPhase::DetectingSource {
                completed: (value >> 8) & 0xff,
                total: value & 0xff,
            },
        }
    }

    /// Joins only an already-finished worker; cancellation/replacement remains responsive.
    pub(crate) fn poll(&mut self) -> Option<Result<TrackAnalysis, &'static str>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        let result = self.thread.take()?.join().unwrap_or(Err(UNAVAILABLE));
        Some(result.map(|(report, digest)| {
            self.digest = digest;
            report
        }))
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
