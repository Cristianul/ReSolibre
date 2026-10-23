//! Bounded, platform-independent STFT over decoded PCM, shared by future live/offline consumers.

pub mod live;

#[cfg(test)]
mod tests;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

pub const FFT_SIZE: usize = 2048;
pub const HOP_SIZE: usize = FFT_SIZE / 2;
pub const BIN_COUNT: usize = FFT_SIZE / 2 + 1;
pub const FLOOR_DB: f32 = -160.0;
pub const MAX_CHANNELS: usize = 32;
pub const MAX_INPUT_FRAMES: usize = 16384;

/// Invalid input leaves the analyzer unchanged; callers may discard it and continue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisError {
    InvalidPosition,
    InvalidSampleRate,
    InvalidChannelCount,
    IncompleteFrame,
    InputTooLarge,
    NonFiniteSample,
    PositionOverflow,
}

/// One Hann-windowed power spectrum, averaged across channels without mixing their samples.
/// Power is relative to a full-scale sample squared; summing linear bin powers gives
/// window-weighted mean-square energy. Values below the display floor are truncated.
pub struct SpectrumFrame {
    pub sample_rate_hz: u32,
    pub start_frame: u64,
    pub end_frame: u64,
    pub valid_frames: usize,
    pub power_db: [f32; BIN_COUNT],
}

impl SpectrumFrame {
    /// Converts a valid bin index to source Hz, including DC and the Nyquist bin.
    pub fn frequency_hz(&self, bin: usize) -> Option<f64> {
        if bin >= BIN_COUNT {
            return None;
        }
        Some(bin as f64 * f64::from(self.sample_rate_hz) / FFT_SIZE as f64)
    }
}

/// Fixed FFT size and bounded channel storage; allocate once outside the audio callback.
/// Processing does not allocate, perform I/O or schedule work. Callbacks borrow a reused
/// result; consumers must copy/aggregate it before returning and must bound their own work.
pub struct SpectrumAnalyzer {
    channels: usize,
    history: Vec<f32>,
    filled: usize,
    unreported: usize,
    total_frames: u64,
    window: Vec<f64>,
    normalization: f64,
    fft: Arc<dyn Fft<f64>>,
    transform: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    power: [f64; BIN_COUNT],
    frame: SpectrumFrame,
}

impl SpectrumAnalyzer {
    /// Validates dimensions before allocation; native sample formats must be converted by adapters.
    pub fn new(sample_rate_hz: u32, channels: usize) -> Result<Self, AnalysisError> {
        if sample_rate_hz == 0 || sample_rate_hz > 1_536_000 {
            return Err(AnalysisError::InvalidSampleRate);
        }
        if channels == 0 || channels > MAX_CHANNELS {
            return Err(AnalysisError::InvalidChannelCount);
        }

        // RustFFT selects supported SIMD at runtime and retains a portable scalar fallback.
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        let window: Vec<f64> = (0..FFT_SIZE)
            .map(|index| {
                let phase = std::f64::consts::TAU * index as f64 / FFT_SIZE as f64;
                0.5 - 0.5 * phase.cos()
            })
            .collect();
        let window_energy: f64 = window.iter().map(|value| value * value).sum();
        let normalization = 1.0 / (FFT_SIZE as f64 * window_energy * channels as f64);

        Ok(Self {
            channels,
            history: vec![0.0; FFT_SIZE * channels],
            filled: 0,
            unreported: 0,
            total_frames: 0,
            window,
            normalization,
            fft,
            transform: vec![Complex::default(); FFT_SIZE],
            scratch,
            power: [0.0; BIN_COUNT],
            frame: SpectrumFrame {
                sample_rate_hz,
                start_frame: 0,
                end_frame: 0,
                valid_frames: 0,
                power_db: [FLOOR_DB; BIN_COUNT],
            },
        })
    }

    /// Processes one bounded interleaved block and emits each complete overlapping window.
    /// Rejects malformed/nonfinite blocks atomically, preserving previously accepted PCM.
    pub fn push(
        &mut self,
        samples: &[f32],
        mut receive: impl FnMut(&SpectrumFrame),
    ) -> Result<(), AnalysisError> {
        self.validate(samples)?;

        for input in samples.chunks_exact(self.channels) {
            let start = self.filled * self.channels;
            self.history[start..start + self.channels].copy_from_slice(input);
            self.filled += 1;
            self.unreported += 1;
            self.total_frames += 1;

            if self.filled == FFT_SIZE {
                self.analyze();
                receive(&self.frame);
                self.history.copy_within(HOP_SIZE * self.channels.., 0);
                self.filled = FFT_SIZE - HOP_SIZE;
                self.unreported = 0;
            }
        }
        Ok(())
    }

    /// Emits a zero-padded final window only when new samples remain; marks its valid extent.
    /// Intended for EOF, not live polling. Repeated calls emit nothing until more input arrives.
    pub fn finish(&mut self) -> Option<&SpectrumFrame> {
        if self.unreported == 0 {
            return None;
        }
        self.history[self.filled * self.channels..].fill(0.0);
        self.analyze();
        self.filled = 0;
        self.unreported = 0;
        Some(&self.frame)
    }

    /// Starts a new timeline after seek/track change; old PCM can no longer contribute.
    pub fn reset(&mut self) {
        self.filled = 0;
        self.unreported = 0;
        self.total_frames = 0;
        self.frame.start_frame = 0;
        self.frame.end_frame = 0;
        self.frame.valid_frames = 0;
        self.frame.power_db.fill(FLOOR_DB);
    }

    /// Checks block shape, work bounds and numeric safety before any history mutation.
    fn validate(&self, samples: &[f32]) -> Result<(), AnalysisError> {
        if samples.len() > MAX_INPUT_FRAMES * self.channels {
            return Err(AnalysisError::InputTooLarge);
        }
        if !samples.len().is_multiple_of(self.channels) {
            return Err(AnalysisError::IncompleteFrame);
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(AnalysisError::NonFiniteSample);
        }
        if self
            .total_frames
            .checked_add((samples.len() / self.channels) as u64)
            .is_none()
        {
            return Err(AnalysisError::PositionOverflow);
        }
        Ok(())
    }

    /// Accumulates per-channel power in f64 so phase inversion and extreme finite PCM stay safe.
    fn analyze(&mut self) {
        self.power.fill(0.0);
        for channel in 0..self.channels {
            for (index, value) in self.transform.iter_mut().enumerate() {
                let sample = f64::from(self.history[index * self.channels + channel]);
                *value = Complex::new(sample * self.window[index], 0.0);
            }
            self.fft
                .process_with_scratch(&mut self.transform, &mut self.scratch);

            for (bin, power) in self.power.iter_mut().enumerate() {
                // Real input has conjugate bins; DC and Nyquist have no distinct partner.
                let partners = if bin == 0 || bin == FFT_SIZE / 2 {
                    1.0
                } else {
                    2.0
                };
                *power += self.transform[bin].norm_sqr() * partners * self.normalization;
            }
        }

        for (level, power) in self.frame.power_db.iter_mut().zip(self.power) {
            *level = if power > 0.0 {
                (10.0 * power.log10()) as f32
            } else {
                FLOOR_DB
            };
            *level = level.max(FLOOR_DB);
        }
        self.frame.start_frame = self.total_frames - self.filled as u64;
        self.frame.end_frame = self.total_frames;
        self.frame.valid_frames = self.filled;
    }
}
