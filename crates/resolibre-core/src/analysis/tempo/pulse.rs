//! Sparse spectral changes can verify an established tempo without supplying measured beat positions.

use super::{ENVELOPE_RATE, estimate::find_peaks};

const BANDS: usize = 4;
const EVENTS_PER_HALF: usize = 32;
const MIN_MATCHES: usize = 4;
const MAX_CHANCE_TAIL: f64 = 0.001 / (BANDS * 4) as f64;

#[derive(Clone, Copy)]
struct Event {
    bin: u16,
    strength: u16,
}

#[derive(Clone)]
pub(super) struct PulseEvidence {
    duration_bins: usize,
    events: [Vec<Event>; BANDS],
}

struct Alignment {
    matches: usize,
    count_fraction: f64,
    strength_fraction: f64,
    first: usize,
    last: usize,
}

impl PulseEvidence {
    /// Retains up to 64 significant changes per band, balanced across the two temporal halves.
    /// Packed source-bin positions and relative strengths keep 512 windows below 576 KiB.
    pub(super) fn collect(envelopes: &[&[f64]]) -> Self {
        let duration_bins = envelopes.first().map_or(0, |envelope| envelope.len());
        let mut events = std::array::from_fn(|_| Vec::new());
        if (8 * ENVELOPE_RATE..=usize::from(u16::MAX)).contains(&duration_bins) {
            for (destination, envelope) in events.iter_mut().zip(envelopes) {
                if envelope.len() == duration_bins {
                    *destination = collect_events(envelope);
                }
            }
        }
        Self {
            duration_bins,
            events,
        }
    }

    /// Tests a fixed BPM and its first four subdivisions with a locally fitted, held-out phase.
    /// The caller must establish the tempo elsewhere and preserve independently supported changes.
    /// Returned concentration is contextual support, never beat confidence or calibrated probability.
    pub(super) fn support(&self, bpm: f64) -> Option<f64> {
        if !bpm.is_finite() || !(40.0..=240.0).contains(&bpm) {
            return None;
        }
        let mut best: Option<f64> = None;
        for events in &self.events {
            let split =
                events.partition_point(|event| usize::from(event.bin) < self.duration_bins / 2);
            let (training, validation) = events.split_at(split);
            if training.len() < MIN_MATCHES || validation.len() < MIN_MATCHES {
                continue;
            }
            for subdivision in 1..=4 {
                let period = 60.0 * ENVELOPE_RATE as f64 / (bpm * f64::from(subdivision));
                let tolerance = (period * 0.08).clamp(1.0, 12.0);
                let phase = fit_phase(training, period, tolerance);
                let fitted = alignment(training, phase, period, tolerance);
                let held_out = alignment(validation, phase, period, tolerance);
                if fitted.matches < MIN_MATCHES || held_out.matches < MIN_MATCHES {
                    continue;
                }
                if held_out.last - fitted.first < self.duration_bins * 2 / 5 {
                    continue;
                }

                // Phase is selected using only the first half. Uniform unrelated held-out events
                // occupy this fraction of a cycle by chance; sparse coincidences do not suffice.
                // The rejection threshold also accounts for all band/subdivision hypotheses.
                let chance = (2.0 * tolerance / period).min(1.0);
                let concentration = fitted
                    .count_fraction
                    .min(fitted.strength_fraction)
                    .min(held_out.count_fraction)
                    .min(held_out.strength_fraction);
                if concentration < 0.35
                    || concentration < chance + 0.20
                    || binomial_tail(validation.len(), held_out.matches, chance) > MAX_CHANCE_TAIL
                {
                    continue;
                }
                let support = (concentration - chance) / (1.0 - chance);
                best = Some(best.map_or(support, |previous| previous.max(support)));
            }
        }
        best
    }
}

