//! Repeated-lag tempo selection and independent validation of observed beat alignment.

use super::{BeatSegment, ENVELOPE_RATE};

const MAX_LAG: usize = 1200;

struct Candidate {
    period: f64,
    correlation: f64,
    quality: f64,
    matched: Vec<(f64, f64)>,
}

/// Measures repetition separately from beat alignment, so missing attacks do not erase a supported tempo.
pub(super) fn estimate(envelope: &[f64], start: f64) -> Option<BeatSegment> {
    if envelope.len() < ENVELOPE_RATE * 8 {
        return None;
    }
    let peak = envelope.iter().copied().fold(0.0_f64, f64::max);
    if peak < 1e-5 {
        return None;
    }
    let mean = envelope.iter().sum::<f64>() / (envelope.len() as f64 * peak);
    let normalized: Vec<_> = envelope
        .iter()
        .map(|value| (value / peak - mean).max(0.0))
        .collect();
    let scores = onset_scores(&normalized);
    let background = scores[50..].iter().sum::<f64>() / scores[50..].len() as f64;
    let strongest = scores[50..].iter().copied().fold(0.0_f64, f64::max);
    // Positive noise envelopes correlate with their own nonzero floor; require a distinct periodic peak.
    if strongest - background < 0.12 {
        return None;
    }
    let peaks = find_peaks(&normalized, mean.max(0.08));
    if peaks.len() < 8 {
        return None;
    }
    let mut candidates = candidates(&scores, &normalized, &peaks);
    candidates.sort_by(|a, b| b.quality.total_cmp(&a.quality));
    let best = candidates.first()?;
    if best.quality < 0.30 || best.correlation < 0.45 {
        return None;
    }
    // Prefer a supported faster pulse within an integer repetition family.
    // A nearby unrelated correlation peak must never win merely because it is faster.
    let selected = candidates
        .iter()
        .filter(|candidate| {
            (2..=4).any(|multiple| {
                (candidate.period * f64::from(multiple) / best.period - 1.0).abs() < 0.02
            }) && candidate.quality >= best.quality * 0.80
                && candidate.correlation >= best.correlation * 0.80
        })
        .min_by(|a, b| a.period.total_cmp(&b.period))
        .unwrap_or(best);
    let period = if selected.matched.len() >= 8 {
        fit_period(&selected.matched)
    } else {
        selected.period
    };
    let bpm = 60.0 * ENVELOPE_RATE as f64 / period;
    if !bpm.is_finite() || !(39.5..=240.5).contains(&bpm) {
        return None;
    }
    let (beats, beat_confidence) =
        observed_beats(&normalized, &peaks, period, selected.correlation, start);
    Some(BeatSegment {
        start_seconds: start,
        end_seconds: start + envelope.len() as f64 / ENVELOPE_RATE as f64,
        bpm,
        confidence: selected.correlation,
        subdivision_ambiguous: candidates.iter().any(|other| {
            let ratio = other.period / selected.period;
            ((ratio - 2.0).abs() < 0.04 || (ratio - 0.5).abs() < 0.01)
                && other.correlation >= selected.correlation * 0.65
        }),
        beat_confidence,
        beats,
    })
}

/// Checks four successive pulse intervals so a short-lag side peak cannot alone determine tempo.
fn candidates(scores: &[f64; MAX_LAG + 1], normalized: &[f64], peaks: &[usize]) -> Vec<Candidate> {
    let mut comb = Vec::with_capacity(2501);
    for step in 0..=2500 {
        let period = 50.0 + step as f64 / 10.0;
        let mut sum = 0.0;
        for multiple in 1..=4 {
            let position = period * multiple as f64;
            let index = position.floor() as usize;
            let fraction = position - index as f64;
            let value =
                scores[index] * (1.0 - fraction) + scores[(index + 1).min(MAX_LAG)] * fraction;
            sum += value / multiple as f64;
        }
        comb.push(sum / (1.0 + 0.5 + 1.0 / 3.0 + 0.25));
    }
    let mut maxima: Vec<_> = (0..comb.len())
        .filter(|index| {
            comb[*index] >= 0.35
                && (*index == 0 || comb[*index] >= comb[*index - 1])
                && (*index + 1 == comb.len() || comb[*index] >= comb[*index + 1])
        })
        .collect();
    maxima.sort_by(|a, b| comb[*b].total_cmp(&comb[*a]));
    let mut distinct = Vec::new();
    for index in maxima {
        if distinct
            .iter()
            .all(|previous: &usize| index.abs_diff(*previous) >= 20)
        {
            distinct.push(index);
            if distinct.len() == 8 {
                break;
            }
        }
    }
    let maxima = distinct;
    maxima
        .into_iter()
        .map(|index| {
            let period = 50.0 + index as f64 / 10.0;
            let matched = match_beats(normalized, peaks, period);
            let coverage = (matched.len() as f64 * period / normalized.len() as f64).min(1.0);
            Candidate {
                period,
                correlation: comb[index],
                quality: comb[index] * coverage.sqrt(),
                matched,
            }
        })
        .collect()
}

