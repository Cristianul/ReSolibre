use super::*;

/// Activity uses original per-channel peaks, ignores quiet edges and isolated clicks, and records excerpt coverage.
#[test]
fn mix_activity_is_independent_of_beats_and_channel_phase() {
    let mut analyzer = TempoAnalyzer::new(8000, 2);
    analyzer.push(&vec![0.002; 2 * 8000 * 2]);
    analyzer.push(&[0.5, -0.5]);
    analyzer.push(&vec![0.0; 7999 * 2]);
    let mut active = Vec::new();
    for frame in 0..4 * 8000 {
        let sample = (std::f64::consts::TAU * 440.0 * frame as f64 / 8000.0).sin() as f32 * 0.1;
        active.extend_from_slice(&[sample, -sample]);
    }
    analyzer.push(&active);
    analyzer.push(&vec![0.002; 2 * 8000 * 2]);
    let report = analyzer.finish();
    assert!(report.bpm.is_none());
    assert_eq!(report.activity.analyzed_seconds, 9.0);
    // A stray click may conservatively retain an edge; it must never erase subsequent music.
    assert_eq!(report.activity.first_sound, Some(2.0));
    assert_eq!(report.activity.last_sound, Some(7.0));
    assert!(report.activity.is_valid());
    let mut click = TempoAnalyzer::new(8000, 1);
    click.push(&[0.5]);
    click.push(&vec![0.0; 7999]);
    assert!(click.finish().activity.bounds().is_none());
    let mut silence = TempoAnalyzer::new(8000, 1);
    silence.push(&vec![0.002; 8000]);
    assert!(silence.finish().activity.bounds().is_none());
}

/// Generates a decaying percussion pulse with an independently specified tempo and channel polarity.
fn pulses(bpm: f64, seconds: usize, channels: usize, rate: u32) -> Vec<f32> {
    let period = 60.0 / bpm;
    (0..seconds * rate as usize)
        .flat_map(|frame| {
            let time = frame as f64 / f64::from(rate);
            let position = (time - 0.25).rem_euclid(period);
            let value = if time >= 0.25 && position < 0.035 {
                ((std::f64::consts::TAU * 900.0 * position).sin() * (-position * 110.0).exp() * 0.6)
                    as f32
            } else {
                0.0
            };
            (0..channels).map(move |channel| if channel % 2 == 0 { value } else { -value })
        })
        .collect()
}

/// Known tempos, arbitrary decoder boundaries and opposite-phase channels preserve tempo and measured beat locations.
#[test]
fn tempo_and_beats_follow_generated_percussion() {
    for bpm in [
        40.0, 60.0, 90.0, 113.0, 120.0, 128.0, 140.0, 174.0, 220.0, 240.0,
    ] {
        let mut analyzer = TempoAnalyzer::new(8000, 2);
        let samples = pulses(bpm, 32, 2, 8000);
        for block in samples.chunks(514) {
            analyzer.push(block);
        }
        let report = analyzer.finish();
        let observed = report
            .bpm
            .unwrap_or_else(|| panic!("missing BPM {bpm}: {report:?}"));
        assert!(
            (observed - bpm).abs() < 1.0,
            "expected {bpm}, got {report:?}"
        );
        for segment in report.segments {
            assert!(segment.confidence >= 0.45);
            for beat in segment.beats {
                let grid = ((beat - 0.25) * bpm / 60.0).round() * 60.0 / bpm + 0.25;
                assert!(
                    (beat - grid).abs() < 0.025,
                    "beat error at {bpm}: {beat} / {grid}"
                );
            }
        }
    }
}

/// Silence, steady tones and short excerpts do not become fabricated tempo; differing sections remain variable.
#[test]
fn unknown_and_variable_tempo_are_explicit() {
    for samples in [
        vec![0.0; 32 * 8000],
        vec![0.25; 32 * 8000],
        pulses(120.0, 4, 1, 8000),
    ] {
        let mut analyzer = TempoAnalyzer::new(8000, 1);
        analyzer.push(&samples);
        assert_eq!(analyzer.finish().bpm, None);
    }
    let mut analyzer = TempoAnalyzer::new(8000, 1);
    analyzer.push(&pulses(100.0, 32, 1, 8000));
    analyzer.push(&pulses(140.0, 32, 1, 8000));
    let report = analyzer.finish();
    assert!(report.variable, "{report:?}");
    assert!(report.bpm.is_none());
    assert_eq!(report.segments.len(), 2);
}

