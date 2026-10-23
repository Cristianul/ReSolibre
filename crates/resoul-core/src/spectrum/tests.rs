use super::*;

/// Generate a bin-centered sine with independently retained opposite-phase channels.
fn tone(frames: usize, channels: usize, bin: usize, amplitude: f32) -> Vec<f32> {
    let mut samples = Vec::with_capacity(frames * channels);
    for frame in 0..frames {
        let phase = std::f64::consts::TAU * bin as f64 * frame as f64 / FFT_SIZE as f64;
        let sample = phase.sin() as f32 * amplitude;
        for channel in 0..channels {
            samples.push(if channel % 2 == 0 { sample } else { -sample });
        }
    }
    samples
}

/// Sum bin energies rather than logarithmic values to verify calibrated mean-square power.
fn total_power(frame: &SpectrumFrame) -> f64 {
    frame
        .power_db
        .iter()
        .map(|db| 10_f64.powf(f64::from(*db) / 10.0))
        .sum()
}

/// A full-scale sine has mean-square energy 0.5, independently of channel count or phase.
#[test]
fn tone_frequency_and_power_preserve_opposite_phase_channels() {
    for channels in [1, 2, 6, MAX_CHANNELS] {
        let mut analyzer = SpectrumAnalyzer::new(48000, channels).unwrap();
        let samples = tone(FFT_SIZE, channels, 64, 1.0);
        let mut received = 0;
        analyzer
            .push(&samples, |frame| {
                let peak = frame
                    .power_db
                    .iter()
                    .enumerate()
                    .max_by(|(_, left), (_, right)| left.total_cmp(right))
                    .unwrap()
                    .0;
                assert_eq!(peak, 64);
                assert_eq!(frame.frequency_hz(peak), Some(1500.0));
                assert!((total_power(frame) - 0.5).abs() < 1e-7);
                assert_eq!(
                    (frame.start_frame, frame.end_frame, frame.valid_frames),
                    (0, 2048, 2048)
                );
                received += 1;
            })
            .unwrap();
        assert_eq!(received, 1);
        assert!(analyzer.finish().is_none());
    }
}

/// DC and Nyquist bins count once; a lower-amplitude tone follows squared amplitude.
#[test]
fn normalization_handles_endpoints_and_gain() {
    let signals = [
        vec![1.0; FFT_SIZE],
        (0..FFT_SIZE)
            .map(|index| if index % 2 == 0 { 1.0 } else { -1.0 })
            .collect(),
        tone(FFT_SIZE, 1, 40, 0.5),
    ];
    for (samples, expected) in signals.iter().zip([1.0, 1.0, 0.125]) {
        let mut analyzer = SpectrumAnalyzer::new(44100, 1).unwrap();
        analyzer
            .push(samples, |frame| {
                assert!((total_power(frame) - expected).abs() < 1e-7);
                assert_eq!(frame.frequency_hz(FFT_SIZE / 2), Some(22050.0));
                assert_eq!(frame.frequency_hz(BIN_COUNT), None);
            })
            .unwrap();
    }
}

/// Chunk boundaries cannot change the FFT output, hop timing or end-of-file partial window.
#[test]
fn streaming_matches_large_blocks_and_marks_partial_tail() {
    let samples = tone(FFT_SIZE * 3 + 137, 2, 23, 0.7);
    let mut reference = SpectrumAnalyzer::new(48000, 2).unwrap();
    let mut expected = Vec::new();
    reference
        .push(&samples, |frame| {
            expected.push((frame.end_frame, frame.power_db))
        })
        .unwrap();
    let mut streamed = SpectrumAnalyzer::new(48000, 2).unwrap();
    let mut actual = Vec::new();
    for block in samples.chunks(37 * 2) {
        streamed
            .push(block, |frame| {
                actual.push((frame.end_frame, frame.power_db))
            })
            .unwrap();
    }
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 5);
    for (index, (end, _)) in actual.iter().enumerate() {
        assert_eq!(*end, (FFT_SIZE + HOP_SIZE * index) as u64);
    }
    let final_reference = reference.finish().unwrap();
    let final_streamed = streamed.finish().unwrap();
    assert_eq!(final_streamed.power_db, final_reference.power_db);
    assert_eq!(final_streamed.end_frame, (FFT_SIZE * 3 + 137) as u64);
    assert_eq!(final_streamed.valid_frames, HOP_SIZE + 137);
    assert!(streamed.finish().is_none());
}

