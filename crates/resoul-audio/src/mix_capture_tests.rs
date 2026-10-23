//! Owned two-channel rhythm fixture for tools/check_mix.py; no desktop audio access.

use crate::{
    AudioOutput,
    session::{Command, MixStatus, Session},
};
use resolibre_core::session::Volume;
use std::time::{Duration, Instant};

/// Captures bass/treble through the production midpoint and return curves on a private output.
#[test]
#[ignore = "run through tools/check_mix_pitch.py"]
fn capture_pitch_return() -> Result<(), crate::PlaybackError> {
    use crate::LocalPlayer;
    use resolibre_core::{
        PlaybackEvent,
        equalizer::Equalizer,
        transition::{BeatRatio, TransitionPlan},
    };

    let (current, next) = match std::env::var("RESOUL_TEST_PITCH_DIRECTION").as_deref() {
        Ok("up") => (100.0, 120.0),
        Ok("down") => (120.0, 100.0),
        _ => panic!("fixed pitch fixture direction required"),
    };
    let outgoing = match std::env::var("RESOUL_TEST_PITCH_STREAM").as_deref() {
        Ok("outgoing") => true,
        Ok("incoming") | Err(_) => false,
        _ => panic!("fixed pitch fixture stream required"),
    };
    let plan = TransitionPlan {
        outgoing_start: if outgoing { 8.0 } else { 0.0 },
        outgoing_end: if outgoing { 20.0 } else { 12.0 },
        incoming_start: 0.0,
        outgoing_bpm: Some(current),
        incoming_bpm: Some(next),
        phase_matched: false,
        beat_ratio: BeatRatio::OneToOne,
    };
    let curve = plan.tempo_return().unwrap();
    let settings = Equalizer::default();
    let mut player = LocalPlayer::open_with_equalizer(
        std::path::Path::new("private-input.wav"),
        AudioOutput::PrivatePulse,
        true,
        Volume::from_percent(100.0).unwrap(),
        settings,
    )?;
    let deadline = Instant::now() + Duration::from_secs(25);
    while !player.prepare_equalizer(settings)? {
        assert!(Instant::now() < deadline, "pitch setup deadline");
        assert_ne!(player.poll_event()?, Some(PlaybackEvent::Ended));
    }
    for invalid in [f64::NAN, f64::INFINITY, -1.0, 0.79, 1.21] {
        assert!(matches!(
            player.set_mix_speed(invalid),
            Err(crate::PlaybackError::InvalidEvent)
        ));
    }
    let initial_speed = current / next;
    let initial_frame = plan.frame(0.0);
    player.set_mix_speed(if outgoing {
        initial_frame.outgoing_speed
    } else {
        initial_frame.incoming_speed
    })?;
    player.set_paused(false)?;
    loop {
        assert!(Instant::now() < deadline, "pitch return deadline");
        assert_ne!(player.try_event()?, Some(PlaybackEvent::Ended));
        if let Some(position) = player.audio_position()? {
            let outgoing_position = position / initial_speed;
            let speed = if outgoing {
                plan.frame(position).outgoing_speed
            } else if outgoing_position < plan.outgoing_end {
                plan.frame(outgoing_position).incoming_speed
            } else {
                curve.frame(position).unwrap().speed
            };
            player.set_mix_speed(speed)?;
            let finished = if outgoing {
                position > plan.outgoing_end
            } else {
                position > curve.end_position() + 1.5
            };
            if finished {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("pitch-return-complete");
    Ok(())
}

/// Exercises production mixing; the parent links both streams and checks independently captured beat times.
#[test]
#[ignore = "run through tools/check_mix.py"]
fn capture_mix() {
    let cache = std::env::current_dir().unwrap().join("analysis-cache");
    // Initialize the owned fixture database before concurrent analysis jobs open it.
    drop(resolibre_storage::AnalysisCache::open(&cache).unwrap());
    let missing_clock = std::env::var_os("RESOUL_TEST_NO_AUDIO_CLOCK").is_some();
    let invalid_clock = std::env::var_os("RESOUL_TEST_INVALID_AUDIO_CLOCK").is_some();
    let output = if invalid_clock {
        AudioOutput::PrivatePulseInvalidClock
    } else if missing_clock {
        AudioOutput::PrivatePulseWithoutClock
    } else if std::env::var_os("RESOUL_TEST_MIX_PIPEWIRE").is_some() {
        AudioOutput::PrivatePipeWire
    } else {
        AudioOutput::PrivatePulse
    };
    let expected_ratio = match std::env::var("RESOUL_TEST_MIX_RATIO").as_deref() {
        Ok("1:2") => (1, 2),
        Ok("2:1") => (2, 1),
        Ok("1:3") => (1, 3),
        Ok("3:1") => (3, 1),
        Ok("2:3") => (2, 3),
        Ok("3:2") => (3, 2),
        Ok("1:1") | Err(_) => (1, 1),
        _ => panic!("invalid fixture ratio"),
    };
    let tempo_only = std::env::var_os("RESOUL_TEST_MIX_TEMPO_ONLY").is_some();
    if tempo_only {
        cache_without_phase(&cache);
    }
    let session = Session::start_with_analysis_cache(output, Some(cache)).unwrap();
    session
        .submit(Command::Volume(Volume::from_percent(100.0).unwrap()))
        .unwrap();
    session.submit(Command::SetMix(true)).unwrap();
    let no_trim = std::env::var_os("RESOUL_TEST_MIX_NO_TRIM").is_some();
    if !no_trim {
        session
            .submit(Command::Edges(
                resolibre_core::playback_edges::EdgeAction::ToggleTrim,
            ))
            .unwrap();
    }
    session
        .submit(Command::Open(vec![
            "outgoing.wav".into(),
            "incoming.wav".into(),
        ]))
        .unwrap();
    let long = std::env::var_os("RESOUL_TEST_MIX_LONG").is_some();
    let deadline = Instant::now() + Duration::from_secs(if long { 70 } else { 40 });
    let mut jumped = false;
    let mut synced = 0;
    let mut matched_tempo = 0;
    let mut overlapped = false;
    let mut adopted = None;
    let mut ready_printed = false;
    let mut minimum_handover = 0.0;
    loop {
        assert!(Instant::now() < deadline, "mix capture deadline");
        let snapshot = session.snapshot();
        if let Some(error) = &snapshot.error {
            eprintln!("mix-error: {error}");
        }
        assert!(snapshot.error.is_none(), "native mix failed");
        if long && !jumped && snapshot.seekable {
            session
                .submit(Command::Seek {
                    generation: snapshot.generation,
                    seconds: 190.0,
                })
                .unwrap();
            jumped = true;
        }
        if snapshot.current == Some(0) {
            assert_ne!(
                snapshot.mix_status,
                MixStatus::Unavailable,
                "optional mix failed"
            );
        }
        if matches!(
            snapshot.mix_status,
            MixStatus::Ready | MixStatus::ReadyCrossfade
        ) && !ready_printed
        {
            if long {
                assert!(
                    snapshot.position.unwrap() < 204.0,
                    "long MIX must prepare before the old 36-second window"
                );
                let region = snapshot.mix_region.expect("prepared long overlap");
                let rounding = if expected_ratio == (1, 1) {
                    0.5
                } else {
                    let crate::session::RhythmStatus::Ready(report) = &snapshot.rhythm else {
                        panic!("fixture tempo unavailable");
                    };
                    0.5 + expected_ratio.0 as f64 * 60.0 / report.bpm.unwrap()
                };
                assert!(
                    region.duration() <= 24.5 && region.duration() > 24.0 - rounding,
                    "long overlap must use ten percent within one shared cycle"
                );
            }
            if no_trim {
                assert!(!snapshot.edges.trim, "MIX must not enable ordinary TRIM");
                assert!(snapshot.trim_end.is_none());
                let region = snapshot.mix_region.expect("prepared MIX bounds");
                assert!(
                    region.end < if long { 239.0 } else { 23.0 }
                        && region.start < if long { 215.0 } else { 20.0 },
                    "MIX must remove the quiet tail"
                );
            }
            let region = snapshot.mix_region.expect("prepared overlap");
            // Incoming source seconds differ from outgoing seconds when BPM is matched.
            minimum_handover =
                region.duration() / (1.0 + resolibre_core::transition::MAX_TEMPO_CHANGE) - 0.1;
            println!("mix-ready");
            ready_printed = true;
        }
        if snapshot.current == Some(0) && snapshot.mix_status == MixStatus::TempoMixing {
            require_fixture_ratio(&snapshot, expected_ratio);
            matched_tempo += 1;
        }
        if snapshot.current == Some(0) && snapshot.mix_status == MixStatus::BeatMixing {
            require_fixture_ratio(&snapshot, expected_ratio);
            synced += 1;
        }
        overlapped |= snapshot.current == Some(0)
            && matches!(
                snapshot.mix_status,
                MixStatus::BeatMixing | MixStatus::TempoMixing | MixStatus::Crossfading
            );
        if snapshot.current == Some(1) {
            assert!(overlapped, "READY must reach overlap before natural EOF");
            assert!(
                snapshot
                    .position
                    .is_some_and(|position| position > minimum_handover),
                "handover must retain the incoming position"
            );
            let start = adopted.get_or_insert_with(Instant::now);
            // Observe the entire return plus several ordinary beats after promotion.
            let tail_seconds = resolibre_core::transition::TEMPO_RETURN_DURATION_SECONDS + 3.0;
            if start.elapsed().as_secs_f64() >= tail_seconds {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if tempo_only {
        assert!(
            matched_tempo > 10,
            "measured BPM never controlled the overlap"
        );
    }
    if missing_clock || no_trim || tempo_only {
        assert_eq!(synced, 0, "fallback cannot claim verified synchronization");
    } else {
        assert!(synced > 10, "beat mixing was never observed");
    }
    session.request_shutdown();
    let mut session = session;
    while !session.finish().unwrap() {
        assert!(Instant::now() < deadline, "mix cleanup deadline");
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("mix-complete");
}

/// Saves genuine fixture tempo with unavailable phase, exercising the production cache and rate-only path.
fn cache_without_phase(directory: &std::path::Path) {
    use crate::{original_audio::SourceKey, rhythm_analysis::RhythmJob};
    use resolibre_storage::{AnalysisCache, audio_identity};
    use std::{fs::File, sync::atomic::AtomicBool};

    let mut database = AnalysisCache::open(directory).unwrap();
    for name in ["outgoing.wav", "incoming.wav"] {
        let file = File::open(name).unwrap();
        let key = SourceKey::read(&file, u32::MAX).unwrap();
        let mut job = RhythmJob::start(file, key, true, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut report = loop {
            if let Some(result) = job.poll() {
                break result.unwrap().1;
            }
            assert!(Instant::now() < deadline, "fixture rhythm deadline");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(
            report.bpm.is_some(),
            "fixture must independently establish tempo"
        );
        for segment in &mut report.segments {
            segment.beats.clear();
            segment.beat_confidence = None;
        }
        let identity =
            audio_identity::identify(File::open(name).unwrap(), &AtomicBool::new(false)).unwrap();
        database.save_rhythm(&identity, true, &report).unwrap();
    }
}

/// Checks the published ratio and reports only owned-fixture tempo numbers on failure.
fn require_fixture_ratio(snapshot: &crate::session::Snapshot, expected: (usize, usize)) {
    let actual = snapshot.mix_beat_ratio.unwrap();
    if actual.counts() == expected {
        return;
    }
    let current = match &snapshot.rhythm {
        crate::session::RhythmStatus::Ready(report) => report.bpm,
        _ => None,
    };
    let opening = snapshot
        .next_rhythm
        .as_ref()
        .and_then(|next| next.report.segments.first())
        .map(|segment| segment.bpm);
    panic!(
        "fixture ratio={actual:?}, expected={expected:?}, current BPM={current:?}, opening BPM={opening:?}"
    );
}
