//! Bounded native-rate excerpts keep the detector away from untrusted container lengths.

use crate::original_audio::{self, SourceKey};
use lossprint::Scanner;
use resolibre_core::analysis::{TrackAnalysis, provenance::ProvenanceReport};
use std::fs::File;
use std::io::{Seek, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(90);
const WINDOW_SECONDS: f64 = 14.5;

/// Analyzes the retained original without DSP; detector failures preserve useful tempo/spectrum results.
pub(super) fn analyze(
    file: &File,
    key: SourceKey,
    report: &TrackAnalysis,
    cancel: &AtomicBool,
) -> ProvenanceReport {
    if !resolibre_core::analysis::provenance::supports_format(
        report.sample_rate_hz,
        report.channels,
    ) {
        return ProvenanceReport::Unsupported;
    }
    if report.duration_seconds < 2.0 {
        return ProvenanceReport::InsufficientAudio;
    }
    let Ok(input) = crate::platform::independent_file(file) else {
        return ProvenanceReport::Unavailable;
    };
    let mut detector = Excerpts::new(
        report.sample_rate_hz,
        report.channels,
        report.duration_seconds,
    );
    let result =
        original_audio::native::decode_prefix(input, key, cancel, DEADLINE, |reader, started| {
            let limit = detector
                .plan
                .last()
                .ok_or(())?
                .1
                .div_ceil(u64::from(detector.rate)) as u32;
            original_audio::pcm::stream_prefix(
                reader,
                cancel,
                started,
                DEADLINE,
                Some(limit),
                |rate, channels, samples| {
                    if (rate, channels) != (detector.rate, detector.channels) {
                        return Err(());
                    }
                    detector.push(samples, cancel)
                },
            )?;
            Ok(())
        });
    if result.is_err() || detector.next != detector.plan.len() || cancel.load(Ordering::Relaxed) {
        return ProvenanceReport::Unavailable;
    }
    detector.finish()
}

struct Excerpts {
    rate: u32,
    channels: usize,
    plan: Vec<(u64, u64)>,
    next: usize,
    position: u64,
    samples: Vec<f32>,
    scanner: Option<Scanner>,
    scores: Vec<f32>,
}

impl Excerpts {
    /// Keeps one excerpt (at most 21.3 MiB at 192 kHz stereo), irrespective of track length.
    fn new(rate: u32, channels: usize, seconds: f64) -> Self {
        let plan = plan(rate, seconds);
        let capacity = (plan[0].1 - plan[0].0) as usize * channels;
        Self {
            rate,
            channels,
            plan,
            next: 0,
            position: 0,
            samples: Vec::with_capacity(capacity),
            scanner: None,
            scores: Vec::with_capacity(6),
        }
    }

    /// Collects non-overlapping excerpts; pipe backpressure pauses decoding while CPU inference runs.
    fn push(&mut self, samples: &[f32], cancel: &AtomicBool) -> Result<(), ()> {
        if cancel.load(Ordering::Relaxed) {
            return Err(());
        }
        let end = self.position + (samples.len() / self.channels) as u64;
        while let Some(&(start, stop)) = self.plan.get(self.next) {
            let first = start.max(self.position);
            let last = stop.min(end);
            if first < last {
                let from = (first - self.position) as usize * self.channels;
                let to = (last - self.position) as usize * self.channels;
                self.samples.extend_from_slice(&samples[from..to]);
            }
            if end < stop {
                break;
            }
            self.score(cancel)?;
            self.samples.clear();
            self.next += 1;
        }
        self.position = end;
        Ok(())
    }

    /// Scores a private, self-generated WAV, never an arbitrary file through lossprint's unbounded reader.
    fn score(&mut self, cancel: &AtomicBool) -> Result<(), ()> {
        if cancel.load(Ordering::Relaxed) {
            return Err(());
        }
        let mean_power = self
            .samples
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum::<f64>()
            / self.samples.len().max(1) as f64;
        // Silence and near-silence are outside useful musical evidence, even if the model gives a score.
        if mean_power < 1e-7 {
            return Ok(());
        }
        let wave = scratch_wave(self.rate, self.channels, &self.samples).map_err(|_| ())?;
        if self.scanner.is_none() {
            self.scanner = Some(Scanner::new().map_err(|_| ())?);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(());
        }
        let score = self
            .scanner
            .as_ref()
            .ok_or(())?
            .score_file(wave)
            .map_err(|_| ())?;
        let value = score.transcode_probability();
        if !value.is_finite() || !(0.0..=1.0).contains(&value) || cancel.load(Ordering::Relaxed) {
            return Err(());
        }
        self.scores.push(value);
        Ok(())
    }

    /// Pools only usable excerpts and preserves disagreement instead of silently asserting authenticity.
    fn finish(self) -> ProvenanceReport {
        if self.scores.is_empty() {
            return ProvenanceReport::InsufficientAudio;
        }
        let minimum_score = self.scores.iter().copied().fold(1.0_f32, f32::min);
        let maximum_score = self.scores.iter().copied().fold(0.0_f32, f32::max);
        let lossy_score = (self.scores.iter().sum::<f32>() / self.scores.len() as f32)
            .clamp(minimum_score, maximum_score);
        ProvenanceReport::Measured {
            lossy_score,
            minimum_score,
            maximum_score,
            windows: self.scores.len() as u8,
        }
    }
}

/// Spreads up to six non-overlapping windows through the middle 92%; short tracks use one full excerpt.
fn plan(rate: u32, seconds: f64) -> Vec<(u64, u64)> {
    let total = (seconds * f64::from(rate)).round() as u64;
    let length = (WINDOW_SECONDS * f64::from(rate)) as u64;
    if total <= length {
        return vec![(0, total)];
    }
    let first = (total as f64 * 0.04) as u64;
    let available = (total as f64 * 0.92) as u64;
    let count = (available / length).clamp(1, 6);
    if count == 1 {
        let start = (total - length) / 2;
        return vec![(start, start + length)];
    }
    let last = total - first - length;
    (0..count)
        .map(|index| {
            let start = first + (last - first) * index / (count - 1);
            (start, start + length)
        })
        .collect()
}

/// Creates an automatically deleted, private temporary file containing unchanged float32 PCM.
fn scratch_wave(rate: u32, channels: usize, samples: &[f32]) -> std::io::Result<File> {
    let size = (samples.len() * 4) as u32;
    check_file_limit(u64::from(size) + 44)?;
    let mut file = tempfile::tempfile()?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + size).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16_u32.to_le_bytes())?;
    file.write_all(&3_u16.to_le_bytes())?;
    file.write_all(&(channels as u16).to_le_bytes())?;
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&(rate * channels as u32 * 4).to_le_bytes())?;
    file.write_all(&(channels as u16 * 4).to_le_bytes())?;
    file.write_all(&32_u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&size.to_le_bytes())?;
    let mut buffer = [0_u8; 65536];
    for chunk in samples.chunks(buffer.len() / 4) {
        for (value, bytes) in chunk.iter().zip(buffer.as_chunks_mut::<4>().0) {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        file.write_all(&buffer[..chunk.len() * 4])?;
    }
    file.rewind()?;
    Ok(file)
}

/// Rejects a restrictive process file quota before writes can raise a fatal SIGXFSZ on Unix.
fn check_file_limit(bytes: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit only fills the correctly sized owned structure; no process state is changed.
        if unsafe { libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if bytes > limit.rlim_cur {
            return Err(std::io::Error::other(
                "Detector scratch file exceeds process limit",
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = bytes;
    Ok(())
}

#[cfg(test)]
mod tests;