/// Bounded time aggregation preserves a moving frequency and an honest duration/Nyquist axis.
#[test]
fn spectrogram_preserves_frequency_time_and_channel_power() {
    let mut analyzer = TrackAnalyzer::new(8000, 2).unwrap();
    for block in 0..300 {
        let samples: Vec<_> = (0..4096)
            .flat_map(|index| {
                let frame = block * 4096 + index;
                let frequency = if block < 150 { 500.0 } else { 2000.0 };
                let sample =
                    (std::f64::consts::TAU * frequency * frame as f64 / 8000.0).sin() as f32 * 0.5;
                [sample, -sample]
            })
            .collect();
        analyzer.push(&samples).unwrap();
    }
    let result = analyzer.finish().unwrap();
    assert!(result.columns <= MAX_COLUMNS);
    assert_eq!(result.power_db.len(), result.columns * ROWS);
    assert!((result.duration_seconds - 153.6).abs() < 0.001);
    let peak_row = |column: usize| {
        result.power_db[column * ROWS..(column + 1) * ROWS]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0
    };
    assert!((peak_row(10) as i32 - 32).abs() <= 1);
    assert!((peak_row(result.columns - 10) as i32 - 128).abs() <= 1);
    assert!((result.sustained_bandwidth_hz.unwrap() - 2000.0).abs() < 20.0);
}

/// Malformed/oversized input is rejected without disturbing a valid report; silence remains inconclusive.
#[test]
fn malformed_and_silent_analysis() {
    let mut analyzer = TrackAnalyzer::new(8000, 2).unwrap();
    assert_eq!(analyzer.push(&[0.0]), Err(AnalysisError::IncompleteFrame));
    assert_eq!(
        analyzer.push(&[f32::NAN, 0.0]),
        Err(AnalysisError::NonFiniteSample)
    );
    assert_eq!(
        analyzer.push(&vec![0.0; 32770]),
        Err(AnalysisError::InputTooLarge)
    );
    assert_eq!(analyzer.decoded_seconds(), 0.0);
    analyzer.frames = 8000 * MAX_SECONDS;
    assert_eq!(
        analyzer.push(&[0.0, 0.0]),
        Err(AnalysisError::InputTooLarge)
    );
    analyzer.frames = 0;
    analyzer.push(&[0.0; 8192]).unwrap();
    let report = analyzer.finish().unwrap();
    assert_eq!(report.sustained_bandwidth_hz, None);
    assert_eq!(report.tempo.bpm, None);
    assert!(report.power_db.iter().all(|value| *value == -120.0));
}

/// Continuous noise and weak subdivisions cannot force a fast tempo; gain and stereo phase do not change the pulse.
#[test]
fn noise_gain_and_subdivision_regressions() {
    let mut state = 17_u64;
    let noise: Vec<f32> = (0..32 * 8000)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((state >> 32) as u32 as f64 / f64::from(u32::MAX) - 0.5) as f32 * 0.2
        })
        .collect();
    let mut analyzer = TempoAnalyzer::new(8000, 1);
    analyzer.push(&noise);
    assert!(analyzer.finish().bpm.is_none());
    let main = pulses(120.0, 32, 1, 8000);
    let subdivisions = pulses(240.0, 32, 1, 8000);
    for gain in [0.01, 0.25, 1.0] {
        let mut analyzer = TempoAnalyzer::new(8000, 1);
        let samples: Vec<_> = main
            .iter()
            .zip(&subdivisions)
            .map(|(main, sub)| (main + sub * 0.1) * gain)
            .collect();
        analyzer.push(&samples);
        let report = analyzer.finish();
        assert!((report.bpm.unwrap() - 120.0).abs() < 0.5, "{report:?}");
    }
}

/// Silence and an isolated high-frequency burst do not imply sustained wide-band content.
#[test]
fn bandwidth_ignores_isolated_spikes() {
    let mut analyzer = TrackAnalyzer::new(8000, 1).unwrap();
    for second in 0..12 {
        let samples: Vec<_> = (0..8000)
            .map(|index| {
                let time = (second * 8000 + index) as f64 / 8000.0;
                let tone = (std::f64::consts::TAU * 500.0 * time).sin() * 0.5;
                let spike = if second == 6 && index < 100 {
                    (std::f64::consts::TAU * 3500.0 * time).sin() * 0.5
                } else {
                    0.0
                };
                (tone + spike) as f32
            })
            .collect();
        analyzer.push(&samples).unwrap();
    }
    let report = analyzer.finish().unwrap();
    assert!((report.sustained_bandwidth_hz.unwrap() - 500.0).abs() < 20.0);
}

