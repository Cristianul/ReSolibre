//! Session-owned processing preparation and bounded, source-identity-scoped analysis reuse.
use super::{EdgeStatus, Snapshot};
use crate::{
    LocalPlayer, PlaybackError,
    edge_analysis::{AnalysisJob, SourceKey},
};
use resolibre_core::playback_edges::{EdgeAnalysis, PlaybackEdges, PlaybackRegion};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct EdgeRuntime {
    job: Option<AnalysisJob>,
    key: Option<SourceKey>,
    analysis: Option<EdgeAnalysis>,
    unavailable: bool,
    applied: Option<PlaybackEdges>,
    resume_needed: bool,
    pub region: Option<PlaybackRegion>,
    cache: VecDeque<(SourceKey, EdgeAnalysis)>,
}

impl EdgeRuntime {
    /// Cancels the previous track's work, preserving only the bounded source-identity cache.
    pub fn reset(&mut self) {
        self.job = None;
        self.key = None;
        self.analysis = None;
        self.unavailable = false;
        self.applied = None;
        self.resume_needed = false;
        self.region = None;
    }

    /// Waits without blocking the playback worker; incomplete results never reach timing or DSP.
    pub fn prepare(
        &mut self,
        player: &mut LocalPlayer,
        snapshot: &mut Snapshot,
    ) -> Result<bool, PlaybackError> {
        let settings = snapshot.edges;
        let preparing_start = self.waiting_for_start();
        if !settings.needs_analysis() {
            self.job = None;
            self.unavailable = false;
            snapshot.edge_status = EdgeStatus::Off;
            self.region = None;
            if self.applied != Some(settings) {
                player.apply_edges(None, snapshot.equalizer, preparing_start)?;
                self.applied = Some(settings);
                self.resume_needed = true;
            }
            return Ok(true);
        }
        if self.analysis.is_none() && !self.unavailable && self.job.is_none() {
            // An already-started track keeps playing while its first analysis runs.
            if preparing_start {
                player.set_paused(true)?;
                self.resume_needed = true;
            }
            if self.start(player).is_err() {
                self.mark_unavailable();
            }
        }
        if let Some(result) = self.job.as_mut().and_then(AnalysisJob::poll) {
            self.job = None;
            match result {
                Ok(analysis) => {
                    self.analysis = Some(analysis);
                    if let Some(key) = self.key {
                        if self.cache.len() == 64 {
                            self.cache.pop_front();
                        }
                        self.cache.push_back((key, analysis));
                    }
                }
                Err(_) => self.mark_unavailable(),
            }
        }
        if self.job.is_some() {
            snapshot.edge_status = EdgeStatus::Pending;
            return Ok(!preparing_start);
        }
        snapshot.edge_status = if self.unavailable {
            EdgeStatus::Unavailable
        } else {
            EdgeStatus::Ready
        };
        if self.applied != Some(settings) {
            self.region = self.analysis.map(|analysis| analysis.region(settings));
            player.apply_edges(
                self.region.map(|region| (region, settings.fade)),
                snapshot.equalizer,
                preparing_start,
            )?;
            self.applied = Some(settings);
            self.resume_needed = true;
        }
        Ok(true)
    }

    /// Initial processing must finish before first resume; live analysis never blocks transport.
    pub fn waiting_for_start(&self) -> bool {
        self.applied.is_none()
    }

    /// Keeps the UI timeline on the original file, including yellow skipped regions.
    pub fn source_duration(&self) -> Option<f64> {
        self.analysis.map(|analysis| analysis.duration)
    }

    /// Applies the user's pause intent once preparation or processing changes have finished.
    pub fn take_resume_needed(&mut self) -> bool {
        std::mem::take(&mut self.resume_needed)
    }

    /// Uses the current stream and retained descriptor; no display label/path is a cache key.
    fn start(&mut self, player: &mut LocalPlayer) -> Result<(), ()> {
        let file = player.analysis_input().map_err(|_| ())?;
        let stream = player.audio_stream_index().map_err(|_| ())?;
        let key = SourceKey::read(&file, stream).map_err(|_| ())?;
        self.key = Some(key);
        if let Some(index) = self.cache.iter().position(|(cached, _)| *cached == key) {
            let entry = self.cache.remove(index).ok_or(())?;
            self.analysis = Some(entry.1);
            self.cache.push_back(entry);
        } else {
            self.job = Some(AnalysisJob::start(file, key).map_err(|_| ())?);
        }
        Ok(())
    }

    /// Reports one recoverable failure without source names or native diagnostics.
    fn mark_unavailable(&mut self) {
        self.unavailable = true;
        eprintln!("warn: TRIM/FADE analysis unavailable; playing original audio");
    }
}
