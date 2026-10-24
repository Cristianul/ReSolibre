//! Portable rhythm analysis with bounded spectral features; no spectrogram image or native dependencies.

use super::tempo::{TempoAnalyzer, TempoReport};
use crate::spectrum::{AnalysisError, MAX_CHANNELS, MAX_INPUT_FRAMES};

pub struct RhythmAnalyzer {
    tempo: TempoAnalyzer,
    channels: usize,
    rate: u32,
    frames: u64,
}

impl RhythmAnalyzer {
    /// Validates dimensions before allocating the bounded onset envelope.
    pub fn new(rate: u32, channels: usize) -> Result<Self, AnalysisError> {
        if rate == 0 || rate > 1_536_000 {
            return Err(AnalysisError::InvalidSampleRate);
        }
        if channels == 0 || channels > MAX_CHANNELS {
            return Err(AnalysisError::InvalidChannelCount);
        }
        Ok(Self {
            tempo: TempoAnalyzer::new(rate, channels),
            channels,
            rate,
            frames: 0,
        })
    }

    /// Rejects malformed blocks atomically; at most four hours of timeline and 32 seconds of PCM-derived envelope exist.
    pub fn push(&mut self, samples: &[f32]) -> Result<(), AnalysisError> {
        if samples.len() > MAX_INPUT_FRAMES * self.channels {
            return Err(AnalysisError::InputTooLarge);
        }
        if !samples.len().is_multiple_of(self.channels) {
            return Err(AnalysisError::IncompleteFrame);
        }
        if samples.iter().any(|value| !value.is_finite()) {
            return Err(AnalysisError::NonFiniteSample);
        }
        let frames = samples.len() as u64 / self.channels as u64;
        if self.frames + frames > u64::from(self.rate) * super::MAX_SECONDS {
            return Err(AnalysisError::InputTooLarge);
        }
        self.tempo.push(samples);
        self.frames += frames;
        Ok(())
    }

    /// Returns only measured tempo/attacks, never extrapolated beats beyond the analyzed excerpt.
    pub fn finish(self) -> TempoReport {
        self.tempo.finish()
    }
}