/// Mixed sustained tones, subdivisions and unequal attacks retain tempo without requiring isolated clicks.
#[test]
fn rhythm_only_handles_layered_audio_and_rejects_invalid_blocks() {
    use super::rhythm::RhythmAnalyzer;
    assert!(RhythmAnalyzer::new(0, 2).is_err());
    assert!(RhythmAnalyzer::new(44100, 0).is_err());
    for bpm in [113.0, 140.0, 174.0] {
        let mut analyzer = RhythmAnalyzer::new(8000, 2).unwrap();
        assert_eq!(analyzer.push(&[0.0]), Err(AnalysisError::IncompleteFrame));
        assert_eq!(
            analyzer.push(&[f32::NAN, 0.0]),
            Err(AnalysisError::NonFiniteSample)
        );
        assert_eq!(
            analyzer.push(&vec![0.0; 32770]),
            Err(AnalysisError::InputTooLarge)
        );
        let main = pulses(bpm, 64, 1, 8000);
        let subdivisions = pulses(bpm * 2.0, 64, 1, 8000);
        for block in 0..64 {
            let samples: Vec<_> = (block * 8000..(block + 1) * 8000)
                .flat_map(|frame| {
                    let time = frame as f64 / 8000.0;
                    let harmony = (std::f64::consts::TAU * 220.0 * time).sin() * 0.08
                        + (std::f64::consts::TAU * 330.0 * time).sin() * 0.04;
                    let sample = main[frame] + subdivisions[frame] * 0.1 + harmony as f32;
                    [sample, -sample]
                })
                .collect();
            analyzer.push(&samples).unwrap();
        }
        let report = analyzer.finish();
        assert!(
            report.bpm.is_some_and(|value| (value - bpm).abs() < 1.0),
            "expected {bpm}, got {:?}",
            report.bpm
        );
        assert!(
            report
                .segments
                .iter()
                .all(|segment| segment.beat_confidence.is_some() == !segment.beats.is_empty())
        );
    }
}

/// Measured local tempo follows seek position; silence and invalid time never inherit a neighbouring estimate.
#[test]
fn local_bpm_follows_variable_sections_and_preserves_unknown_gaps() {
    let mut analyzer = TempoAnalyzer::new(8000, 1);
    analyzer.push(&pulses(120.0, 32, 1, 8000));
    analyzer.push(&vec![0.0; 32 * 8000]);
    analyzer.push(&pulses(140.0, 32, 1, 8000));
    let report = analyzer.finish();
    assert!(report.variable);
    let (minimum, maximum) = report.bpm_range().unwrap();
    assert!((minimum - 120.0).abs() < 1.0);
    assert!((maximum - 140.0).abs() < 1.0);
    assert!((report.bpm_at(12.0).unwrap() - 120.0).abs() < 1.0);
    assert!((report.bpm_at(80.0).unwrap() - 140.0).abs() < 1.0);
    for position in [32.0, 50.0, 96.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(report.bpm_at(position), None);
    }
    let stable = tempo::TempoReport {
        bpm: Some(120.0),
        ..Default::default()
    };
    assert_eq!(stable.bpm_at(90.0), Some(120.0));
    assert_eq!(stable.bpm_range(), Some((120.0, 120.0)));
    assert_eq!(tempo::TempoReport::default().bpm_range(), None);
    assert_eq!(tempo::TempoReport::default().bpm_at(0.0), None);
    let mut invalid = report.clone();
    invalid.segments[0].bpm = f64::NAN;
    let (minimum, maximum) = invalid.bpm_range().unwrap();
    assert_eq!(minimum, maximum);
    assert!((minimum - 140.0).abs() < 1.0);
    invalid.segments[1].end_seconds = f64::INFINITY;
    assert_eq!(invalid.bpm_range(), None);
}

