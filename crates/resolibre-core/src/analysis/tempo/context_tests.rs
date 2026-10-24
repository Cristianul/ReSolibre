//! Whole-recording rhythm regressions use streamed, owned signals with known musical changes.

use super::TempoReport;
use crate::analysis::rhythm::RhythmAnalyzer;

const RATE: u32 = 8_000;
const BLOCK_FRAMES: usize = 1_024;
const SECTION_SECONDS: usize = 32;
const FIRST_ATTACK: f64 = 0.25;

/// Streams generated mono samples through the public analyzer without retaining the recording.
fn analyze_generated(seconds: usize, mut sample: impl FnMut(f64) -> f32) -> TempoReport {
    let mut analyzer = RhythmAnalyzer::new(RATE, 1).unwrap();
    let mut block = Vec::with_capacity(BLOCK_FRAMES);
    let total_frames = seconds * RATE as usize;

    for frame in 0..total_frames {
        block.push(sample(frame as f64 / f64::from(RATE)));
        if block.len() == BLOCK_FRAMES {
            analyzer.push(&block).unwrap();
            block.clear();
        }
    }

    if !block.is_empty() {
        analyzer.push(&block).unwrap();
    }

    analyzer.finish()
}

/// Produces one decaying pitched attack; its onset is explicit rather than inferred by the test.
fn attack(elapsed: f64, frequency: f64, decay: f64, duration: f64) -> f64 {
    if !(0.0..duration).contains(&elapsed) {
        return 0.0;
    }

    (std::f64::consts::TAU * frequency * elapsed).sin() * (-decay * elapsed).exp()
}

/// Places periodic attacks on a known clock, preserving the first attack across decoder blocks.
fn periodic_attack(time: f64, bpm: f64, frequency: f64, decay: f64, duration: f64) -> f64 {
    if time < FIRST_ATTACK {
        return 0.0;
    }

    let elapsed = (time - FIRST_ATTACK).rem_euclid(60.0 / bpm);
    attack(elapsed, frequency, decay, duration)
}

/// Supplies reproducible quiet broadband interference without storing a noise buffer.
fn next_noise(state: &mut u64) -> f64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1);
    f64::from((*state >> 32) as u32) / f64::from(u32::MAX) - 0.5
}

/// Recreates the observed failure: a steady low kick remains while doubled hi-hats become louder.
fn changing_hat_recording(bpm: f64) -> TempoReport {
    let hat_gains = [0.01, 0.2, 0.6, 0.01];
    let mut noise_state = 17_u64;

    analyze_generated(SECTION_SECONDS * hat_gains.len(), |time| {
        let section = (time as usize / SECTION_SECONDS).min(hat_gains.len() - 1);
        let kick = periodic_attack(time, bpm, 60.0, 30.0, 0.15) * 0.4;
        let hat = periodic_attack(time, bpm * 2.0, 1_800.0, 110.0, 0.035) * hat_gains[section];
        let harmony = (std::f64::consts::TAU * 220.0 * time).sin() * 0.08
            + (std::f64::consts::TAU * 330.0 * time).sin() * 0.04;
        let noise = next_noise(&mut noise_state) * 0.005;

        (kick + hat + harmony + noise) as f32
    })
}

/// Checks the global and measured local estimates without requiring any particular segment count.
fn assert_stable_tempo(report: &TempoReport, expected: f64) {
    assert!(!report.variable, "arrangement became variable: {report:?}");
    assert!(
        report.bpm.is_some_and(|bpm| (bpm - expected).abs() < 1.0),
        "expected supported {expected} BPM: {report:?}"
    );
    assert!(
        !report.segments.is_empty(),
        "no measured rhythm: {report:?}"
    );

    for segment in &report.segments {
        assert!(
            (segment.bpm - expected).abs() < 1.0,
            "a changing instrument replaced the track pulse: {segment:?}"
        );
    }
}

/// A trusted beat must coincide with a generated attack, even when the accepted tempo changes.
fn assert_trusted_beats_have_attacks(report: &TempoReport, attack_bpm: impl Fn(f64) -> f64) {
    for segment in &report.segments {
        if !segment
            .beat_confidence
            .is_some_and(|confidence| confidence >= 0.55)
        {
            continue;
        }

        for beat in &segment.beats {
            let period = 60.0 / attack_bpm(*beat);
            let nearest = ((beat - FIRST_ATTACK) / period).round() * period + FIRST_ATTACK;
            assert!(
                *beat >= FIRST_ATTACK && (beat - nearest).abs() < 0.03,
                "trusted beat has no nearby source attack: {beat}"
            );
        }
    }
}

