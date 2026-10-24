//! Compact periodicity alternatives retained until the complete recording supplies context.

use super::{BeatSegment, ENVELOPE_RATE, WINDOW, pulse::PulseEvidence};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::sync::Arc;

const FIRST_PERIOD: f64 = 50.0;
const PERIOD_STEP: f64 = 0.25;
const PERIODS: usize = 1001;
const DIRECT_PERIODS: usize = 251;
const MAX_LAG: usize = 1200;
const FFT_SIZE: usize = (2 * WINDOW).next_power_of_two();
const MIN_PERIODIC_CONTRAST: f64 = 0.15;

struct PeriodicityCurves {
    repeated: Vec<f32>,
    direct: Vec<f32>,
}

#[derive(Clone)]
pub(super) struct PeriodicityAnalyzer {
    forward: Arc<dyn Fft<f64>>,
    inverse: Arc<dyn Fft<f64>>,
    buffer: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
}

#[derive(Clone)]
pub(super) struct WindowEvidence {
    pub start: f64,
    pub duration: f64,
    pub original: Option<BeatSegment>,
    support: Vec<f32>,
    direct_support: Vec<f32>,
    pulse: PulseEvidence,
    maximum: f64,
}

impl PeriodicityAnalyzer {
    /// Allocates one reusable FFT workspace; full-track retention contains only small score curves.
    pub fn new() -> Self {
        let mut planner = FftPlanner::new();
        let forward = planner.plan_fft_forward(FFT_SIZE);
        let inverse = planner.plan_fft_inverse(FFT_SIZE);
        let scratch_length = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        Self {
            forward,
            inverse,
            buffer: vec![Complex::default(); FFT_SIZE],
            scratch: vec![Complex::default(); scratch_length],
        }
    }

    /// Measures repeated onset intervals with overlap normalization and an explicit noise-floor gate.
    fn curve(&mut self, envelope: &[f64]) -> PeriodicityCurves {
        let mut curves = PeriodicityCurves {
            repeated: vec![0.0; PERIODS],
            direct: vec![0.0; DIRECT_PERIODS],
        };
        if envelope.len() < 8 * ENVELOPE_RATE {
            return curves;
        }

        let peak = envelope.iter().copied().fold(0.0_f64, f64::max);
        if peak < 1e-8 {
            return curves;
        }

        let mean = envelope.iter().sum::<f64>() / envelope.len() as f64;
        let mut values: Vec<f64> = envelope
            .iter()
            .map(|value| (value - mean).max(0.0))
            .collect();
        let centered_mean = values.iter().sum::<f64>() / values.len() as f64;
        for value in &mut values {
            *value -= centered_mean;
        }

        // Adjacent five-millisecond bins can straddle the same attack or spectral hop.
        let mut smoothed = values.clone();
        for index in 1..values.len() - 1 {
            smoothed[index] =
                values[index - 1] * 0.25 + values[index] * 0.5 + values[index + 1] * 0.25;
        }

        let mut energy = Vec::with_capacity(smoothed.len() + 1);
        energy.push(0.0);

        // Zero padding prevents the end of this window from correlating with its beginning.
        self.buffer.fill(Complex::default());
        for (index, value) in smoothed.iter().enumerate() {
            self.buffer[index].re = *value;
            energy.push(energy[index] + value * value);
        }

        self.forward
            .process_with_scratch(&mut self.buffer, &mut self.scratch);
        for value in &mut self.buffer {
            *value = Complex::new(value.norm_sqr(), 0.0);
        }
        self.inverse
            .process_with_scratch(&mut self.buffer, &mut self.scratch);

        let mut scores = [0.0_f64; MAX_LAG + 1];
        for lag in 50..=MAX_LAG {
            let left = energy[smoothed.len() - lag];
            let right = energy[smoothed.len()] - energy[lag];
            let denominator = (left * right).sqrt() * FFT_SIZE as f64;
            if denominator > 1e-15 {
                scores[lag] = (self.buffer[lag].re / denominator).clamp(-1.0, 1.0);
            }
        }

        let strongest = scores[50..].iter().copied().fold(0.0_f64, f64::max);
        let background = scores[50..].iter().sum::<f64>() / (MAX_LAG - 49) as f64;
        if strongest < 0.20 || strongest - background < MIN_PERIODIC_CONTRAST {
            return curves;
        }

        for (index, value) in curves.direct.iter_mut().enumerate() {
            *value = (scores[FIRST_PERIOD as usize + index] - background).max(0.0) as f32;
        }

        for (index, value) in curves.repeated.iter_mut().enumerate() {
            let period = FIRST_PERIOD + index as f64 * PERIOD_STEP;
            let mut total = 0.0;
            for multiple in 1..=4 {
                let position = period * multiple as f64;
                let lower = position.floor() as usize;
                let fraction = position - lower as f64;
                let interpolated =
                    scores[lower] * (1.0 - fraction) + scores[(lower + 1).min(MAX_LAG)] * fraction;
                total += interpolated / multiple as f64;
            }
            *value = (total / (1.0 + 0.5 + 1.0 / 3.0 + 0.25)).max(0.0) as f32;
        }
        curves
    }