/// Smooths five-millisecond jitter and measures normalized repetition within a fixed six-second lag budget.
fn onset_scores(normalized: &[f64]) -> [f64; MAX_LAG + 1] {
    let mut smoothed = normalized.to_vec();
    for index in 1..normalized.len() - 1 {
        smoothed[index] =
            0.25 * normalized[index - 1] + 0.5 * normalized[index] + 0.25 * normalized[index + 1];
    }
    let mut scores = [0.0_f64; MAX_LAG + 1];
    for (lag, score) in scores.iter_mut().enumerate().skip(50) {
        let mut product = 0.0;
        let mut left = 0.0;
        let mut right = 0.0;
        for index in lag..normalized.len() {
            product += smoothed[index] * smoothed[index - lag];
            left += smoothed[index].powi(2);
            right += smoothed[index - lag].powi(2);
        }
        if left > 0.0 && right > 0.0 {
            *score = product / (left * right).sqrt();
        }
    }
    scores
}

/// Retains isolated attacks above the measured floor; nearby duplicates share one strongest onset.
pub(super) fn find_peaks(normalized: &[f64], threshold: f64) -> Vec<usize> {
    let mut peaks = Vec::new();
    for index in 1..normalized.len() - 1 {
        if normalized[index] < threshold
            || normalized[index] < normalized[index - 1]
            || normalized[index] <= normalized[index + 1]
        {
            continue;
        }
        if let Some(&previous) = peaks.last()
            && index - previous < 20
        {
            if normalized[index] > normalized[previous]
                && let Some(previous) = peaks.last_mut()
            {
                *previous = index;
            }
            continue;
        }
        peaks.push(index);
    }
    peaks
}

/// Finds the strongest phase cluster; averaging opposite subdivisions would put a grid between actual attacks.
fn match_beats(normalized: &[f64], peaks: &[usize], period: f64) -> Vec<(f64, f64)> {
    let mut best_strength = 0.0;
    let mut phase = 0.0;
    for candidate in 0..period.ceil() as usize {
        let candidate = candidate as f64;
        let mut strength = 0.0;
        for index in peaks {
            let beat = ((*index as f64 - candidate) / period).round();
            let residual = *index as f64 - candidate - beat * period;
            strength += normalized[*index] * (1.0 - (residual / 5.0).powi(2)).max(0.0);
        }
        if strength > best_strength {
            best_strength = strength;
            phase = candidate;
        }
    }
    peaks
        .iter()
        .filter_map(|index| {
            let beat = ((*index as f64 - phase) / period).round();
            let residual = *index as f64 - phase - beat * period;
            (residual.abs() <= 5.0).then_some((beat, *index as f64))
        })
        .collect()
}

/// Refines the fractional period from measured attacks without accumulating integer-lag drift.
fn fit_period(matched: &[(f64, f64)]) -> f64 {
    let mean_beat = matched.iter().map(|pair| pair.0).sum::<f64>() / matched.len() as f64;
    let mean_time = matched.iter().map(|pair| pair.1).sum::<f64>() / matched.len() as f64;
    let numerator = matched
        .iter()
        .map(|(beat, time)| (beat - mean_beat) * (time - mean_time))
        .sum::<f64>();
    let denominator = matched
        .iter()
        .map(|(beat, _)| (beat - mean_beat).powi(2))
        .sum::<f64>();
    numerator / denominator
}

/// Rechecks phase in short neighborhoods; only attacks from sufficiently supported regions enter the timeline.
fn observed_beats(
    normalized: &[f64],
    peaks: &[usize],
    period: f64,
    correlation: f64,
    start: f64,
) -> (Vec<f64>, Option<f64>) {
    let mut beats = Vec::new();
    let mut confidence = 1.0_f64;
    for (section, values) in normalized.chunks(8 * ENVELOPE_RATE).enumerate() {
        if values.len() < 8 * ENVELOPE_RATE {
            continue;
        }
        let offset = section * 8 * ENVELOPE_RATE;
        let local: Vec<_> = peaks
            .iter()
            .copied()
            .filter(|index| *index >= offset && *index < offset + values.len())
            .map(|index| index - offset)
            .collect();
        let matched = match_beats(values, &local, period);
        let coverage = (matched.len() as f64 * period / values.len() as f64).min(1.0);
        let score = correlation * coverage;
        if matched.len() < 4 || score < 0.45 {
            continue;
        }
        confidence = confidence.min(score);
        beats.extend(
            matched
                .iter()
                .map(|(_, time)| start + (offset as f64 + time) / ENVELOPE_RATE as f64),
        );
    }
    let confidence = (!beats.is_empty()).then_some(confidence);
    (beats, confidence)
}
