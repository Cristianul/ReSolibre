//! Sparse measured attacks may inherit a stable track tempo, but never authorize a synthetic beat grid.

use super::{BeatSegment, ENVELOPE_RATE, MAX_SEGMENTS};

const MAX_ATTACKS: usize = 128;

#[derive(Clone)]
pub(super) struct WeakWindow {
    start: f64,
    duration: f64,
    attacks: [Vec<f32>; 2],
}

impl WeakWindow {
    /// Retains only bounded attack times, not PCM or complete envelopes, for a later context check.
    pub(super) fn collect(full: &[f64], bass: &[f64], start: f64) -> Self {
        Self {
            start,
            duration: full.len() as f64 / ENVELOPE_RATE as f64,
            attacks: [collect_attacks(full), collect_attacks(bass)],
        }
    }

    /// Requires multiple actual notes near an established phase; silence and unrelated attacks remain unknown.
    fn recover(&self, anchor: &BeatSegment) -> Option<BeatSegment> {
        let period = 60.0 / anchor.bpm;
        let reference = anchor.beats.iter().copied().min_by(|left, right| {
            (left - self.start)
                .abs()
                .total_cmp(&(right - self.start).abs())
        })?;
        if (reference - self.start).abs() > 64.0 || self.duration < 4.0 {
            return None;
        }

        let tolerance = (period * 0.12).clamp(0.025, 0.06);
        let mut best = None;
        for attacks in &self.attacks {
            if attacks.len() < 4 {
                continue;
            }

            let beats: Vec<f64> = attacks
                .iter()
                .filter_map(|time| {
                    let time = self.start + f64::from(*time);
                    let nearest = reference + ((time - reference) / period).round() * period;
                    ((time - nearest).abs() <= tolerance).then_some(time)
                })
                .collect();
            let support = beats.len() as f64 / attacks.len() as f64;
            if beats.len() < 4 || support < 0.65 {
                continue;
            }
            let span = beats[beats.len() - 1] - beats[0];
            if span < self.duration * 0.4 {
                continue;
            }

            if best
                .as_ref()
                .is_some_and(|previous: &BeatSegment| previous.beats.len() >= beats.len())
            {
                continue;
            }
            best = Some(BeatSegment {
                start_seconds: self.start,
                end_seconds: self.start + self.duration,
                bpm: anchor.bpm,
                // The displayed estimate benefits from context; native SYNC still requires independent evidence.
                confidence: anchor.confidence.min(0.5),
                subdivision_ambiguous: true,
                beat_confidence: Some(0.45 * support),
                beats,
            });
        }
        best
    }
}

/// Uses independently measured, agreeing track tempo as a prior; recovered windows never become anchors.
pub(super) fn recover_windows(segments: &mut Vec<BeatSegment>, windows: &[WeakWindow]) {
    if segments.is_empty() || windows.is_empty() {
        return;
    }

    let mut tempos: Vec<_> = segments.iter().map(|segment| segment.bpm).collect();
    tempos.sort_by(f64::total_cmp);
    let median = tempos[tempos.len() / 2];
    if tempos.iter().any(|bpm| (bpm / median - 1.0).abs() > 0.03) {
        return;
    }

    let mut recovered = Vec::new();
    for window in windows {
        let anchor = segments
            .iter()
            .filter(|segment| {
                segment
                    .beat_confidence
                    .is_some_and(|confidence| confidence >= 0.55)
            })
            .min_by(|left, right| {
                distance(left, window.start).total_cmp(&distance(right, window.start))
            });
        if let Some(anchor) = anchor
            && let Some(segment) = window.recover(anchor)
        {
            recovered.push(segment);
        }
        if segments.len() + recovered.len() >= MAX_SEGMENTS {
            break;
        }
    }
    segments.extend(recovered);
    segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
}

/// Measures proximity to a known interval without privileging a later or earlier anchor.
fn distance(segment: &BeatSegment, time: f64) -> f64 {
    if time < segment.start_seconds {
        segment.start_seconds - time
    } else {
        (time - segment.end_seconds).max(0.0)
    }
}

/// Keeps the strongest 128 resolved attacks in source order; the lower threshold still requires phase agreement later.
fn collect_attacks(envelope: &[f64]) -> Vec<f32> {
    let peak = envelope.iter().copied().fold(0.0_f64, f64::max);
    if peak < 1e-5 {
        return Vec::new();
    }

    let mut peaks = super::estimate::find_peaks(envelope, peak * 0.04);
    peaks.sort_by(|left, right| envelope[*right].total_cmp(&envelope[*left]));
    peaks.truncate(MAX_ATTACKS);
    peaks.sort_unstable();
    peaks
        .into_iter()
        .map(|index| index as f32 / ENVELOPE_RATE as f32)
        .collect()
}