    /// Combines band support without counting correlated bands as independent confirmations.
    pub fn collect(
        &mut self,
        start: f64,
        envelopes: &[&[f64]],
        original: Option<BeatSegment>,
    ) -> WindowEvidence {
        let duration = envelopes
            .first()
            .map_or(0.0, |values| values.len() as f64 / ENVELOPE_RATE as f64);
        let mut support = vec![0.0_f32; PERIODS];
        let mut direct_support = vec![0.0_f32; DIRECT_PERIODS];
        for envelope in envelopes {
            let curves = self.curve(envelope);
            for (combined, measured) in support.iter_mut().zip(curves.repeated) {
                *combined = combined.max(measured);
            }
            for (combined, measured) in direct_support.iter_mut().zip(curves.direct) {
                *combined = combined.max(measured);
            }
        }
        let maximum = support.iter().copied().fold(0.0_f32, f32::max) as f64;
        WindowEvidence {
            start,
            duration,
            original,
            support,
            direct_support,
            pulse: PulseEvidence::collect(envelopes),
            maximum,
        }
    }
}

impl WindowEvidence {
    /// Verifies an externally established tempo against independent halves of the local spectral events.
    pub fn contextual_pulse_support(&self, bpm: f64) -> Option<f64> {
        self.pulse.support(bpm)
    }

    /// Interpolates measured support at a candidate BPM, without extending the search bounds.
    pub fn score(&self, bpm: f64) -> f64 {
        interpolate(&self.support, bpm, PERIOD_STEP)
    }

    /// Measures noise-floor contrast at one actual interval, excluding support borrowed from its longer multiples.
    pub fn direct_score(&self, bpm: f64) -> f64 {
        interpolate(&self.direct_support, bpm, 1.0)
    }

    /// Applies the same periodicity contrast gate to a proposed pulse's own interval.
    pub fn has_direct_pulse_support(&self, bpm: f64) -> bool {
        self.direct_score(bpm) >= MIN_PERIODIC_CONTRAST
    }

    /// Returns the strongest local periodicity, which is not itself proof of the musical beat level.
    pub fn maximum(&self) -> f64 {
        self.maximum
    }

    /// Retains at most eight separate periodicity peaks plus the independently fitted energy estimate.
    pub fn candidates(&self) -> Vec<f64> {
        let mut peaks: Vec<usize> = (0..self.support.len())
            .filter(|index| {
                self.support[*index] >= 0.20
                    && (*index == 0 || self.support[*index] >= self.support[index - 1])
                    && (*index + 1 == self.support.len()
                        || self.support[*index] >= self.support[index + 1])
            })
            .collect();
        peaks.sort_by(|left, right| self.support[*right].total_cmp(&self.support[*left]));
        let mut result = Vec::new();
        if let Some(original) = &self.original {
            result.push(original.bpm);
        }
        for index in peaks {
            let period = FIRST_PERIOD + index as f64 * PERIOD_STEP;
            let bpm = 60.0 * ENVELOPE_RATE as f64 / period;
            if result
                .iter()
                .all(|previous| (bpm / previous - 1.0).abs() > 0.015)
            {
                result.push(bpm);
            }
            if result.len() == 9 {
                break;
            }
        }
        result
    }
}

/// Interpolates a fixed-size score curve while rejecting invalid or unsupported pulse rates.
fn interpolate(values: &[f32], bpm: f64, period_step: f64) -> f64 {
    if !bpm.is_finite() || !(40.0..=240.0).contains(&bpm) {
        return 0.0;
    }
    let coordinate = (60.0 * ENVELOPE_RATE as f64 / bpm - FIRST_PERIOD) / period_step;
    let lower = coordinate.floor().max(0.0) as usize;
    if lower >= values.len() - 1 {
        return f64::from(values[values.len() - 1]);
    }
    let fraction = coordinate - lower as f64;
    f64::from(values[lower]) * (1.0 - fraction) + f64::from(values[lower + 1]) * fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Longer correlation multiples cannot authorize nonexistent intervening attacks at twice the true rate.
    #[test]
    fn direct_interval_rejects_borrowed_half_period_support() {
        let mut envelope = vec![0.0; WINDOW];
        let mut time = 0.25;
        while time < 32.0 {
            let index = (time * ENVELOPE_RATE as f64).round() as usize;
            envelope[index] = 1.0;
            time += 60.0 / 70.0;
        }
        let mut analyzer = PeriodicityAnalyzer::new();
        let window = analyzer.collect(0.0, &[&envelope], None);
        assert!(window.has_direct_pulse_support(70.0));
        assert!(
            window.score(140.0) > 0.20,
            "fixture must expose borrowed longer-lag support"
        );
        assert!(!window.has_direct_pulse_support(140.0));
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 39.0, 241.0] {
            assert_eq!(window.direct_score(invalid), 0.0);
            assert!(!window.has_direct_pulse_support(invalid));
        }
    }
}