/// Invalid blocks are rejected before partial history or timeline state can change.
#[test]
fn rejects_untrusted_input_atomically() {
    assert!(matches!(
        SpectrumAnalyzer::new(0, 2),
        Err(AnalysisError::InvalidSampleRate)
    ));
    assert!(matches!(
        SpectrumAnalyzer::new(u32::MAX, 2),
        Err(AnalysisError::InvalidSampleRate)
    ));
    assert!(matches!(
        SpectrumAnalyzer::new(48000, 0),
        Err(AnalysisError::InvalidChannelCount)
    ));
    assert!(matches!(
        SpectrumAnalyzer::new(48000, MAX_CHANNELS + 1),
        Err(AnalysisError::InvalidChannelCount)
    ));
    let mut analyzer = SpectrumAnalyzer::new(48000, 2).unwrap();
    let first = tone(100, 2, 13, 0.3);
    analyzer
        .push(&first, |_| panic!("incomplete window"))
        .unwrap();
    let history = analyzer.history.clone();
    for invalid in [
        vec![f32::NAN, 0.0],
        vec![0.0, f32::INFINITY],
        vec![f32::NEG_INFINITY, 0.0],
    ] {
        assert_eq!(
            analyzer.push(&invalid, |_| panic!("invalid block")),
            Err(AnalysisError::NonFiniteSample)
        );
    }
    assert_eq!(
        analyzer.push(&[0.0], |_| {}),
        Err(AnalysisError::IncompleteFrame)
    );
    assert_eq!(
        analyzer.push(&vec![0.0; MAX_INPUT_FRAMES * 2 + 2], |_| {}),
        Err(AnalysisError::InputTooLarge)
    );
    assert_eq!(analyzer.total_frames, 100);
    assert_eq!(analyzer.history, history);
    analyzer.total_frames = u64::MAX;
    assert_eq!(
        analyzer.push(&[0.0, 0.0], |_| {}),
        Err(AnalysisError::PositionOverflow)
    );
}

/// Seek/reset discards old samples, and zero-padded short tracks retain their actual duration.
#[test]
fn reset_silence_and_short_tracks_have_no_stale_energy() {
    let mut analyzer = SpectrumAnalyzer::new(48000, 1).unwrap();
    analyzer.push(&tone(1500, 1, 13, 0.4), |_| {}).unwrap();
    analyzer.reset();
    analyzer
        .push(&[0.0; 100], |_| panic!("short window"))
        .unwrap();
    let frame = analyzer.finish().unwrap();
    assert_eq!(
        (frame.start_frame, frame.end_frame, frame.valid_frames),
        (0, 100, 100)
    );
    assert!(frame.power_db.iter().all(|db| *db == FLOOR_DB));
    assert!(analyzer.finish().is_none());
    analyzer.reset();
    analyzer
        .push(&[0.0; FFT_SIZE], |frame| {
            assert_eq!(frame.end_frame, FFT_SIZE as u64);
            assert!(frame.power_db.iter().all(|db| *db == FLOOR_DB));
        })
        .unwrap();
}

/// Finite Float32 extremes cannot overflow the f64 transform into invalid public results.
#[test]
fn finite_extreme_samples_remain_finite() {
    let mut analyzer = SpectrumAnalyzer::new(48000, 1).unwrap();
    analyzer
        .push(&vec![f32::MAX; FFT_SIZE], |frame| {
            assert!(frame.power_db.iter().all(|db| db.is_finite()));
            assert!(frame.power_db[0] > 0.0);
        })
        .unwrap();
}
