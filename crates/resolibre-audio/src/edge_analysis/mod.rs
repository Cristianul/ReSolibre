//! Bounded, cancellable original-audio analysis. No playback processing enters this decoder.

use crate::original_audio;
pub(crate) use crate::original_audio::SourceKey;
use crate::original_audio::pcm;

use resolibre_core::playback_edges::EdgeAnalysis;
use std::fs::File;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

const DEADLINE: Duration = Duration::from_secs(15);
const UNAVAILABLE: &str = "TRIM/FADE analysis unavailable; playing original audio";

pub(crate) struct AnalysisJob {
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<EdgeAnalysis, &'static str>>>,
}

impl AnalysisJob {
    /// Starts at most one owned decoder; the session polls completion without blocking controls.
    pub(crate) fn start(file: File, key: SourceKey) -> Result<Self, &'static str> {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let thread = thread::Builder::new()
            .name("resolibre-trim".into())
            .spawn(move || {
                original_audio::decode(file, key, &worker_cancel, DEADLINE, |reader, started| {
                    pcm::decode(reader, &worker_cancel, started, DEADLINE)
                })
            })
            .map_err(|_| UNAVAILABLE)?;
        Ok(Self {
            cancel,
            thread: Some(thread),
        })
    }

    /// Returns one complete, validated result; partial decoding never becomes a playback boundary.
    pub(crate) fn poll(&mut self) -> Option<Result<EdgeAnalysis, &'static str>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        Some(self.thread.take()?.join().unwrap_or(Err(UNAVAILABLE)))
    }
}

impl Drop for AnalysisJob {
    /// Cancels and reaps this job only; socket timeouts let the child observe cancellation promptly.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stream identity and in-place writes invalidate cached bounds; cancellation reaps an owned job.
    #[test]
    fn source_identity_and_cancel_are_bounded() {
        let path = std::env::temp_dir().join(format!("resolibre-edge-key-{}", std::process::id()));
        std::fs::write(&path, b"not audio").unwrap();
        let file = File::open(&path).unwrap();
        let first = SourceKey::read(&file, 0).unwrap();
        assert_ne!(first, SourceKey::read(&file, 1).unwrap());
        std::fs::write(&path, b"changed input").unwrap();
        assert_ne!(first, SourceKey::read(&file, 0).unwrap());
        let key = SourceKey::read(&file, 0).unwrap();
        let job = AnalysisJob::start(file, key).unwrap();
        let started = Instant::now();
        drop(job);
        assert!(started.elapsed() < Duration::from_secs(1));
        std::fs::remove_file(path).unwrap();
    }
}