/// A stable kick and clear opening/closing anchors survive changing hi-hat prominence.
#[test]
fn stable_kick_survives_louder_doubled_hats() {
    let report = changing_hat_recording(120.0);
    assert_trusted_beats_have_attacks(&report, |_| 240.0);
    assert_stable_tempo(&report, 120.0);
}

/// The same arrangement regression works away from the original 120 BPM reproduction.
#[test]
fn stable_kick_context_is_not_specific_to_one_tempo() {
    for bpm in [100.0, 140.0] {
        let report = changing_hat_recording(bpm);
        assert_trusted_beats_have_attacks(&report, |_| bpm * 2.0);
        assert_stable_tempo(&report, bpm);
    }
}

/// On-grid pitched arpeggios and alternate kick accents do not create new local tempos.
#[test]
fn stable_pulse_survives_arpeggios_and_alternate_accents() {
    let report = analyze_generated(128, |time| {
        let section = time as usize / SECTION_SECONDS;
        let beat_number = ((time - FIRST_ATTACK).max(0.0) * 2.0).floor() as usize;
        let kick_gain = if section == 2 && beat_number % 2 == 1 {
            0.18
        } else {
            0.4
        };
        let kick = periodic_attack(time, 120.0, 60.0, 30.0, 0.15) * kick_gain;
        let arpeggio = if section == 1 || section == 2 {
            let note = ((time - FIRST_ATTACK).max(0.0) * 4.0).floor() as usize;
            let frequency = [220.0, 275.0, 330.0, 440.0][note % 4];
            periodic_attack(time, 240.0, frequency, 12.0, 0.2) * 0.15
        } else {
            0.0
        };

        (kick + arpeggio) as f32
    });

    assert_trusted_beats_have_attacks(&report, |time| {
        if (32.0..96.0).contains(&time) {
            240.0
        } else {
            120.0
        }
    });
    assert_stable_tempo(&report, 120.0);
}

/// Sparse supported notes inherit tempo while remaining actual onsets below the synchronization gate.
#[test]
fn contextual_notes_preserve_onsets_without_fabricating_a_grid() {
    let note_times = [1.25, 3.75, 8.25, 11.25, 17.75, 23.25, 28.75];
    let report = analyze_generated(96, |time| {
        if !(32.0..64.0).contains(&time) {
            return (periodic_attack(time, 120.0, 900.0, 110.0, 0.035) * 0.6) as f32;
        }

        let local_time = time - 32.0;
        let mut value = 0.0;
        for (index, note) in note_times.iter().enumerate() {
            let elapsed = local_time - note;
            let soft_attack = (elapsed / 0.01).clamp(0.0, 1.0);
            value += attack(elapsed, 440.0 + index as f64 * 55.0, 9.0, 0.5) * soft_attack * 0.04;
        }

        value as f32
    });

    assert_stable_tempo(&report, 120.0);
    let contextual: Vec<_> = report
        .segments
        .iter()
        .filter(|segment| segment.start_seconds >= 32.0 && segment.end_seconds <= 64.0)
        .collect();
    assert!(
        !contextual.is_empty(),
        "supported notes were lost: {report:?}"
    );

    for segment in contextual {
        assert!(
            segment.confidence <= 0.5,
            "context became independent evidence"
        );
        assert!(
            segment
                .beat_confidence
                .is_none_or(|confidence| confidence < 0.55),
            "context alone cannot authorize synchronization"
        );
        assert!(!segment.beats.is_empty(), "recovery needs measured notes");
        for beat in &segment.beats {
            assert!(
                note_times
                    .iter()
                    .any(|note| (beat - 32.0 - note).abs() < 0.06),
                "the reported beat has no source attack: {beat}"
            );
        }
    }
}

/// Generates separately supported tempo sections whose pulse clocks genuinely change at the boundary.
fn changing_tempo_recording(tempos: &[f64]) -> TempoReport {
    analyze_generated(SECTION_SECONDS * tempos.len(), |time| {
        let section = time as usize / SECTION_SECONDS;
        let local_time = time - (section * SECTION_SECONDS) as f64;
        (periodic_attack(local_time, tempos[section], 900.0, 110.0, 0.035) * 0.6) as f32
    })
}