/// Smooths alternating spectral hops and rejects flat residual energy before retaining isolated peaks.
fn collect_events(envelope: &[f64]) -> Vec<Event> {
    if envelope
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Vec::new();
    }
    let mut smoothed = envelope.to_vec();
    for index in 2..envelope.len() - 2 {
        smoothed[index] = (envelope[index - 2]
            + 2.0 * envelope[index - 1]
            + 2.0 * envelope[index]
            + 2.0 * envelope[index + 1]
            + envelope[index + 2])
            / 8.0;
    }
    let peak = smoothed.iter().copied().fold(0.0_f64, f64::max);
    let mean = smoothed.iter().sum::<f64>() / smoothed.len() as f64;
    if peak < 1e-8 || peak < mean * 1.5 {
        return Vec::new();
    }
    let peaks = find_peaks(&smoothed, (peak * 0.05).max(mean));
    let middle = envelope.len() / 2;
    let mut retained = Vec::new();
    for first_half in [true, false] {
        let mut selected: Vec<_> = peaks
            .iter()
            .copied()
            .filter(|index| (*index < middle) == first_half)
            .collect();
        selected.sort_by(|left, right| smoothed[*right].total_cmp(&smoothed[*left]));
        selected.truncate(EVENTS_PER_HALF);
        retained.extend(selected.into_iter().map(|index| Event {
            bin: index as u16,
            strength: (smoothed[index] / peak * f64::from(u16::MAX)).round() as u16,
        }));
    }
    retained.sort_by_key(|event| event.bin);
    retained
}

/// Fits only phase at the supplied period; event strength cannot compensate for a poor count fit.
fn fit_phase(events: &[Event], period: f64, tolerance: f64) -> f64 {
    let mut phase = f64::from(events[0].bin);
    let mut best = -1.0;
    for event in events {
        let candidate = f64::from(event.bin);
        let measured = alignment(events, candidate, period, tolerance);
        let score = measured.count_fraction.min(measured.strength_fraction);
        if score > best {
            best = score;
            phase = candidate;
        }
    }
    phase
}

/// Counts independent observed changes near a phase, without filling any missing pulse positions.
fn alignment(events: &[Event], phase: f64, period: f64, tolerance: f64) -> Alignment {
    let total_strength: f64 = events.iter().map(|event| f64::from(event.strength)).sum();
    let mut matches = 0;
    let mut matched_strength = 0.0;
    let mut first = usize::MAX;
    let mut last = 0;
    for event in events {
        let cycles = (f64::from(event.bin) - phase) / period;
        if (cycles - cycles.round()).abs() * period > tolerance {
            continue;
        }
        matches += 1;
        matched_strength += f64::from(event.strength);
        first = first.min(usize::from(event.bin));
        last = last.max(usize::from(event.bin));
    }
    Alignment {
        matches,
        count_fraction: matches as f64 / events.len() as f64,
        strength_fraction: matched_strength / total_strength.max(1.0),
        first,
        last,
    }
}

