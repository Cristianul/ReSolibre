//! Portable original-audio diagnostics with bounded storage, independent of playback and platform decoders.

pub mod activity;
pub mod provenance;
pub mod rhythm;
pub mod tempo;
#[cfg(test)]
mod tests;

use crate::spectrum::{
    AnalysisError, BIN_COUNT, FFT_SIZE, HOP_SIZE, SpectrumAnalyzer, SpectrumFrame,
};
use tempo::{TempoAnalyzer, TempoReport};

pub const ROWS: usize = 256;
pub const MAX_COLUMNS: usize = 512;
pub const MAX_SECONDS: u64 = 4 * 60 * 60;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackAnalysis {
    pub sample_rate_hz: u32,
    pub channels: usize,
    pub duration_seconds: f64,
    pub columns: usize,
    /// Column-major mean power of the strongest FFT bin in each linear-frequency row, in dBFS/bin.
    pub power_db: Vec<f32>,
    pub seconds_per_column: f64,
    pub sustained_bandwidth_hz: Option<f64>,
    pub tempo: TempoReport,
    pub provenance: provenance::ProvenanceReport,
}

/// Small completed measurements retained independently of the large spectrogram matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackAnalysisSummary {
    pub sample_rate_hz: u32,
    pub channels: usize,
    pub duration_seconds: f64,
    pub columns: usize,
    pub seconds_per_column: f64,
    pub sustained_bandwidth_hz: Option<f64>,
    pub tempo: TempoReport,
    pub provenance: provenance::ProvenanceReport,
}

impl From<&TrackAnalysis> for TrackAnalysisSummary {
    /// Copies measurements and observed beats without copying or retaining any spectral bins.
    fn from(report: &TrackAnalysis) -> Self {
        Self {
            sample_rate_hz: report.sample_rate_hz,
            channels: report.channels,
            duration_seconds: report.duration_seconds,
            columns: report.columns,
            seconds_per_column: report.seconds_per_column,
            sustained_bandwidth_hz: report.sustained_bandwidth_hz,
            tempo: report.tempo.clone(),
            provenance: report.provenance,
        }
    }
}

pub struct TrackAnalyzer {
    rate: u32,
    channels: usize,
    frames: u64,
    fft: SpectrumAnalyzer,
    tempo: TempoAnalyzer,
    map: Aggregation,
}

struct Aggregation {
    columns: Vec<[f64; ROWS]>,
    counts: Vec<u64>,
    span: u64,
    active: u64,
    sustained: [u64; BIN_COUNT],
}

impl TrackAnalyzer {
    /// Allocates bounded FFT/aggregation state; adapters retain responsibility for cancellation and I/O.
    pub fn new(rate: u32, channels: usize) -> Result<Self, AnalysisError> {
        Ok(Self {
            rate,
            channels,
            frames: 0,
            fft: SpectrumAnalyzer::new(rate, channels)?,
            tempo: TempoAnalyzer::new(rate, channels),
            map: Aggregation {
                columns: Vec::with_capacity(MAX_COLUMNS),
                counts: Vec::with_capacity(MAX_COLUMNS),
                span: 1,
                active: 0,
                sustained: [0; BIN_COUNT],
            },
        })
    }

    /// Rejects malformed or overlong media before changing any result; source samples are borrowed only.
    pub fn push(&mut self, samples: &[f32]) -> Result<(), AnalysisError> {
        let frames = (samples.len() / self.channels) as u64;
        if self.frames.saturating_add(frames) > u64::from(self.rate) * MAX_SECONDS {
            return Err(AnalysisError::InputTooLarge);
        }
        self.fft.push(samples, |frame| self.map.accept(frame))?;
        self.tempo.push(samples);
        self.frames += frames;
        Ok(())
    }

    /// Reports decoded source time for progress without requiring trusted container duration metadata.
    pub fn decoded_seconds(&self) -> f64 {
        self.frames as f64 / f64::from(self.rate)
    }

    /// Finalizes one complete result; silence remains measurable but carries no tempo/bandwidth claim.
    pub fn finish(mut self) -> Result<TrackAnalysis, AnalysisError> {
        if self.frames == 0 {
            return Err(AnalysisError::InvalidPosition);
        }
        if let Some(frame) = self.fft.finish() {
            self.map.accept(frame);
        }
        let bandwidth = if self.map.active >= 4 {
            self.map
                .sustained
                .iter()
                .enumerate()
                .rev()
                .find(|(_, count)| **count * 5 >= self.map.active)
                .map(|(bin, _)| bin as f64 * f64::from(self.rate) / FFT_SIZE as f64)
        } else {
            None
        };
        let mut power_db = Vec::with_capacity(self.map.columns.len() * ROWS);
        for (column, count) in self.map.columns.iter().zip(&self.map.counts) {
            for sum in column {
                let power = sum / *count as f64;
                power_db.push(if power > 0.0 {
                    (10.0 * power.log10()).max(-120.0) as f32
                } else {
                    -120.0
                });
            }
        }
        Ok(TrackAnalysis {
            sample_rate_hz: self.rate,
            channels: self.channels,
            duration_seconds: self.decoded_seconds(),
            columns: self.map.columns.len(),
            power_db,
            seconds_per_column: self.map.span as f64 * HOP_SIZE as f64 / f64::from(self.rate),
            sustained_bandwidth_hz: bandwidth,
            tempo: self.tempo.finish(),
            provenance: provenance::ProvenanceReport::Unavailable,
        })
    }
}

impl Aggregation {
    /// Merges adjacent time bins when full; memory never grows with track duration.
    fn compact(&mut self) {
        for index in 0..MAX_COLUMNS / 2 {
            for row in 0..ROWS {
                self.columns[index][row] =
                    self.columns[index * 2][row] + self.columns[index * 2 + 1][row];
            }
            self.counts[index] = self.counts[index * 2] + self.counts[index * 2 + 1];
        }
        self.columns.truncate(MAX_COLUMNS / 2);
        self.counts.truncate(MAX_COLUMNS / 2);
        self.span *= 2;
    }

    /// Aggregates channel-preserving power and rejects isolated high-frequency events from sustained bandwidth.
    fn accept(&mut self, frame: &SpectrumFrame) {
        let ordinal = frame.start_frame / HOP_SIZE as u64;
        while ordinal / self.span >= MAX_COLUMNS as u64 {
            self.compact();
        }
        let column = (ordinal / self.span) as usize;
        while self.columns.len() <= column {
            self.columns.push([0.0; ROWS]);
            self.counts.push(0);
        }
        let mut peaks = [-160.0_f32; ROWS];
        let peak = frame.power_db[1..]
            .iter()
            .copied()
            .fold(-160.0_f32, f32::max);
        let threshold = (peak - 50.0).max(-90.0);
        let active = peak > -80.0 && frame.valid_frames == FFT_SIZE;
        if active {
            self.active += 1;
        }
        for (bin, level) in frame.power_db.iter().enumerate() {
            let row = (bin * ROWS / BIN_COUNT).min(ROWS - 1);
            peaks[row] = peaks[row].max(*level);
            if bin > 0 && active && *level >= threshold {
                self.sustained[bin] += 1;
            }
        }
        for (sum, peak) in self.columns[column].iter_mut().zip(peaks) {
            *sum += 10.0_f64.powf(f64::from(peak) / 10.0);
        }
        self.counts[column] += 1;
    }
}
