//! Real decoder/session regressions use generated private files and silent output only.
use resolibre_audio::{
    AudioOutput,
    session::{Command, EdgeStatus, MixStatus, Session, Snapshot},
};
use resolibre_core::{
    playback_edges::{EdgeAction, FadeMode, PlaybackEdges},
    session::{ScanDirection, SessionState},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

/// Creates stereo antiphase audio with faint edge noise and an intentional interior pause.
fn fixture() -> (PathBuf, Vec<u8>) {
    let directory = std::env::temp_dir().join(format!(
        "resolibre-edges-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("private.wav");
    let rate = 16000_u32;
    let frames = rate * 4;
    let data_bytes = frames * 4;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data_bytes).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 4).to_le_bytes());
    bytes.extend(4_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data_bytes.to_le_bytes());
    for frame in 0..frames {
        let time = f64::from(frame) / f64::from(rate);
        let active = (0.8..1.8).contains(&time) || (2.2..3.2).contains(&time);
        let sample = if active {
            (6000.0 * (time * 440.0 * std::f64::consts::TAU).sin()) as i16
        } else if frame % 2 == 0 {
            49
        } else {
            -49
        };
        bytes.extend(sample.to_le_bytes());
        bytes.extend((-sample).to_le_bytes());
    }
    fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

/// Bounds every asynchronous observation and reports only safe session state on failure.
#[track_caller]
fn wait(session: &Session, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(18);
    loop {
        let snapshot = session.snapshot();
        assert_ne!(snapshot.state, SessionState::Failed, "{:?}", snapshot.error);
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "wait at {}: state={:?}, processing={:?}, position={:?}, scan={:?}",
            std::panic::Location::caller(),
            snapshot.state,
            snapshot.edge_status,
            snapshot.position,
            snapshot.scanning
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Cold TRIM starts with the original audio clock, then moves to the measured opening edge.
#[test]
fn cold_trim_plays_while_analysis_runs_and_updates_position() {
    let (path, original) = fixture();
    let session = Session::start(AudioOutput::Silent).unwrap();
    session
        .submit(Command::Edges(EdgeAction::ToggleTrim))
        .unwrap();
    session.submit(Command::SetMix(true)).unwrap();
    session
        .submit(Command::Open(vec![path.clone(), path.clone()]))
        .unwrap();

    let pending = wait(&session, |snapshot| {
        snapshot.state == SessionState::Playing
            && snapshot.edge_status == EdgeStatus::Pending
            && snapshot.seekable
    });
    assert_eq!(pending.trim_start, 0.0);
    assert_eq!(pending.mix_status, MixStatus::Preparing);
    assert!(pending.position.is_some_and(|position| position < 0.75));

    session.submit(Command::TogglePause).unwrap();
    let ready = wait(&session, |snapshot| {
        snapshot.state == SessionState::Paused
            && snapshot.edge_status == EdgeStatus::Ready
            && snapshot.seekable
            && snapshot.position.is_some_and(|position| position >= 0.74)
    });
    assert!((ready.trim_start - 0.75).abs() < 0.005);
    assert_eq!(ready.generation, pending.generation);

    session.request_shutdown();
    drop(session);
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

/// Trim is shared by seek, scan and queue EOF; toggles preserve source time and interior silence.
#[test]
fn trim_fade_session_respects_original_timeline_and_queue() {
    let (path, original) = fixture();
    let modified = path.metadata().unwrap().modified().unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    session
        .submit(Command::Edges(EdgeAction::Set(PlaybackEdges {
            trim: true,
            fade: FadeMode::Both,
        })))
        .unwrap();
    session
        .submit(Command::Open(vec![path.clone(), path.clone()]))
        .unwrap();
    let playing = wait(&session, |s| {
        s.state == SessionState::Playing && s.edge_status == EdgeStatus::Ready && s.seekable
    });
    assert!((playing.trim_start - 0.75).abs() < 0.005);
    assert!((playing.trim_end.unwrap() - 3.25).abs() < 0.005);
    assert!((playing.duration.unwrap() - 4.0).abs() < 0.001);
    assert!(playing.position.unwrap() >= 0.74);
    session.submit(Command::TogglePause).unwrap();
    wait(&session, |s| s.state == SessionState::Paused && s.seekable);
    session
        .submit(Command::Seek {
            generation: playing.generation,
            seconds: 0.0,
        })
        .unwrap();
    wait(&session, |s| {
        s.state == SessionState::Paused
            && s.seekable
            && s.position.is_some_and(|p| (p - 0.75).abs() < 0.03)
    });
    session
        .submit(Command::Seek {
            generation: playing.generation,
            seconds: 2.0,
        })
        .unwrap();
    wait(&session, |s| {
        // The paused null sink reports its buffered clock; it must stay inside the retained pause.
        s.seekable && s.position.is_some_and(|p| (1.8..=2.05).contains(&p))
    });
    session.submit(Command::Previous).unwrap();
    let restarted = wait(&session, |s| {
        s.seekable && s.position.is_some_and(|p| (p - 0.75).abs() < 0.03)
    });
    assert_eq!(restarted.generation, playing.generation);
    assert_eq!(restarted.current, Some(0));
    assert_eq!(restarted.state, SessionState::Paused);
    session
        .submit(Command::Seek {
            generation: playing.generation,
            seconds: 2.0,
        })
        .unwrap();
    wait(&session, |s| {
        s.seekable && s.position.is_some_and(|p| (1.8..=2.05).contains(&p))
    });
    session
        .submit(Command::Scan {
            generation: playing.generation,
            direction: Some(ScanDirection::Backward),
        })
        .unwrap();
    wait(&session, |s| {
        s.scanning.is_some() && s.position.is_some_and(|p| p <= 0.78)
    });
    thread::sleep(Duration::from_millis(150));
    let held = session.snapshot();
    assert_eq!(held.current, Some(0));
    assert!(held.position.unwrap() >= 0.74);
    session
        .submit(Command::Scan {
            generation: playing.generation,
            direction: None,
        })
        .unwrap();
    wait(&session, |s| {
        s.scanning.is_none() && s.state == SessionState::Paused
    });
    session
        .submit(Command::Edges(EdgeAction::ToggleTrim))
        .unwrap();
    wait(&session, |s| !s.edges.trim && s.trim_start == 0.0);
    session
        .submit(Command::Seek {
            generation: playing.generation,
            seconds: 0.0,
        })
        .unwrap();
    wait(&session, |s| s.position.is_some_and(|p| p < 0.03));
    session
        .submit(Command::Edges(EdgeAction::ToggleTrim))
        .unwrap();
    wait(&session, |s| s.edges.trim && s.trim_start > 0.74);
    session.submit(Command::TogglePause).unwrap();
    wait(&session, |s| s.state == SessionState::Playing);
    session
        .submit(Command::Seek {
            generation: playing.generation,
            seconds: 4.0,
        })
        .unwrap();
    let next = wait(&session, |s| {
        s.current == Some(1) && s.state == SessionState::Playing
    });
    assert_eq!(next.edge_status, EdgeStatus::Ready);
    assert!(next.position.unwrap() >= 0.74);
    session.submit(Command::TogglePause).unwrap();
    wait(&session, |s| s.state == SessionState::Paused && s.seekable);
    session
        .submit(Command::Seek {
            generation: next.generation,
            seconds: 1.3,
        })
        .unwrap();
    wait(&session, |s| {
        s.seekable && s.position.is_some_and(|p| (1.0..1.4).contains(&p))
    });
    session.submit(Command::Previous).unwrap();
    wait(&session, |s| {
        s.current == Some(0) && s.state == SessionState::Paused && s.seekable
    });
    session.submit(Command::Next).unwrap();
    wait(&session, |s| {
        s.current == Some(1) && s.state == SessionState::Paused && s.seekable
    });
    session.submit(Command::TogglePause).unwrap();
    let completion = Instant::now();
    wait(&session, |s| s.state == SessionState::Ended);
    assert!(completion.elapsed() < Duration::from_secs(4));
    // A distinct source forces a fresh job; Stop must cancel it without a stale result resuming audio.
    let copy = path.with_file_name("cancel.wav");
    fs::write(&copy, &original).unwrap();
    session.submit(Command::Open(vec![copy])).unwrap();
    wait(&session, |s| s.edge_status == EdgeStatus::Pending);
    let stop_started = Instant::now();
    session.submit(Command::Stop).unwrap();
    wait(&session, |s| s.state == SessionState::Stopped);
    assert!(stop_started.elapsed() < Duration::from_secs(1));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(session.snapshot().state, SessionState::Stopped);
    assert_eq!(session.snapshot().saved.edges.fade, FadeMode::Both);
    session.request_shutdown();
    drop(session);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(path.metadata().unwrap().modified().unwrap(), modified);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

/// Cold analysis leaves live transport usable, and completion cannot override a user's new pause intent.
#[test]
fn live_analysis_keeps_transport_and_pause_intent() {
    let (path, original) = fixture();
    let session = Session::start(AudioOutput::Silent).unwrap();
    session.submit(Command::Open(vec![path.clone()])).unwrap();
    let playing = wait(&session, |s| s.state == SessionState::Playing && s.seekable);
    session
        .submit(Command::Seek {
            generation: playing.generation,
            seconds: 2.0,
        })
        .unwrap();
    wait(&session, |s| {
        s.position.is_some_and(|p| (1.9..2.2).contains(&p))
    });
    session
        .submit(Command::Edges(EdgeAction::Set(PlaybackEdges {
            trim: true,
            fade: FadeMode::Both,
        })))
        .unwrap();
    let pending = wait(&session, |s| s.edge_status == EdgeStatus::Pending);
    assert_eq!(pending.state, SessionState::Playing);
    assert!(pending.seekable);
    assert_eq!(pending.generation, playing.generation);
    session.submit(Command::TogglePause).unwrap();
    wait(&session, |s| s.state == SessionState::Paused);
    let prepared = wait(&session, |s| s.edge_status == EdgeStatus::Ready);
    assert_eq!(prepared.state, SessionState::Paused);
    assert_eq!(prepared.generation, playing.generation);
    for trim in [false, true] {
        session
            .submit(Command::Edges(EdgeAction::ToggleTrim))
            .unwrap();
        let updated = wait(&session, |s| {
            s.edges.trim == trim && s.edge_status == EdgeStatus::Ready
        });
        assert_eq!(updated.state, SessionState::Paused);
        assert!(
            updated.position.is_some_and(|p| (1.8..2.4).contains(&p)),
            "toggle={trim}, prepared={:?}, updated={:?}",
            prepared.position,
            updated.position
        );
    }
    session.request_shutdown();
    drop(session);
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