/// Bounds accidental held-out phase agreement under a uniform independent-event reference model.
/// This is a rejection heuristic; musical events are not assumed statistically independent.
fn binomial_tail(trials: usize, minimum: usize, chance: f64) -> f64 {
    let mut probability = (1.0 - chance).powi(trials as i32);
    let mut tail = 0.0;
    for successes in 0..=trials {
        if successes >= minimum {
            tail += probability;
        }
        if successes < trials {
            probability *=
                (trials - successes) as f64 / (successes + 1) as f64 * chance / (1.0 - chance);
        }
    }
    tail
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates bounded novelty envelopes from actual change times, independent of any beat output.
    fn changes(seconds: usize, times: &[f64]) -> Vec<f64> {
        let mut envelope = vec![0.0; seconds * ENVELOPE_RATE];
        for (index, time) in times.iter().enumerate() {
            let bin = (*time * ENVELOPE_RATE as f64).round() as usize;
            if bin < envelope.len() {
                envelope[bin] = 1.0 + (index % 3) as f64 * 0.2;
            }
        }
        envelope
    }

    /// Places sparse melodic changes on a known pulse while deliberately omitting most beat positions.
    #[test]
    fn sparse_melodic_changes_verify_tempo_without_a_dense_grid() {
        for bpm in [60.0, 100.0, 140.0] {
            let period = 60.0 / bpm;
            let times: Vec<_> = (0..64)
                .filter(|beat| beat % 4 == 0 || beat % 7 == 0)
                .map(|beat| 0.25 + f64::from(beat) * period)
                .filter(|time| *time < 32.0)
                .collect();
            let envelope = changes(32, &times);
            let evidence = PulseEvidence::collect(&[&envelope]);
            assert!(evidence.support(bpm).is_some(), "{bpm}");
        }
    }

    /// Subdivision-aligned notes can support contextual tempo without choosing a canonical beat level.
    #[test]
    fn irregular_subdivision_notes_verify_only_the_known_family() {
        for subdivision in [2, 3, 4] {
            let period = 0.6 / f64::from(subdivision);
            let times: Vec<_> = (0..220)
                .filter(|step| step % 5 == 0 || step % 7 == 2)
                .map(|step| 0.21 + f64::from(step) * period)
                .filter(|time| *time < 32.0)
                .collect();
            let envelope = changes(32, &times);
            let evidence = PulseEvidence::collect(&[&envelope]);
            assert!(evidence.support(100.0).is_some(), "{subdivision}");
            assert!(evidence.support(113.0).is_none(), "{subdivision}");
        }
    }

    /// Fitting the first half cannot excuse a later phase reset or an unrelated tempo.
    #[test]
    fn held_out_phase_reset_and_tempo_changes_are_rejected() {
        for changed_tempo in [false, true] {
            let times: Vec<_> = (0..64)
                .map(|beat| {
                    if beat < 32 {
                        0.25 + f64::from(beat) * 0.5
                    } else if changed_tempo {
                        16.25 + f64::from(beat - 32) * 0.57
                    } else {
                        16.25 + f64::from(beat - 32) * 0.5 + 0.047
                    }
                })
                .filter(|time| *time < 32.0)
                .collect();
            let envelope = changes(32, &times);
            assert!(
                PulseEvidence::collect(&[&envelope])
                    .support(120.0)
                    .is_none()
            );
        }
    }

    /// Silence, residual sustained modulation, and seeded unrelated changes provide no contextual clock.
    #[test]
    fn noise_and_stationary_modulation_do_not_verify_a_tempo() {
        let silence = vec![0.0; 32 * ENVELOPE_RATE];
        let stationary: Vec<_> = (0..silence.len())
            .map(|bin| 0.01 + 0.0001 * (bin as f64 * 0.1).sin())
            .collect();
        let mut state = 17_u64;
        let noise: Vec<_> = (0..silence.len())
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                (state >> 32) as f64 / f64::from(u32::MAX)
            })
            .collect();
        for envelope in [silence, stationary, noise] {
            let evidence = PulseEvidence::collect(&[&envelope]);
            for bpm in [40.0, 60.0, 75.0, 100.0, 120.0, 140.0, 180.0, 240.0] {
                assert!(evidence.support(bpm).is_none(), "{bpm}");
            }
        }
    }

    /// Multiple unrelated noise realizations must not pass by searching several subdivision phases.
    #[test]
    fn repeated_noise_trials_do_not_gain_support_from_hypothesis_search() {
        for seed in 1_u64..=16 {
            let mut state = seed;
            let noise: Vec<_> = (0..32 * ENVELOPE_RATE)
                .map(|_| {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005_u64)
                        .wrapping_add(1);
                    (state >> 32) as f64 / f64::from(u32::MAX)
                })
                .collect();
            let evidence = PulseEvidence::collect(&[&noise]);
            for bpm in [40.0, 60.0, 75.0, 100.0, 120.0, 140.0, 180.0, 240.0] {
                assert!(evidence.support(bpm).is_none(), "seed={seed}, bpm={bpm}");
            }
        }
    }

    /// Invalid or tiny input remains unsupported, and retained event storage has an explicit maximum.
    #[test]
    fn invalid_inputs_and_storage_are_bounded() {
        let envelope = vec![1.0; 32 * ENVELOPE_RATE];
        let mut invalid = envelope.clone();
        invalid[10] = f64::NAN;
        for values in [&[][..], &envelope[..100], invalid.as_slice()] {
            let evidence = PulseEvidence::collect(&[values]);
            assert!(evidence.support(120.0).is_none());
        }
        let evidence = PulseEvidence::collect(&[envelope.as_slice(); BANDS]);
        for bpm in [f64::NAN, f64::INFINITY, 0.0, 39.0, 241.0] {
            assert!(evidence.support(bpm).is_none());
        }
        let per_window = std::mem::size_of::<PulseEvidence>()
            + BANDS * EVENTS_PER_HALF * 2 * std::mem::size_of::<Event>();
        assert!(per_window * 512 < 576 * 1024);
    }
}
