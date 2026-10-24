//! Whole-recording tempo context, with local evidence allowed to contradict the dominant pulse.

use super::{BeatSegment, evidence::WindowEvidence};

/// Revisits every local winner and gap while retaining measured attack provenance separately from tempo.
pub(super) fn resolve_track(windows: &[WindowEvidence]) -> Vec<BeatSegment> {
    let anchor = choose_anchor(windows);
    let mut segments = Vec::new();
    for (index, window) in windows.iter().enumerate() {
        let resolved = resolve_window(window, anchor)
            .or_else(|| {
                let regional = regional_anchor(windows, index, anchor)?;
                Some(contextual_segment(window, regional))
            })
            .or_else(|| recover_contextual_gap(window, anchor));
        if let Some(segment) = resolved {
            segments.push(segment);
        }
    }
    segments
}

/// Confirms a separate local tempo in adjacent windows instead of extending an incompatible track prior.
fn regional_anchor(windows: &[WindowEvidence], index: usize, dominant: Option<f64>) -> Option<f64> {
    let window = &windows[index];
    let mut best = None;
    let mut best_score = 0.0;
    for bpm in window.candidates() {
        if !supports(window, bpm) {
            continue;
        }
        if dominant.is_some_and(|prior| same_metrical_family(bpm, prior)) {
            continue;
        }

        let mut neighboring_support = 0.0_f64;
        let start = index.saturating_sub(1);
        let end = (index + 2).min(windows.len());
        for (offset, evidence) in windows[start..end].iter().enumerate() {
            let neighbor = start + offset;
            if neighbor == index || !supports(evidence, bpm) {
                continue;
            }
            if dominant.is_some_and(|prior| supports(evidence, prior)) {
                continue;
            }
            neighboring_support = neighboring_support.max(evidence.score(bpm) / evidence.maximum());
        }
        if neighboring_support == 0.0 {
            continue;
        }

        // Keep the previous metrical level when several locally supported subdivisions agree.
        // Acoustic evidence still determines whether this is a distinct tempo at all.
        let reference = dominant.unwrap_or(120.0);
        let distance = (bpm / reference).ln() / 0.6;
        let metrical_preference = 0.75 + 0.25 * (-0.5 * distance * distance).exp();
        let score =
            (window.score(bpm) / window.maximum() + neighboring_support) * metrical_preference;
        if score > best_score {
            best = Some(bpm);
            best_score = score;
        }
    }
    best
}

/// Recognizes half/double subdivisions without treating a changed metrical interpretation as a tempo change.
fn same_metrical_family(bpm: f64, reference: f64) -> bool {
    [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0, 1.5, 2.0, 3.0, 4.0]
        .iter()
        .any(|ratio| (bpm / (reference * ratio) - 1.0).abs() < 0.03)
}

/// Checks sparse changes against a previously established tempo, without learning a new clock from them.
fn recover_contextual_gap(window: &WindowEvidence, anchor: Option<f64>) -> Option<BeatSegment> {
    let anchor = anchor?;
    for candidate in window.candidates() {
        if !supports(window, candidate) || (candidate / anchor - 1.0).abs() < 0.03 {
            continue;
        }
        // Without direct recurrence, a slower or unrelated clock must not be erased by fitting
        // a denser phase grid. Direct recurrence plus held-out events can resolve weak ambiguity.
        if !window.has_direct_pulse_support(anchor)
            && (candidate < anchor || !same_metrical_family(candidate, anchor))
        {
            return None;
        }
    }
    // Testing an established hypothesis needs less evidence than discovering a new tempo.
    // Its own interval must still rise above the noise floor and be near the strongest local peak.
    let recurrence = window.score(anchor);
    if window.has_direct_pulse_support(anchor)
        && recurrence >= 0.15
        && recurrence >= window.maximum() * 0.85
    {
        return Some(contextual_segment(window, anchor));
    }
    let support = window.contextual_pulse_support(anchor)?;
    let mut segment = contextual_segment(window, anchor);
    segment.confidence = support.min(0.45);
    Some(segment)
}

/// Selects a repeatedly supported interpretation; a single window cannot bootstrap track-wide certainty.
fn choose_anchor(windows: &[WindowEvidence]) -> Option<f64> {
    let mut candidates: Vec<f64> = Vec::new();
    for window in windows {
        for bpm in window.candidates() {
            if candidates
                .iter()
                .all(|previous| (bpm / previous - 1.0).abs() > 0.005)
            {
                candidates.push(bpm);
            }
        }
    }
    // Each bounded window contributes at most nine alternatives. Score each once;
    // the cached local maximum keeps each candidate/window comparison constant-time.
    let selected = candidates
        .into_iter()
        .map(|bpm| (bpm, anchor_score(windows, bpm)))
        .max_by(|left, right| left.1.total_cmp(&right.1))?
        .0;
    let supported = windows
        .iter()
        .filter(|window| supports(window, selected))
        .count();
    if supported < 2 {
        return None;
    }
    let mut fitted: Vec<(f64, f64)> = windows
        .iter()
        .filter_map(|window| {
            let original = window.original.as_ref()?;
            if (original.bpm / selected - 1.0).abs() <= 0.015 {
                Some((original.bpm, original.confidence * window.duration))
            } else {
                None
            }
        })
        .collect();
    if fitted.is_empty() {
        return Some(selected);
    }
    fitted.sort_by(|left, right| left.0.total_cmp(&right.0));
    let total = fitted.iter().map(|(_, weight)| weight).sum::<f64>();
    let mut accumulated = 0.0;
    for (bpm, weight) in fitted {
        accumulated += weight;
        if accumulated >= total / 2.0 {
            return Some(bpm);
        }
    }
    Some(selected)
}