/// Sparse piano-like notes stay on the established pulse even when their irregular spacing has no independent tempo.
#[test]
fn context_recovers_sparse_notes_without_filling_silence_or_unrelated_attacks() {
    let notes = [1.0, 3.5, 8.0, 11.0, 17.5, 23.0, 28.5];
    let sparse = sparse_notes(&notes, 0.25);
    let mut alone = TempoAnalyzer::new(8000, 1);
    alone.push(&sparse);
    assert!(
        alone.finish().segments.is_empty(),
        "fixture needs contextual tempo"
    );

    let mut contextual = TempoAnalyzer::new(8000, 1);
    contextual.push(&pulses(120.0, 32, 1, 8000));
    contextual.push(&sparse);
    contextual.push(&pulses(120.0, 32, 1, 8000));
    let report = contextual.finish();
    assert!(!report.variable);
    let middle = report
        .segments
        .iter()
        .find(|segment| segment.start_seconds == 32.0)
        .expect("recover the sparse passage using established tempo and phase");
    assert!((middle.bpm - 120.0).abs() < 1.0);
    assert!(middle.confidence <= 0.5);
    assert!(
        middle.beat_confidence.unwrap() < 0.55,
        "context cannot authorize native synchronization"
    );
    assert!(middle.beats.len() >= 4 && middle.beats.len() <= notes.len());
    for beat in &middle.beats {
        assert!(
            notes.iter().any(|note| (beat - 32.25 - note).abs() < 0.06),
            "never fill unobserved beats"
        );
    }

    let mut intro = TempoAnalyzer::new(8000, 1);
    intro.push(&sparse);
    intro.push(&pulses(120.0, 32, 1, 8000));
    assert_eq!(
        intro.finish().segments[0].start_seconds,
        0.0,
        "later evidence can recover a sparse intro"
    );

    let mut variable = TempoAnalyzer::new(8000, 1);
    variable.push(&pulses(120.0, 32, 1, 8000));
    variable.push(&sparse);
    variable.push(&pulses(140.0, 32, 1, 8000));
    let variable = variable.finish();
    assert!(variable.variable);
    assert!(
        !variable
            .segments
            .iter()
            .any(|segment| segment.start_seconds == 32.0),
        "conflicting tempo must not force a global prior"
    );

    let mut state = 17_u64;
    let noise: Vec<f32> = (0..32 * 8000)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((state >> 32) as u32 as f64 / f64::from(u32::MAX) - 0.5) as f32 * 0.02
        })
        .collect();
    for (kind, middle) in [("silence", vec![0.0; 32 * 8000]), ("noise", noise)] {
        let mut analyzer = TempoAnalyzer::new(8000, 1);
        analyzer.push(&pulses(120.0, 32, 1, 8000));
        analyzer.push(&middle);
        analyzer.push(&pulses(120.0, 32, 1, 8000));
        let report = analyzer.finish();
        assert!(
            !report
                .segments
                .iter()
                .any(|segment| segment.start_seconds == 32.0),
            "{kind}: {report:?}"
        );
    }
}

/// Consistently shifted melodic notes can confirm tempo while remaining unusable as aligned beats.
#[test]
fn contextual_offbeat_notes_do_not_claim_the_percussion_phase() {
    let notes = [1.0, 3.5, 8.0, 11.0, 17.5, 23.0, 28.5];
    let mut analyzer = TempoAnalyzer::new(8000, 1);
    analyzer.push(&pulses(120.0, 32, 1, 8000));
    analyzer.push(&sparse_notes(&notes, 0.0));
    analyzer.push(&pulses(120.0, 32, 1, 8000));
    let report = analyzer.finish();
    let middle = report
        .segments
        .iter()
        .find(|segment| segment.start_seconds == 32.0)
        .expect("melodic timing supports tempo despite its different phase");
    assert!((middle.bpm - 120.0).abs() < 1.0);
    assert!(middle.beats.is_empty());
    assert!(middle.beat_confidence.is_none());
    assert!(middle.subdivision_ambiguous);
    assert!(middle.confidence <= 0.5);
}

/// Generates owned soft, decaying pitched attacks at chosen source times, without a percussion track.
fn sparse_notes(notes: &[f64], offset: f64) -> Vec<f32> {
    let mut samples = vec![0.0; 32 * 8000];
    for (index, note) in notes.iter().enumerate() {
        let start = ((note + offset) * 8000.0) as usize;
        for frame in 0..4000 {
            let time = frame as f64 / 8000.0;
            let attack = (time / 0.01).min(1.0);
            let pitch = 440.0 + index as f64 * 55.0;
            samples[start + frame] = (0.04
                * attack
                * (-time * 9.0).exp()
                * (std::f64::consts::TAU * pitch * time).sin())
                as f32;
        }
    }
    samples
}