/// Conflicting well-supported tempi stay local rather than being forced onto the strongest anchor.
#[test]
fn real_tempo_changes_survive_context_including_half_tempo() {
    for tempos in [[100.0, 140.0], [140.0, 70.0], [70.0, 140.0]] {
        let report = changing_tempo_recording(&tempos);
        assert!(report.variable, "real change erased: {report:?}");
        assert!(
            report.bpm.is_none(),
            "variable track received one global tempo"
        );

        for (section, expected) in tempos.into_iter().enumerate() {
            let position = (section * SECTION_SECONDS + SECTION_SECONDS / 2) as f64;
            assert!(
                report
                    .bpm_at(position)
                    .is_some_and(|bpm| (bpm - expected).abs() < 1.0),
                "expected measured {expected} BPM at {position}: {report:?}"
            );
        }
    }
}

/// A small amount of natural attack jitter must not erase either direction of a genuine tempo change.
#[test]
fn jittered_tempo_change_keeps_continuous_observed_pulse() {
    for (first_bpm, next_bpm) in [(70.0, 140.0), (140.0, 70.0)] {
        let mut random_state = 17_u64;
        let mut attacks = Vec::new();
        let mut next_time = 96.0 + FIRST_ATTACK;
        while next_time < 128.0 {
            attacks.push(next_time + next_noise(&mut random_state) * 0.012);
            next_time += 60.0 / next_bpm;
        }

        let mut attack_index = 0;
        let report = analyze_generated(128, |time| {
            if time < 96.0 {
                return (periodic_attack(time, first_bpm, 900.0, 110.0, 0.035) * 0.6) as f32;
            }
            while attack_index + 1 < attacks.len() && attacks[attack_index + 1] <= time {
                attack_index += 1;
            }
            (attack(time - attacks[attack_index], 900.0, 110.0, 0.035) * 0.6) as f32
        });

        assert!(
            report.variable,
            "jitter hid a real tempo change: {report:?}"
        );
        assert!(report.bpm.is_none());
        assert!(
            report
                .bpm_at(48.0)
                .is_some_and(|bpm| (bpm - first_bpm).abs() < 1.0)
        );
        assert!(
            report
                .bpm_at(112.0)
                .is_some_and(|bpm| (bpm - next_bpm).abs() < 1.0)
        );
    }
}

/// Several modest but persistent tempo changes must not disappear into a single dominant value.
#[test]
fn repeated_tempo_changes_retain_their_local_measurements() {
    let tempos = [100.0, 108.0, 116.0, 124.0];
    let report = changing_tempo_recording(&tempos);
    assert!(report.variable, "tempo progression erased: {report:?}");

    for (section, expected) in tempos.into_iter().enumerate() {
        let position = (section * SECTION_SECONDS + SECTION_SECONDS / 2) as f64;
        assert!(
            report
                .bpm_at(position)
                .is_some_and(|bpm| (bpm - expected).abs() < 1.0),
            "tempo progression lost {expected} BPM: {report:?}"
        );
    }
}

/// An accelerating pulse cannot be advertised as one reliable constant tempo when local fitting fails.
#[test]
fn continuous_acceleration_is_variable_or_explicitly_unresolved() {
    let duration = 128.0;
    let initial_bpm = 100.0;
    let change_per_second = 40.0 / duration;
    let report = analyze_generated(duration as usize, |time| {
        let elapsed = time - FIRST_ATTACK;
        if elapsed < 0.0 {
            return 0.0;
        }

        let cycles = initial_bpm / 60.0 * elapsed + change_per_second / 120.0 * elapsed * elapsed;
        let local_bpm = initial_bpm + change_per_second * elapsed;
        let pulse_elapsed = cycles.fract() * 60.0 / local_bpm;
        (attack(pulse_elapsed, 900.0, 110.0, 0.035) * 0.6) as f32
    });

    assert!(
        report.variable || report.bpm.is_none(),
        "acceleration was presented as a confident constant: {report:?}"
    );
}

/// Strong neighboring anchors cannot manufacture measured tempo or beats in silence or unrelated noise.
#[test]
fn contextual_anchors_leave_silence_and_noise_unmeasured() {
    for add_noise in [false, true] {
        let mut noise_state = 17_u64;
        let report = analyze_generated(96, |time| {
            if (32.0..64.0).contains(&time) {
                return if add_noise {
                    (next_noise(&mut noise_state) * 0.02) as f32
                } else {
                    0.0
                };
            }

            (periodic_attack(time, 120.0, 900.0, 110.0, 0.035) * 0.6) as f32
        });

        assert!(
            report
                .segments
                .iter()
                .all(|segment| { segment.end_seconds <= 32.0 || segment.start_seconds >= 64.0 }),
            "unmeasured gap acquired a tempo segment: {report:?}"
        );
        assert!(report.segments.iter().all(|segment| {
            segment
                .beats
                .iter()
                .all(|beat| !(32.0..64.0).contains(beat))
        }));
    }
}