/// Balances repeated acoustic support with independently selected local pulse levels.
fn anchor_score(windows: &[WindowEvidence], bpm: f64) -> f64 {
    let mut support = 0.0;
    let mut original_support = 0.0;
    for window in windows {
        let maximum = window.maximum();
        if maximum < 0.20 || !window.has_direct_pulse_support(bpm) {
            continue;
        }
        let relative = window.score(bpm) / maximum;
        if relative < 0.60 {
            continue;
        }
        support += relative * window.duration;
        if let Some(original) = &window.original
            && (original.bpm / bpm - 1.0).abs() < 0.015
        {
            original_support += original.confidence * window.duration;
        }
    }
    // A soft musical-pulse preference resolves near-equivalent repetition levels only.
    // Strong measured local evidence can still select slow or fast music outside this preference.
    let log_distance = (bpm / 120.0).ln() / 0.6;
    let pulse_preference = 0.4 + 0.6 * (-0.5 * log_distance * log_distance).exp();
    support * pulse_preference + 0.6 * original_support
}

/// Requires real local periodicity; silence and unrelated attacks cannot inherit a numerical grid.
fn supports(window: &WindowEvidence, bpm: f64) -> bool {
    let score = window.score(bpm);
    if score < 0.22 || !window.has_direct_pulse_support(bpm) {
        return false;
    }
    let maximum = window.maximum();
    if score >= maximum * 0.65 {
        return true;
    }
    // Missing alternate attacks reduce the direct score while retaining the same rhythmic family.
    // Require measured support for the proposed pulse AND its locally stronger related level.
    score >= maximum * 0.40
        && [0.5, 2.0, 1.0 / 3.0, 3.0, 0.25, 4.0, 2.0 / 3.0, 1.5]
            .iter()
            .any(|ratio| window.score(bpm * ratio) >= maximum * 0.85)
}

/// Preserves convincing local changes and otherwise checks the retained track hypothesis against this window.
fn resolve_window(window: &WindowEvidence, anchor: Option<f64>) -> Option<BeatSegment> {
    let Some(anchor) = anchor else {
        return window.original.clone();
    };
    let anchor_supported = supports(window, anchor);
    if let Some(original) = &window.original {
        if (original.bpm / anchor - 1.0).abs() <= 0.015 {
            return Some(original.clone());
        }
        let strongly_measured = original.confidence >= 0.80
            && original
                .beat_confidence
                .is_some_and(|confidence| confidence >= 0.70);
        let continuous_pulse = dense_observed_pulse(original, window.duration);

        // Weak conflicting estimates do not prove a local change when neither interpretation is supported.
        // Keep the interval unknown instead of inventing anchor evidence or declaring variable tempo.
        if !anchor_supported {
            return if strongly_measured || continuous_pulse {
                Some(original.clone())
            } else {
                None
            };
        }
        let strong_disagreement =
            strongly_measured && window.score(anchor) < window.maximum() * 0.85;

        // Small timing jitter can lower autocorrelation while a dense, continuous pulse remains observed.
        let contradictory_pulse = continuous_pulse && contradicts_anchor_phase(original, anchor);
        if strong_disagreement || contradictory_pulse {
            return Some(original.clone());
        }
    }
    if !anchor_supported {
        return None;
    }
    Some(contextual_segment(window, anchor))
}

/// Reports supported tempo without treating delayed spectral changes as precise audible beats.
fn contextual_segment(window: &WindowEvidence, bpm: f64) -> BeatSegment {
    BeatSegment {
        start_seconds: window.start,
        end_seconds: window.start + window.duration,
        bpm,
        confidence: window.score(bpm).clamp(0.45, 0.50),
        subdivision_ambiguous: true,
        // Spectral periodicity establishes tempo, not the exact time of an audible attack.
        beats: Vec::new(),
        beat_confidence: None,
    }
}

/// Requires a continuous observed pulse across the window, rather than unrelated eight-second phase fits.
fn dense_observed_pulse(segment: &BeatSegment, duration: f64) -> bool {
    let period = 60.0 / segment.bpm;
    if (segment.beats.len() as f64) < duration / period * 0.75 {
        return false;
    }
    let Some(reference) = segment.beats.first() else {
        return false;
    };
    let aligned = segment
        .beats
        .iter()
        .filter(|beat| {
            let distance = (**beat - reference) / period;
            (distance - distance.round()).abs() * period <= 0.025
        })
        .count();
    aligned as f64 >= segment.beats.len() as f64 * 0.90
}

