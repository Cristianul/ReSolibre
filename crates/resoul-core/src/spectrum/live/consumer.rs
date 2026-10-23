//! Shared bounded live scheduling and stale-stream rejection, independent of any native audio engine.

use super::{BAND_COUNT, LiveLevels, reduce};
use crate::spectrum::{AnalysisError, FFT_SIZE, MAX_CHANNELS, MAX_INPUT_FRAMES, SpectrumAnalyzer};
use std::collections::VecDeque;

/// One complete interleaved block with an adapter-assigned increasing stream instance and source counter.
pub struct PcmChunk<'a> {
    pub instance: u32,
    pub rate: u32,
    pub channels: usize,
    pub first_frame: u64,
    pub samples: &'a [f32],
}

pub struct LiveConsumer {
    analyzer: Option<SpectrumAnalyzer>,
    instance: u32,
    barrier: u32,
    rate: u32,
    channels: usize,
    next_frame: u64,
    origin: u64,
    offset: f64,
    epoch: u64,
    pending: VecDeque<(f64, [f32; BAND_COUNT])>,
    levels: [f32; BAND_COUNT],
}

impl Default for LiveConsumer {
    /// Reserves the fixed result queue before any PCM arrives.
    fn default() -> Self {
        Self {
            analyzer: None,
            instance: 0,
            barrier: 0,
            rate: 0,
            channels: 0,
            next_frame: 0,
            origin: 0,
            offset: 0.0,
            epoch: 0,
            pending: VecDeque::with_capacity(128),
            levels: [0.0; BAND_COUNT],
        }
    }
}

impl LiveConsumer {
    /// Invalidates old stream instances before an adapter reconstructs playback at a new source position.
    pub fn seek(&mut self, barrier: u32, seconds: f64) -> Result<(), AnalysisError> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(AnalysisError::InvalidPosition);
        }
        self.barrier = self.barrier.max(barrier);
        self.instance = 0;
        self.offset = seconds;
        self.analyzer = None;
        self.clear();
        Ok(())
    }

    /// Returns the last explicitly established source offset while a native clock is unavailable.
    pub fn offset(&self) -> f64 {
        self.offset
    }

    /// Validates a bounded chunk, rejects stale streams and resets history at discontinuities.
    pub fn accept(&mut self, chunk: PcmChunk<'_>, clock: f64) -> Result<(), AnalysisError> {
        validate(&chunk, clock)?;
        if chunk.samples.is_empty() {
            return Ok(());
        }
        if chunk.instance <= self.barrier || chunk.instance < self.instance {
            return Ok(());
        }
        let changed = chunk.instance != self.instance
            || chunk.rate != self.rate
            || chunk.channels != self.channels;
        if changed || chunk.first_frame != self.next_frame {
            self.restart(&chunk, clock, changed)?;
        }
        self.next_frame = chunk
            .first_frame
            .checked_add((chunk.samples.len() / chunk.channels) as u64)
            .ok_or(AnalysisError::PositionOverflow)?;
        self.analyze(&chunk)
    }

    /// Advances only to measurements due at the supplied playback clock; pause preserves displayed energy.
    pub fn present(&mut self, clock: f64, paused: bool, scanning: bool) -> LiveLevels {
        if clock.is_finite() && clock >= 0.0 && !paused && !scanning {
            while self.pending.front().is_some_and(|(time, _)| *time <= clock) {
                if let Some((_, levels)) = self.pending.pop_front() {
                    self.levels = levels;
                }
            }
        }
        if scanning {
            self.levels.fill(0.0);
        }
        LiveLevels {
            epoch: self.epoch,
            levels: self.levels,
        }
    }

    /// Clears results with a new presentation identity; callers cannot retain bars from a prior timeline.
    fn clear(&mut self) {
        self.pending.clear();
        self.levels.fill(0.0);
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Starts a complete window after a gap or format change, reusing FFT storage when possible.
    fn restart(
        &mut self,
        chunk: &PcmChunk<'_>,
        clock: f64,
        changed: bool,
    ) -> Result<(), AnalysisError> {
        if changed && self.instance != 0 {
            self.offset = clock;
        }
        if changed || self.analyzer.is_none() {
            self.analyzer = Some(SpectrumAnalyzer::new(chunk.rate, chunk.channels)?);
        } else if let Some(analyzer) = self.analyzer.as_mut() {
            analyzer.reset();
        }
        self.instance = chunk.instance;
        self.rate = chunk.rate;
        self.channels = chunk.channels;
        self.origin = chunk.first_frame;
        self.clear();
        Ok(())
    }

    /// Takes full windows at no more than 20 measurements/s; skipped samples keep their source positions.
    fn analyze(&mut self, chunk: &PcmChunk<'_>) -> Result<(), AnalysisError> {
        let analyzer = self
            .analyzer
            .as_mut()
            .ok_or(AnalysisError::InvalidPosition)?;
        let stride = u64::from(chunk.rate / 20).max(FFT_SIZE as u64);
        let mut position = chunk.first_frame;
        while position < self.next_frame {
            if position < self.origin {
                position = self.origin.min(self.next_frame);
                continue;
            }
            let window_end = self
                .origin
                .checked_add(FFT_SIZE as u64)
                .ok_or(AnalysisError::PositionOverflow)?;
            let end = window_end.min(self.next_frame);
            let start_sample = (position - chunk.first_frame) as usize * chunk.channels;
            let end_sample = (end - chunk.first_frame) as usize * chunk.channels;
            analyzer.push(&chunk.samples[start_sample..end_sample], |frame| {
                let center = (frame.start_frame + frame.end_frame) as f64 / 2.0;
                let time = self.offset + (self.origin as f64 + center) / f64::from(chunk.rate);
                if self.pending.len() == 128 {
                    self.pending.pop_front();
                }
                self.pending.push_back((time, reduce(frame)));
            })?;
            position = end;
            if position == window_end {
                self.origin = self
                    .origin
                    .checked_add(stride)
                    .ok_or(AnalysisError::PositionOverflow)?;
                analyzer.reset();
            }
        }
        Ok(())
    }
}