/// Spectral and sparse-note recovery publish one bounded interval for a shared weak passage.
#[test]
fn overlapping_context_evidence_keeps_one_valid_timeline() {
    let mut noise_state = 17_u64;
    let report = analyze_generated(96, |time| {
        let harmony = (std::f64::consts::TAU * 220.0 * time).sin() * 0.025
            + (std::f64::consts::TAU * 330.0 * time).sin() * 0.012;
        let noise = next_noise(&mut noise_state) * 0.0005;
        let mut rhythm = periodic_attack(time, 120.0, 900.0, 110.0, 0.035) * 0.6;
        if (32.0..64.0).contains(&time) {
            let local = time - 32.0;
            let group_position = (local - FIRST_ATTACK).rem_euclid(4.0);
            rhythm = if local >= FIRST_ATTACK && group_position < 1.5 {
                periodic_attack(local, 120.0, 660.0, 18.0, 0.12) * 0.08
            } else {
                0.0
            };
        }

        (rhythm + harmony + noise) as f32
    });

    assert!(
        report.segments.len() <= 512,
        "report exceeded its public bound"
    );
    assert!(
        report
            .segments
            .iter()
            .any(|segment| { segment.start_seconds <= 48.0 && segment.end_seconds > 48.0 }),
        "the supported weak passage was lost: {report:?}"
    );

    let mut previous_end = 0.0;
    for segment in &report.segments {
        assert!(segment.start_seconds.is_finite() && segment.end_seconds.is_finite());
        assert!(
            segment.start_seconds >= previous_end,
            "duplicate or overlapping context"
        );
        assert!(segment.end_seconds > segment.start_seconds && segment.end_seconds <= 96.0);
        assert!((40.0..=240.0).contains(&segment.bpm));
        assert!((0.0..=1.0).contains(&segment.confidence));
        assert!(
            segment
                .beat_confidence
                .is_none_or(|value| (0.0..=1.0).contains(&value))
        );

        let mut previous_beat = None;
        for beat in &segment.beats {
            assert!(
                beat.is_finite() && *beat >= segment.start_seconds && *beat < segment.end_seconds
            );
            assert!(previous_beat.is_none_or(|previous| *beat > previous));
            previous_beat = Some(*beat);
        }

        previous_end = segment.end_seconds;
    }
}

/// Alternates weak notes with stronger accents, occasionally weakening an accent without changing pulse.
fn weak_alternate_attacks(time: f64) -> f32 {
    let beat = ((time - FIRST_ATTACK).max(0.0) * 2.0).floor() as usize;
    let gain = if beat.is_multiple_of(2) && beat % 8 != 6 {
        0.4
    } else {
        0.035
    };
    (periodic_attack(time, 120.0, 900.0, 110.0, 0.035) * gain) as f32
}

/// Repeated full-tempo anchors resolve a weak half-tempo interpretation without claiming phase.
#[test]
fn weak_alternate_attacks_use_supported_full_track_tempo() {
    let report = analyze_generated(96, |time| {
        if (32.0..64.0).contains(&time) {
            weak_alternate_attacks(time - 32.0)
        } else {
            (periodic_attack(time, 120.0, 900.0, 110.0, 0.035) * 0.6) as f32
        }
    });
    assert_stable_tempo(&report, 120.0);
    assert_trusted_beats_have_attacks(&report, |_| 120.0);
    let middle = report
        .segments
        .iter()
        .find(|segment| segment.start_seconds <= 48.0 && segment.end_seconds > 48.0)
        .expect("retained measured periodicity in the weak passage");
    assert!(middle.beats.is_empty());
    assert!(middle.beat_confidence.is_none());
}

/// A steady carrier's RMS ripple must not become a new whole-track tempo through contextual evidence.
#[test]
fn stationary_carrier_does_not_bootstrap_track_context() {
    let report = analyze_generated(64, |time| {
        (0.2 * (std::f64::consts::TAU * 440.0 * time).sin()) as f32
    });
    assert!(
        report.bpm.is_none(),
        "carrier ripple became BPM: {report:?}"
    );
    assert!(report.segments.is_empty());
}