/// Rejects a proposed grid only when no phase can explain most observed attacks; a sparse subset is compatible.
fn contradicts_anchor_phase(segment: &BeatSegment, anchor: f64) -> bool {
    let period = 60.0 / anchor;
    let required = segment.beats.len() as f64 * 0.90;
    for reference in &segment.beats {
        let aligned = segment
            .beats
            .iter()
            .filter(|beat| {
                let distance = (**beat - reference) / period;
                (distance - distance.round()).abs() * period <= 0.025
            })
            .count();
        if aligned as f64 >= required {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::tempo::{WINDOW, evidence::PeriodicityAnalyzer};

    /// Generates recurring spectral changes with known source timing, independent of the estimator.
    fn recurring_envelope(bpm: f64) -> Vec<f64> {
        let mut envelope = vec![0.0; WINDOW];
        let mut time = 0.25;
        while time < 32.0 {
            let bin = (time * super::super::ENVELOPE_RATE as f64).round() as usize;
            envelope[bin] = 1.0;
            time += 60.0 / bpm;
        }
        envelope
    }

    /// Creates independently recurring spectral changes without supplying any RMS estimate or beat timestamps.
    fn spectral_window(start: f64, bpm: f64) -> WindowEvidence {
        let envelope = recurring_envelope(bpm);
        PeriodicityAnalyzer::new().collect(start, &[&envelope], None)
    }

    /// Shared slower accents in a known-tempo neighbor cannot independently confirm a new slow region.
    #[test]
    fn regional_confirmation_requires_a_neighbor_incompatible_with_the_prior() {
        let pulse = recurring_envelope(100.0);
        let accents = recurring_envelope(40.0);
        let mut analyzer = PeriodicityAnalyzer::new();
        let windows = [
            analyzer.collect(0.0, &[&pulse, &accents], None),
            analyzer.collect(32.0, &[&pulse, &accents], None),
            analyzer.collect(64.0, &[&accents], None),
        ];
        assert!(supports(&windows[1], 100.0));
        assert!(supports(&windows[1], 40.0));
        assert!(supports(&windows[2], 40.0));
        assert!(regional_anchor(&windows, 2, Some(100.0)).is_none());
        let segments = resolve_track(&windows);
        assert_eq!(segments.len(), 2);
        assert!(
            segments
                .iter()
                .all(|segment| (segment.bpm - 100.0).abs() < 1.0)
        );
    }

    /// Independent adjacent spectral regions can establish a real change even when RMS finds no beat grid.
    #[test]
    fn adjacent_spectral_windows_preserve_a_distinct_local_tempo() {
        let windows = [
            spectral_window(0.0, 120.0),
            spectral_window(32.0, 120.0),
            spectral_window(64.0, 90.0),
            spectral_window(96.0, 90.0),
        ];
        let segments = resolve_track(&windows);
        assert_eq!(segments.len(), 4);
        for (index, segment) in segments.iter().enumerate() {
            let expected = if index < 2 { 120.0 } else { 90.0 };
            assert!((segment.bpm - expected).abs() < 1.0, "{segments:?}");
            assert!(segment.beats.is_empty());
            assert!(segment.beat_confidence.is_none());
        }
    }

    /// One unrelated spectral window cannot bootstrap a new local tempo or inherit an unsupported prior.
    #[test]
    fn isolated_spectral_conflict_stays_unresolved() {
        let windows = [
            spectral_window(0.0, 120.0),
            spectral_window(32.0, 120.0),
            spectral_window(64.0, 90.0),
            spectral_window(96.0, 120.0),
        ];
        let segments = resolve_track(&windows);
        assert_eq!(segments.len(), 3);
        assert!(segments.iter().all(|segment| segment.start_seconds != 64.0));
    }

    /// Unsupported weak conflicts stay unknown while a continuous measured local clock remains authoritative.
    #[test]
    fn unsupported_conflict_requires_convincing_local_evidence() {
        let original = BeatSegment {
            start_seconds: 0.0,
            end_seconds: 32.0,
            bpm: 60.0,
            confidence: 0.5,
            subdivision_ambiguous: false,
            beats: (0..8).map(|beat| 0.25 + f64::from(beat)).collect(),
            beat_confidence: Some(0.5),
        };
        let mut analyzer = PeriodicityAnalyzer::new();
        let mut window = analyzer.collect(0.0, &[&vec![0.0; WINDOW]], Some(original));
        assert!(resolve_window(&window, Some(120.0)).is_none());
        assert!(resolve_window(&window, Some(60.0)).is_some());
        assert!(resolve_window(&window, None).is_some());

        window.original.as_mut().unwrap().beats =
            (0..32).map(|beat| 0.25 + f64::from(beat)).collect();
        let preserved = resolve_window(&window, Some(120.0)).unwrap();
        assert_eq!(preserved.bpm, 60.0);
        assert_eq!(preserved.beats.len(), 32);
    }
}
