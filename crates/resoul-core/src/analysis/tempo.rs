//! Bounded onset-energy tempo measurements. Values describe pulses, not musical meter or downbeats.

mod context;
mod estimate;
use super::activity::{ActivityAnalyzer, AudioActivity};
use estimate::estimate;

const ENVELOPE_RATE: usize = 200;
const WINDOW: usize = 32 * ENVELOPE_RATE;
const MAX_SEGMENTS: usize = 512;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeatSegment {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub bpm: f64,
    /// Observed onset times; a consumer must not extrapolate through an unmeasured gap.
    pub beats: Vec<f64>,
    /// Regularity score in 0..1, not a probability of musical correctness.
    pub confidence: f64,
    pub subdivision_ambiguous: bool,
    /// Absent when a tempo was measured but attack alignment was too weak for a beat timeline.
    pub beat_confidence: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TempoReport {
    /// Original-audio activity and coverage, independent of whether beat tracking succeeded.
    pub activity: AudioActivity,
    pub bpm: Option<f64>,
    pub variable: bool,
    pub segments: Vec<BeatSegment>,
}

impl TempoReport {
    /// Returns the observed local BPM range, ignoring malformed intervals and retaining a stable fallback.
    pub fn bpm_range(&self) -> Option<(f64, f64)> {
        let mut range: Option<(f64, f64)> = None;
        for segment in &self.segments {
            if !segment.start_seconds.is_finite()
                || !segment.end_seconds.is_finite()
                || segment.start_seconds < 0.0
                || segment.end_seconds <= segment.start_seconds
                || !segment.bpm.is_finite()
                || segment.bpm <= 0.0
            {
                continue;
            }
            range = Some(match range {
                Some((minimum, maximum)) => (minimum.min(segment.bpm), maximum.max(segment.bpm)),
                None => (segment.bpm, segment.bpm),
            });
        }
        if range.is_none() && !self.variable {
            return self
                .bpm
                .filter(|bpm| bpm.is_finite() && *bpm > 0.0)
                .map(|bpm| (bpm, bpm));
        }
        range
    }

    /// Returns measured local tempo for variable recordings, without extending a segment across an unmeasured gap.
    /// Stable recordings retain their supported global estimate; invalid positions and numbers remain unknown.
    pub fn bpm_at(&self, seconds: f64) -> Option<f64> {
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        let bpm = if self.variable {
            self.segments
                .iter()
                .find(|segment| seconds >= segment.start_seconds && seconds < segment.end_seconds)
                .map(|segment| segment.bpm)
        } else {
            self.bpm
        };
        bpm.filter(|bpm| bpm.is_finite() && *bpm > 0.0)
    }
}

#[derive(Clone)]
pub(super) struct TempoAnalyzer {
    rate: u32,
    channels: usize,
    frames: u64,
    bin: u64,
    energy: f64,
    activity_peak: f64,
    activity: ActivityAnalyzer,
    bass_energy: f64,
    bass_previous: f64,
    bass_envelope: Vec<f64>,
    lowpass: Vec<[f64; 2]>,
    coefficient: f64,
    count: usize,
    previous: f64,
    envelope: Vec<f64>,
    start_seconds: f64,
    segments: Vec<BeatSegment>,
    weak_windows: Vec<context::WeakWindow>,
}

impl TempoAnalyzer {
    /// Allocates a single 32-second energy envelope; PCM is never retained.
    pub(super) fn new(rate: u32, channels: usize) -> Self {
        Self {
            rate,
            channels,
            frames: 0,
            bin: 0,
            energy: 0.0,
            activity_peak: 0.0,
            activity: ActivityAnalyzer::default(),
            bass_energy: 0.0,
            bass_previous: 0.0,
            bass_envelope: Vec::with_capacity(WINDOW),
            lowpass: vec![[0.0; 2]; channels],
            coefficient: 1.0 - (-std::f64::consts::TAU * 200.0 / f64::from(rate)).exp(),
            count: 0,
            previous: 0.0,
            envelope: Vec::with_capacity(WINDOW),
            start_seconds: 0.0,
            segments: Vec::new(),
            weak_windows: Vec::new(),
        }
    }

    /// Accepts already-validated interleaved PCM and preserves opposite-phase channel energy.
    pub(super) fn push(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            let bin = self.frames * ENVELOPE_RATE as u64 / u64::from(self.rate);
            if bin != self.bin {
                self.flush_bin();
                self.bin = bin;
            }
            for (sample, state) in frame.iter().zip(&mut self.lowpass) {
                let sample = f64::from(*sample);
                self.energy += sample * sample;
                self.activity_peak = self.activity_peak.max(sample.abs());
                // Two low-pass stages isolate bass attacks without combining opposite-polarity channels.
                state[0] += self.coefficient * (sample - state[0]);
                state[1] += self.coefficient * (state[0] - state[1]);
                self.bass_energy += state[1] * state[1];
            }
            self.count += self.channels;
            self.frames += 1;
        }
    }

    /// Converts energy rises into onsets, independent of volume and channel polarity.
    fn flush_bin(&mut self) {
        if self.count == 0 {
            return;
        }
        self.activity.observe(
            self.activity_peak,
            self.bin as f64 / ENVELOPE_RATE as f64,
            (self.bin + 1) as f64 / ENVELOPE_RATE as f64,
        );
        self.activity_peak = 0.0;
        let amplitude = (self.energy / self.count as f64).sqrt();
        self.envelope.push((amplitude - self.previous).max(0.0));
        self.previous = amplitude;
        let bass = (self.bass_energy / self.count as f64).sqrt();
        self.bass_envelope
            .push((bass - self.bass_previous).max(0.0));
        self.bass_previous = bass;
        self.bass_energy = 0.0;
        self.energy = 0.0;
        self.count = 0;
        if self.envelope.len() == WINDOW {
            self.flush_window();
        }
    }

    /// Retains only measured beat segments and advances across inconclusive regions without inventing beats.
    fn flush_window(&mut self) {
        let full = estimate(&self.envelope, self.start_seconds);
        let bass = estimate(&self.bass_envelope, self.start_seconds);
        let selected = match (full, bass) {
            (Some(mut full), Some(bass)) => {
                // Preserve broadband tempo unless the bass independently agrees or supplies stronger measured alignment.
                let agrees = (full.bpm / bass.bpm - 1.0).abs() < 0.03;
                let ratio = full.bpm / bass.bpm;
                let octave = (ratio - 2.0).abs() < 0.06 || (ratio - 0.5).abs() < 0.015;
                if octave && bass.beat_confidence > full.beat_confidence {
                    // Bass may attack on alternate beats. Retain those actual attacks, never fill missing ones.
                    full.beats = bass.beats;
                    full.beat_confidence = bass.beat_confidence;
                    full.subdivision_ambiguous = true;
                    Some(full)
                } else if (agrees && bass.beat_confidence > full.beat_confidence)
                    || (bass.beat_confidence.unwrap_or(0.0) > full.confidence)
                {
                    Some(bass)
                } else {
                    Some(full)
                }
            }
            (full, bass) => full.or(bass),
        };
        if self.segments.len() + self.weak_windows.len() < MAX_SEGMENTS {
            if let Some(segment) = selected {
                self.segments.push(segment);
            } else {
                self.weak_windows.push(context::WeakWindow::collect(
                    &self.envelope,
                    &self.bass_envelope,
                    self.start_seconds,
                ));
            }
        }
        self.bass_envelope.clear();
        self.start_seconds += self.envelope.len() as f64 / ENVELOPE_RATE as f64;
        self.envelope.clear();
    }

    /// Summarizes agreeing local estimates; differing local tempos never become a misleading global value.
    pub(super) fn finish(mut self) -> TempoReport {
        self.flush_bin();
        self.flush_window();
        let mut tempos: Vec<f64> = self.segments.iter().map(|segment| segment.bpm).collect();
        tempos.sort_by(f64::total_cmp);
        let median = tempos.get(tempos.len() / 2).copied();
        let variable =
            median.is_some_and(|median| tempos.iter().any(|bpm| (bpm / median - 1.0).abs() > 0.03));
        // Context may fill sparse windows, but must not vote on the independent overall tempo.
        context::recover_windows(&mut self.segments, &self.weak_windows);
        TempoReport {
            activity: self
                .activity
                .finish(self.frames as f64 / f64::from(self.rate)),
            bpm: if variable { None } else { median },
            variable,
            segments: self.segments,
        }
    }
}