/// Checks sizes, clock and numeric input before modifying stream state or allocating FFT buffers.
fn validate(chunk: &PcmChunk<'_>, clock: f64) -> Result<(), AnalysisError> {
    if !clock.is_finite() || clock < 0.0 || chunk.instance == 0 {
        return Err(AnalysisError::InvalidPosition);
    }
    if !(1..=MAX_CHANNELS).contains(&chunk.channels) {
        return Err(AnalysisError::InvalidChannelCount);
    }
    if !(1..=1_536_000).contains(&chunk.rate) {
        return Err(AnalysisError::InvalidSampleRate);
    }
    if chunk.samples.len() > MAX_INPUT_FRAMES * chunk.channels {
        return Err(AnalysisError::InputTooLarge);
    }
    if !chunk.samples.len().is_multiple_of(chunk.channels) {
        return Err(AnalysisError::IncompleteFrame);
    }
    if chunk.samples.iter().any(|sample| !sample.is_finite()) {
        return Err(AnalysisError::NonFiniteSample);
    }
    let reserve = MAX_INPUT_FRAMES as u64 + FFT_SIZE as u64 + u64::from(chunk.rate / 20);
    if chunk.first_frame.checked_add(reserve).is_none() {
        return Err(AnalysisError::PositionOverflow);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Borrows generated mono PCM with a source counter rather than exposing any adapter representation.
    fn chunk(instance: u32, first_frame: u64, samples: &[f32]) -> PcmChunk<'_> {
        PcmChunk {
            instance,
            rate: 48000,
            channels: 1,
            first_frame,
            samples,
        }
    }

    /// Band reduction and cadence are independent of decoder block sizes, and queues remain bounded.
    #[test]
    fn chunks_share_one_clock_and_bounded_cadence() {
        let samples: Vec<_> = (0..MAX_INPUT_FRAMES)
            .map(|index| (std::f32::consts::TAU * 1000.0 * index as f32 / 48000.0).sin() * 0.25)
            .collect();
        let mut whole = LiveConsumer::default();
        whole.accept(chunk(1, 0, &samples), 0.0).unwrap();
        let mut split = LiveConsumer::default();
        for (index, samples) in samples.chunks(512).enumerate() {
            split
                .accept(chunk(1, (index * 512) as u64, samples), 0.0)
                .unwrap();
        }
        assert_eq!(whole.pending, split.pending);
        assert_eq!(whole.pending.len(), 6);
        assert!(
            whole
                .present(0.0, false, false)
                .levels
                .iter()
                .all(|level| *level == 0.0)
        );
        let ready = whole.present(1.0, false, false);
        assert!(ready.levels.iter().any(|level| *level > 0.5));
        assert_eq!(whole.present(2.0, true, false), ready);
        for index in 1..30 {
            split
                .accept(chunk(1, (index * MAX_INPUT_FRAMES) as u64, &samples), 0.0)
                .unwrap();
        }
        assert_eq!(split.pending.len(), 128);
    }

    /// Seek barriers reject old producers and dropped PCM cannot bridge an FFT window.
    #[test]
    fn seek_and_gaps_discard_old_energy() {
        let samples = vec![0.5; FFT_SIZE];
        let mut consumer = LiveConsumer::default();
        consumer.accept(chunk(1, 0, &samples), 0.0).unwrap();
        consumer.seek(1, 3.0).unwrap();
        let cleared = consumer.present(3.0, true, false);
        consumer
            .accept(chunk(1, FFT_SIZE as u64, &samples), 3.0)
            .unwrap();
        assert_eq!(consumer.present(3.0, true, false), cleared);
        consumer.accept(chunk(2, 0, &samples[..1024]), 3.0).unwrap();
        consumer
            .accept(chunk(2, 4096, &samples[..1024]), 3.0)
            .unwrap();
        assert!(consumer.pending.is_empty());
        consumer
            .accept(chunk(2, 5120, &samples[..1024]), 3.0)
            .unwrap();
        assert_eq!(consumer.pending.len(), 1);
        assert!(consumer.pending.front().unwrap().0 > 3.08);
    }

    /// Malformed clocks, dimensions, samples and counters cannot mutate accepted stream state.
    #[test]
    fn invalid_chunks_preserve_state() {
        let mut consumer = LiveConsumer::default();
        consumer.accept(chunk(1, 0, &[0.0; 1024]), 0.0).unwrap();
        let epoch = consumer.epoch;
        let next = consumer.next_frame;
        assert_eq!(
            consumer.accept(chunk(1, 1024, &[f32::NAN]), 0.0),
            Err(AnalysisError::NonFiniteSample)
        );
        assert_eq!(
            consumer.accept(chunk(1, 1024, &[0.0]), f64::INFINITY),
            Err(AnalysisError::InvalidPosition)
        );
        assert_eq!(
            consumer.accept(chunk(1, u64::MAX, &[0.0]), 0.0),
            Err(AnalysisError::PositionOverflow)
        );
        for (channels, rate, expected) in [
            (0, 48000, AnalysisError::InvalidChannelCount),
            (MAX_CHANNELS + 1, 48000, AnalysisError::InvalidChannelCount),
            (1, 0, AnalysisError::InvalidSampleRate),
            (1, 1_536_001, AnalysisError::InvalidSampleRate),
            (2, 48000, AnalysisError::IncompleteFrame),
        ] {
            let mut malformed = chunk(1, 1024, &[0.0]);
            malformed.channels = channels;
            malformed.rate = rate;
            assert_eq!(consumer.accept(malformed, 0.0), Err(expected));
        }
        let oversized = vec![0.0; MAX_INPUT_FRAMES + 1];
        assert_eq!(
            consumer.accept(chunk(1, 1024, &oversized), 0.0),
            Err(AnalysisError::InputTooLarge)
        );
        assert_eq!(
            consumer.seek(1, f64::NAN),
            Err(AnalysisError::InvalidPosition)
        );
        assert_eq!((consumer.epoch, consumer.next_frame), (epoch, next));
    }
}
