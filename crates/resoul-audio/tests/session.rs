//! Real native session journeys with generated read-only test inputs and bounded waits.

use resolibre_audio::{
    AudioOutput,
    session::{Command, Session, Snapshot, display_title},
};
use resolibre_core::queue::QueueAction;
use resolibre_core::session::{SessionState, Volume};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
    bytes: Vec<u8>,
}

impl Fixture {
    /// Generates measured regular attacks with short quiet edges, retaining bytes for preservation assertions.
    fn rhythmic(bpm: f64) -> Self {
        let mut fixture = Self::new(16);
        for frame in 0..16 * 8000 {
            let time = frame as f64 / 8000.0;
            let phase = (time - 1.0).rem_euclid(60.0 / bpm);
            let sample = if (1.0..15.0).contains(&time) && phase < 0.035 {
                ((std::f64::consts::TAU * 900.0 * phase).sin() * (-phase * 110.0).exp() * 12000.0)
                    as i16
            } else {
                0
            };
            fixture.bytes[44 + frame * 2..46 + frame * 2].copy_from_slice(&sample.to_le_bytes());
        }
        fs::write(&fixture.path, &fixture.bytes).unwrap();
        fixture
    }

    /// Makes an owned mono WAV long enough to distinguish seek targets and EOF.
    fn new(seconds: u32) -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("resolibre-session-{}-{id}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("track.wav");
        let size = seconds * 8000 * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
        bytes.extend_from_slice(&8000_u32.to_le_bytes());
        bytes.extend_from_slice(&16000_u32.to_le_bytes());
        bytes.extend_from_slice(b"\x02\0\x10\0data");
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.resize(44 + size as usize, 0);
        fs::write(&path, &bytes).unwrap();
        Self {
            directory,
            path,
            bytes,
        }
    }
}

impl Drop for Fixture {
    /// Verifies source preservation before deleting only this test's generated files.
    fn drop(&mut self) {
        assert_eq!(fs::read(&self.path).unwrap(), self.bytes);
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

/// Waits for actual worker observations, never accepting a previous revision.
fn wait(session: &Session, revision: u64, condition: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = session.snapshot();
        if snapshot.revision >= revision && condition(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "session deadline: {:?}, {:?}, position={:?}, duration={:?}, generation={}, requested revision={revision}, observed={}",
            snapshot.state,
            snapshot.error,
            snapshot.position,
            snapshot.duration,
            snapshot.generation,
            snapshot.revision
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Requires complete native release within the local reference deadline.
fn close(mut session: Session) {
    let start = Instant::now();
    session.request_shutdown();
    while !session.finish().unwrap() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "native cleanup deadline"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session.snapshot().finished);
}

/// Plays, pauses, seeks while paused, navigates and rejects a prior track's seek.
#[test]
fn controls_preserve_pause_and_reject_stale_seek() {
    let fixture = Fixture::new(130);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![
            fixture.path.clone(),
            fixture.path.clone(),
        ]))
        .unwrap();
    let first = wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.seekable
    });
    assert_ne!(first.tracks[0].id, first.tracks[1].id);
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| s.state == SessionState::Paused);
    let revision = session
        .submit(Command::Seek {
            generation: first.generation,
            seconds: 103.0,
        })
        .unwrap();
    // The paused null sink reports time-pos minus its buffered output delay.
    // This asserts command delivery only; marked monitor tests establish audio seek accuracy.
    wait(&session, revision, |s| {
        s.position.is_some_and(|v| (v - 103.0).abs() < 0.5)
    });
    let revision = session.submit(Command::Next).unwrap();
    let second = wait(&session, revision, |s| {
        s.current == Some(1) && s.state == SessionState::Paused
    });
    assert!(second.generation > first.generation);
    let revision = session
        .submit(Command::Seek {
            generation: first.generation,
            seconds: 100.0,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Paused && s.position.is_some_and(|v| v < 1.0)
    });
    let revision = session.submit(Command::Stop).unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Stopped && s.position == Some(0.0)
    });
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.position.is_some_and(|v| v < 1.0)
    });
    close(session);
}

/// Observed facts survive pause but never cross stop, failed selection or replacement.
#[test]
fn stream_information_follows_engine_lifetime() {
    let fixture = Fixture::new(6);
    let session = Session::start(AudioOutput::Silent).unwrap();
    assert!(session.snapshot().stream_info.is_none());
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone()]))
        .unwrap();
    let playing = wait(&session, revision, |s| {
        s.state == SessionState::Playing
            && s.stream_info.is_some_and(|info| info.channels == Some(1))
    });
    assert_eq!(
        playing.stream_info.unwrap().codec,
        resolibre_core::track_info::AudioCodec::Pcm
    );
    let revision = session.submit(Command::TogglePause).unwrap();
    let paused = wait(&session, revision, |s| s.state == SessionState::Paused);
    assert_eq!(paused.stream_info, playing.stream_info);
    let revision = session.submit(Command::Stop).unwrap();
    assert!(
        wait(&session, revision, |s| s.state == SessionState::Stopped)
            .stream_info
            .is_none()
    );
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.stream_info.is_some()
    });
    let missing = fixture.directory.join("missing.flac");
    let revision = session.submit(Command::Open(vec![missing])).unwrap();
    assert!(
        wait(&session, revision, |s| s.state == SessionState::Failed)
            .stream_info
            .is_none()
    );
    let revision = session
        .submit(Command::Replace(vec![fixture.path.clone()]))
        .unwrap();
    assert!(
        wait(&session, revision, |s| s.state == SessionState::Stopped)
            .stream_info
            .is_none()
    );
    close(session);
}

/// EOF advances exactly once per occurrence, then stops at the last item without looping.
#[test]
fn eof_advances_and_failed_input_can_recover() {
    let fixture = Fixture::new(1);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![
            fixture.path.clone(),
            fixture.path.clone(),
        ]))
        .unwrap();
    wait(&session, revision, |s| {
        s.current == Some(1) && s.state == SessionState::Ended
    });
    let revision = session
        .submit(Command::Open(vec![
            fixture.directory.join("missing-private-title"),
        ]))
        .unwrap();
    let failed = wait(&session, revision, |s| s.state == SessionState::Failed);
    assert!(!failed.error.as_deref().unwrap().contains("private"));
    let revision = session
        .submit(Command::Queue(QueueAction::Select(
            session.snapshot().tracks[0].id,
        )))
        .unwrap();
    wait(&session, revision, |s| s.state == SessionState::Playing);
    close(session);
}

/// Add leaves playback intact, keeps duplicates, and stops the queue at a bad next file.
#[test]
fn append_and_next_failure_do_not_silently_skip() {
    let fixture = Fixture::new(1);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone()]))
        .unwrap();
    let first = wait(&session, revision, |s| s.state == SessionState::Playing);
    let revision = session
        .submit(Command::Add(vec![
            fixture.directory.join("missing"),
            fixture.path.clone(),
        ]))
        .unwrap();
    let appended = wait(&session, revision, |s| s.tracks.len() == 3);
    assert_eq!(appended.generation, first.generation);
    let failed = wait(&session, revision, |s| s.state == SessionState::Failed);
    assert_eq!(failed.current, Some(1));
    let revision = session.submit(Command::Next).unwrap();
    wait(&session, revision, |s| {
        s.current == Some(2) && s.state == SessionState::Playing
    });
    close(session);
}

/// Superseded loading, command bursts and repeated destruction release their native owners.
#[test]
fn rapid_stop_and_shutdown_release_workers() {
    let fixture = Fixture::new(130);
    for _ in 0..12 {
        let session = Session::start(AudioOutput::Silent).unwrap();
        session
            .submit(Command::Open(vec![fixture.path.clone()]))
            .unwrap();
        for percent in 0..100 {
            let _ = session.submit(Command::Volume(
                Volume::from_percent(percent as f64).unwrap(),
            ));
        }
        let revision = session.submit(Command::Stop).unwrap();
        wait(&session, revision, |s| s.state == SessionState::Stopped);
        session
            .submit(Command::Open(vec![fixture.path.clone()]))
            .unwrap();
        close(session);
    }
}

/// Invalid floats, huge batches, and unsafe display characters cannot cross boundaries.
#[test]
fn rejects_unbounded_input_and_sanitizes_display() {
    let session = Session::start(AudioOutput::Silent).unwrap();
    assert!(
        session
            .submit(Command::Seek {
                generation: 0,
                seconds: f64::NAN
            })
            .is_err()
    );
    assert!(
        session
            .submit(Command::Add(vec![PathBuf::from("x"); 1025]))
            .is_err()
    );
    let title = display_title(std::path::Path::new("track\n\u{202e}.mp3"));
    assert!(!title.contains('\n'));
    assert!(!title.contains('\u{202e}'));
    close(session);
}

/// Exercises only explicitly supplied real files; paths and metadata never enter output.
#[test]
#[ignore = "requires RESOUL_TEST_FLAC and RESOUL_TEST_MP3 local inputs"]
fn supplied_music_journey() {
    let files = ["RESOUL_TEST_FLAC", "RESOUL_TEST_MP3"].map(|name| {
        PathBuf::from(std::env::var_os(name).expect("explicit local fixture required"))
    });
    let output = if std::env::var_os("RESOUL_TEST_DEVICE").is_some() {
        AudioOutput::Device
    } else {
        AudioOutput::Silent
    };
    let session = Session::start(output).unwrap();
    session
        .submit(Command::Equalizer(resolibre_core::equalizer::EqAction::Load(
            resolibre_core::equalizer::Equalizer::preset(2).unwrap(),
        )))
        .unwrap();
    let revision = session.submit(Command::Open(files.to_vec())).unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.seekable
    });
    for index in [0, 1, 0, 1] {
        let revision = session
            .submit(Command::Queue(QueueAction::Select(
                session.snapshot().tracks[index].id,
            )))
            .unwrap();
        let snapshot = wait(&session, revision, |s| {
            s.current == Some(index) && s.state == SessionState::Playing && s.seekable
        });
        let revision = session.submit(Command::TogglePause).unwrap();
        wait(&session, revision, |s| s.state == SessionState::Paused);
        for target in [103.0, 4.0] {
            let revision = session
                .submit(Command::Seek {
                    generation: snapshot.generation,
                    seconds: target,
                })
                .unwrap();
            wait(&session, revision, |s| {
                s.state == SessionState::Paused
                    && s.position.is_some_and(|v| (v - target).abs() < 0.5)
            });
        }
        let revision = session
            .submit(Command::Volume(Volume::from_percent(25.0).unwrap()))
            .unwrap();
        wait(&session, revision, |s| s.volume.percent() == 25.0);
        let revision = session.submit(Command::TogglePause).unwrap();
        wait(&session, revision, |s| s.state == SessionState::Playing);
        std::thread::sleep(Duration::from_millis(300));
        let revision = session.submit(Command::Stop).unwrap();
        wait(&session, revision, |s| s.state == SessionState::Stopped);
    }
    close(session);
}

/// A duration-less FLAC may still support absolute native seeks; duration is not capability.
#[test]
fn durationless_input_retains_native_seek_capability() {
    let mut fixture = Fixture::new(130);
    fixture.bytes = include_bytes!("fixtures/durationless.flac").to_vec();
    fs::write(&fixture.path, &fixture.bytes).unwrap();
    let path = fixture.path.clone();
    let mut native = resolibre_audio::LocalPlayer::open(&path, AudioOutput::Silent).unwrap();
    loop {
        if native.poll_event().unwrap() == Some(resolibre_core::PlaybackEvent::Loaded) {
            break;
        }
    }
    let timing = native.timing().unwrap();
    println!(
        "durationless native: duration={:?}, seekable={}",
        timing.duration, timing.seekable
    );
    assert!(timing.seekable);
    drop(native);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session.submit(Command::Open(vec![path])).unwrap();
    let loaded = wait(&session, revision, |s| s.state == SessionState::Playing);
    println!(
        "durationless session: duration={:?}, seekable={}",
        loaded.duration, loaded.seekable
    );
    assert!(loaded.seekable, "duration must not mask native capability");
    let revision = session
        .submit(Command::Seek {
            generation: loaded.generation,
            seconds: 103.0,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.position.is_some_and(|v| (103.0..104.0).contains(&v))
    });
    close(session);
}

/// Queue edits preserve native playback; removing current stops and stale IDs cannot act on others.
#[test]
fn queue_edits_preserve_playback_and_stop_only_removed_current() {
    let fixture = Fixture::new(130);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone(); 3]))
        .unwrap();
    let loaded = wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.seekable
    });
    let ids: Vec<_> = loaded.tracks.iter().map(|track| track.id).collect();
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| s.state == SessionState::Paused);
    let revision = session
        .submit(Command::Seek {
            generation: loaded.generation,
            seconds: 103.0,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.position.is_some_and(|p| (p - 103.0).abs() < 0.5)
    });
    let revision = session
        .submit(Command::Queue(QueueAction::MoveTo {
            id: ids[0],
            target: ids[1],
            after: true,
        }))
        .unwrap();
    let moved = wait(&session, revision, |s| s.current == Some(1));
    assert_eq!(moved.generation, loaded.generation);
    assert_eq!(moved.state, SessionState::Paused);
    assert!((moved.position.unwrap() - 103.0).abs() < 0.5);
    let revision = session
        .submit(Command::Queue(QueueAction::Remove(ids[1])))
        .unwrap();
    let removed = wait(&session, revision, |s| s.tracks.len() == 2);
    assert_eq!(removed.current, Some(0));
    assert_eq!(removed.generation, loaded.generation);
    assert_eq!(removed.state, SessionState::Paused);
    for action in [
        QueueAction::Select(ids[1]),
        QueueAction::Remove(ids[1]),
        QueueAction::MoveTo {
            id: ids[0],
            target: ids[1],
            after: false,
        },
        QueueAction::MoveTo {
            id: ids[1],
            target: ids[0],
            after: true,
        },
        QueueAction::Move {
            id: ids[1],
            forward: true,
        },
    ] {
        let revision = session.submit(Command::Queue(action)).unwrap();
        let unchanged = wait(&session, revision, |_| true);
        // Metadata completion may change the shared view revision without editing queue membership/order.
        let before_ids: Vec<_> = removed.tracks.iter().map(|track| track.id).collect();
        let after_ids: Vec<_> = unchanged.tracks.iter().map(|track| track.id).collect();
        assert_eq!(after_ids, before_ids);
        assert_eq!(unchanged.generation, removed.generation);
    }
    let revision = session
        .submit(Command::Queue(QueueAction::Remove(ids[0])))
        .unwrap();
    let stopped = wait(&session, revision, |s| s.state == SessionState::Stopped);
    assert_eq!(stopped.tracks[0].id, ids[2]);
    assert!(stopped.generation > loaded.generation);
    assert_eq!(stopped.position, Some(0.0));
    assert!(!stopped.seekable);
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| s.state == SessionState::Playing);
    let revision = session
        .submit(Command::Queue(QueueAction::Remove(ids[2])))
        .unwrap();
    let empty = wait(&session, revision, |s| s.state == SessionState::Idle);
    assert!(empty.tracks.is_empty());
    assert_eq!(empty.current, None);
    close(session);
}

/// Mode changes preserve the native engine; paused shuffle navigation replays actual history.
#[test]
fn shuffle_modes_preserve_timing_and_paused_history() {
    use resolibre_core::queue::RepeatMode;
    let fixture = Fixture::new(130);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone(); 4]))
        .unwrap();
    let first = wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.seekable
    });
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| s.state == SessionState::Paused);
    let revision = session
        .submit(Command::Seek {
            generation: first.generation,
            seconds: 103.0,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.position.is_some_and(|p| (p - 103.0).abs() < 0.5)
    });
    let revision = session
        .submit(Command::Queue(QueueAction::SetShuffle(true)))
        .unwrap();
    let enabled = wait(&session, revision, |s| s.shuffle);
    assert_eq!(enabled.generation, first.generation);
    assert_eq!(enabled.state, SessionState::Paused);
    assert!((enabled.position.unwrap() - 103.0).abs() < 0.5);
    assert!(enabled.can_previous); // Back can restart even at the beginning of shuffle history.
    assert!(enabled.can_next);
    let revision = session
        .submit(Command::Queue(QueueAction::SetRepeat(RepeatMode::One)))
        .unwrap();
    let changed = wait(&session, revision, |s| s.repeat == RepeatMode::One);
    assert_eq!(changed.generation, first.generation);
    let revision = session.submit(Command::Next).unwrap();
    let second = wait(&session, revision, |s| {
        s.state == SessionState::Paused && s.generation > first.generation
    });
    assert_ne!(second.current, first.current);
    let revision = session.submit(Command::Previous).unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Paused && s.current == first.current
    });
    let revision = session.submit(Command::Next).unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Paused && s.current == second.current
    });
    let revision = session.submit(Command::Stop).unwrap();
    let stopped = wait(&session, revision, |s| s.state == SessionState::Stopped);
    assert!(stopped.shuffle);
    assert_eq!(stopped.repeat, RepeatMode::One);
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.current == second.current
    });
    close(session);
}

/// Normal EOF repeats one/all, Off ends, and failure never becomes an automatic retry loop.
#[test]
fn repeat_eof_and_failure_follow_core_policy() {
    use resolibre_core::queue::RepeatMode;
    let fixture = Fixture::new(1);
    let session = Session::start(AudioOutput::Silent).unwrap();
    session
        .submit(Command::Queue(QueueAction::SetRepeat(RepeatMode::One)))
        .unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone(); 2]))
        .unwrap();
    let first = wait(&session, revision, |s| s.state == SessionState::Playing);
    let repeated = wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.generation > first.generation
    });
    assert_eq!(repeated.current, Some(0));
    let revision = session
        .submit(Command::Queue(QueueAction::SetRepeat(RepeatMode::All)))
        .unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.current == Some(1)
    });
    wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.current == Some(0)
    });
    let revision = session
        .submit(Command::Queue(QueueAction::SetRepeat(RepeatMode::Off)))
        .unwrap();
    wait(&session, revision, |s| s.state == SessionState::Ended);
    session
        .submit(Command::Queue(QueueAction::SetRepeat(RepeatMode::All)))
        .unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.directory.join("missing")]))
        .unwrap();
    let failed = wait(&session, revision, |s| s.state == SessionState::Failed);
    std::thread::sleep(Duration::from_millis(120));
    let stable = session.snapshot();
    assert_eq!(stable.generation, failed.generation);
    assert_eq!(stable.state, SessionState::Failed);
    close(session);
}

/// Native EOF advances a shuffled pass once per occurrence and then stops with Repeat Off.
#[test]
fn shuffle_eof_exhausts_one_pass() {
    let fixture = Fixture::new(1);
    let session = Session::start(AudioOutput::Silent).unwrap();
    session
        .submit(Command::Queue(QueueAction::SetShuffle(true)))
        .unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone(); 3]))
        .unwrap();
    let mut observed = wait(&session, revision, |s| s.state == SessionState::Playing);
    let mut visited = std::collections::HashSet::from([observed.current.unwrap()]);
    for _ in 0..2 {
        observed = wait(&session, revision, |s| {
            s.state == SessionState::Playing && s.generation > observed.generation
        });
        assert!(visited.insert(observed.current.unwrap()));
    }
    let ended = wait(&session, revision, |s| s.state == SessionState::Ended);
    assert_eq!(visited.len(), 3);
    assert!(!ended.can_next);
    assert!(ended.can_previous);
    close(session);
}

/// EQ survives paused seek/navigation/Stop; invalid edits never reach native execution.
#[test]
fn equalizer_preserves_session_and_handles_low_rates() {
    use resolibre_core::equalizer::{EqAction, Equalizer};
    let fixture = Fixture::new(130);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let curve = Equalizer::new(true, [6.0; 10], -2.0).unwrap();
    session
        .submit(Command::Equalizer(EqAction::Load(curve)))
        .unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone(); 2]))
        .unwrap();
    let loaded = wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.seekable
    });
    assert_eq!(loaded.eq_sample_rate, 8000);
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| s.state == SessionState::Paused);
    let revision = session
        .submit(Command::Seek {
            generation: loaded.generation,
            seconds: 103.0,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.position.is_some_and(|p| (p - 103.0).abs() < 0.5)
    });
    for action in [
        EqAction::Band { index: 9, db: 12.0 },
        EqAction::Enabled(false),
        EqAction::Enabled(true),
        EqAction::Reset,
    ] {
        let revision = session.submit(Command::Equalizer(action)).unwrap();
        let snapshot = wait(&session, revision, |s| s.state == SessionState::Paused);
        assert_eq!(snapshot.generation, loaded.generation);
        assert!(snapshot.position.is_some_and(|p| (p - 103.0).abs() < 0.5));
        std::thread::sleep(Duration::from_millis(250));
        assert_ne!(session.snapshot().state, SessionState::Failed);
    }
    assert!(
        session
            .submit(Command::Equalizer(EqAction::Preamp(f64::NAN)))
            .is_err()
    );
    let revision = session.submit(Command::Next).unwrap();
    wait(&session, revision, |s| {
        s.current == Some(1) && s.state == SessionState::Paused
    });
    let revision = session.submit(Command::Stop).unwrap();
    wait(&session, revision, |s| s.state == SessionState::Stopped);
    let revision = session.submit(Command::TogglePause).unwrap();
    let snapshot = wait(&session, revision, |s| s.state == SessionState::Playing);
    assert!(snapshot.equalizer.enabled());
    close(session);
}

/// Measures bounded reuse with full EQ curves and graph-preserving seek; invoke with external RSS sampling.
#[test]
#[ignore = "native resource workload; run separately with allocator policy and RSS sampling"]
fn equalizer_resource_cycles() {
    use resolibre_core::equalizer::{EqAction, Equalizer};
    let fixture = Fixture::new(130);
    let session = Session::start(AudioOutput::Silent).unwrap();
    session
        .submit(Command::Add(vec![fixture.path.clone()]))
        .unwrap();
    for cycle in 0..60 {
        let gain = if cycle % 2 == 0 { 12.0 } else { -12.0 };
        let settings = Equalizer::new(true, [gain; 10], -2.0).unwrap();
        session
            .submit(Command::Equalizer(EqAction::Load(settings)))
            .unwrap();
        let revision = session.submit(Command::TogglePause).unwrap();
        let snapshot = wait(&session, revision, |s| {
            s.state == SessionState::Playing && s.seekable
        });
        session
            .submit(Command::Equalizer(EqAction::Preamp(-1.0)))
            .unwrap();
        let revision = session
            .submit(Command::Seek {
                generation: snapshot.generation,
                seconds: 103.0,
            })
            .unwrap();
        wait(&session, revision, |s| {
            s.position.is_some_and(|p| (p - 103.0).abs() < 0.5)
        });
        let revision = session.submit(Command::Stop).unwrap();
        wait(&session, revision, |s| s.state == SessionState::Stopped);
    }
    close(session);
}

/// Seeking to EOF after the startup deadline must advance, not treat draining audio as failed load.
#[test]
fn seek_to_eof_after_startup_advances_queue() {
    let fixture = Fixture::new(30);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![
            fixture.path.clone(),
            fixture.path.clone(),
        ]))
        .unwrap();
    let first = wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.seekable
    });
    // Exercise a late seek: startup's deadline has expired, but playback is already established.
    std::thread::sleep(Duration::from_millis(10_100));
    let revision = session
        .submit(Command::Seek {
            generation: first.generation,
            seconds: 30.0,
        })
        .unwrap();
    let next = wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.current == Some(1)
    });
    assert!(next.generation > first.generation);
    assert!(next.error.is_none());
    let curve = resolibre_core::equalizer::Equalizer::preset(2).unwrap();
    session
        .submit(Command::Equalizer(resolibre_core::equalizer::EqAction::Load(
            curve,
        )))
        .unwrap();
    let revision = session
        .submit(Command::Seek {
            generation: next.generation,
            seconds: 30.0,
        })
        .unwrap();
    let ended = wait(&session, revision, |s| s.state == SessionState::Ended);
    assert_eq!(ended.current, Some(1));
    assert!(ended.error.is_none());
    assert_eq!(ended.equalizer, curve);
    close(session);
}

/// Exercises late and near-EOF seeks across MP3/FLAC transitions using explicitly supplied inputs.
#[test]
#[ignore = "requires RESOUL_TEST_FLAC and RESOUL_TEST_MP3 local inputs"]
fn supplied_music_eof_seek_journey() {
    use resolibre_core::equalizer::{EqAction, Equalizer};
    let mp3 = PathBuf::from(std::env::var_os("RESOUL_TEST_MP3").expect("MP3 fixture"));
    let flac = PathBuf::from(std::env::var_os("RESOUL_TEST_FLAC").expect("FLAC fixture"));
    let paths = vec![
        mp3.clone(),
        flac.clone(),
        mp3.clone(),
        flac.clone(),
        mp3,
        flac,
    ];
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session.submit(Command::Open(paths)).unwrap();
    let mut current = wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.seekable
    });
    assert!(current.duration.is_some_and(|seconds| seconds > 12.0));
    std::thread::sleep(Duration::from_millis(10_100));
    for index in 0..6 {
        let mut curve = Equalizer::preset(2).unwrap();
        curve.apply(EqAction::Enabled(index % 2 == 1)).unwrap();
        session
            .submit(Command::Equalizer(EqAction::Load(curve)))
            .unwrap();
        let duration = current.duration.expect("known fixture duration");
        let target = if index % 2 == 0 {
            duration
        } else {
            duration - 0.02
        };
        let revision = session
            .submit(Command::Seek {
                generation: current.generation,
                seconds: target,
            })
            .unwrap();
        let next = wait(&session, revision, |s| {
            if index == 5 {
                s.state == SessionState::Ended
            } else {
                s.current == Some(index + 1) && s.state == SessionState::Playing && s.seekable
            }
        });
        assert!(next.error.is_none());
        assert_eq!(next.equalizer, curve);
        if index < 5 {
            assert_eq!(next.generation, current.generation + 1);
        }
        current = next;
    }
    close(session);
}

/// Repeated pointer-like seeks must not fail while EOF is pending or skip the next occurrence.
#[test]
fn repeated_end_seeks_preserve_three_track_queue() {
    let fixture = Fixture::new(30);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone(); 3]))
        .unwrap();
    let first = wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.seekable
    });
    let mut revision = revision;
    for _ in 0..40 {
        revision = session
            .submit(Command::Seek {
                generation: first.generation,
                seconds: 30.0,
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(2));
        assert_ne!(
            session.snapshot().state,
            SessionState::Failed,
            "late seek must not fail playback"
        );
    }
    let next = wait(&session, revision, |s| {
        s.current == Some(1) && s.state == SessionState::Playing
    });
    assert_eq!(next.generation, first.generation + 1);
    assert!(next.position.is_some_and(|seconds| seconds < 2.0));
    assert!(next.error.is_none());
    close(session);
}

/// Restoring missing references stays silent; later Play uses the selected occurrence and saved gain/EQ.
#[test]
fn restore_preserves_intent_without_opening_audio() {
    use resolibre_audio::session::SavedSession;
    use resolibre_core::equalizer::Equalizer;
    use resolibre_core::queue::RepeatMode;
    let fixture = Fixture::new(5);
    let saved = SavedSession {
        mix_enabled: false,
        paths: vec![
            fixture.directory.join("missing.mp3"),
            fixture.path.clone(),
            fixture.path.clone(),
        ]
        .into(),
        current: Some(2),
        volume: Volume::from_percent(21.0).unwrap(),
        equalizer: Equalizer::new(true, [2.0; 10], -2.0).unwrap(),
        edges: Default::default(),
        shuffle: true,
        repeat: RepeatMode::All,
    };
    let session = Session::start(AudioOutput::Silent).unwrap();
    let invalid = SavedSession {
        current: Some(3),
        ..saved.clone()
    };
    assert!(session.submit(Command::Restore(invalid)).is_err());
    let revision = session.submit(Command::Restore(saved.clone())).unwrap();
    let snapshot = wait(&session, revision, |s| s.state == SessionState::Stopped);
    assert_eq!(snapshot.saved, saved);
    assert_eq!(snapshot.generation, 0);
    assert!(!snapshot.seekable);
    assert!(snapshot.error.is_none());
    assert_ne!(snapshot.tracks[1].id, snapshot.tracks[2].id);
    assert!(
        session
            .submit(Command::Restore(SavedSession::default()))
            .is_err()
    );
    let revision = session.submit(Command::TogglePause).unwrap();
    let playing = wait(&session, revision, |s| s.state == SessionState::Playing);
    assert_eq!(playing.current, Some(2));
    assert_eq!(playing.volume, saved.volume);
    assert_eq!(playing.equalizer, saved.equalizer);
    let revision = session
        .submit(Command::Queue(QueueAction::Move {
            id: playing.tracks[2].id,
            forward: false,
        }))
        .unwrap();
    let moved = wait(&session, revision, |s| s.current == Some(1));
    assert_eq!(moved.saved.current, Some(1));
    let revision = session
        .submit(Command::Queue(QueueAction::Remove(moved.tracks[0].id)))
        .unwrap();
    let removed = wait(&session, revision, |s| s.tracks.len() == 2);
    assert_eq!(removed.saved.paths.len(), 2);
    close(session);
}

/// Replacement stops once, resets timing and keeps gain/EQ/modes; invalid and stale intents are inert.
#[test]
fn replacement_preserves_settings_and_rejects_invalid_batches() {
    use resolibre_core::equalizer::EqAction;
    use resolibre_core::queue::RepeatMode;
    let fixture = Fixture::new(130);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone()]))
        .unwrap();
    let playing = wait(&session, revision, |s| s.state == SessionState::Playing);
    let old_id = playing.tracks[0].id;
    let generation = playing.generation;
    assert!(session.submit(Command::Replace(vec![])).is_err());
    assert_eq!(session.snapshot().generation, generation);
    session
        .submit(Command::Volume(Volume::from_percent(32.0).unwrap()))
        .unwrap();
    session
        .submit(Command::Equalizer(EqAction::Preamp(-1.0)))
        .unwrap();
    session
        .submit(Command::Queue(QueueAction::SetShuffle(true)))
        .unwrap();
    session
        .submit(Command::Queue(QueueAction::SetRepeat(RepeatMode::All)))
        .unwrap();
    let revision = session
        .submit(Command::Replace(vec![fixture.path.clone(); 2]))
        .unwrap();
    let replaced = wait(&session, revision, |s| {
        s.state == SessionState::Stopped && s.tracks.len() == 2
    });
    assert_eq!(replaced.current, Some(0));
    assert_eq!(replaced.position, Some(0.0));
    assert_eq!(replaced.duration, None);
    assert!(!replaced.seekable);
    assert_eq!(replaced.volume.percent(), 32.0);
    assert!(replaced.shuffle);
    assert_eq!(replaced.repeat, RepeatMode::All);
    assert_eq!(replaced.equalizer.preamp(), -1.0);
    assert!(replaced.tracks[0].id > old_id);
    let revision = session
        .submit(Command::Queue(QueueAction::Select(old_id)))
        .unwrap();
    let stale = wait(&session, revision, |_| true);
    assert_eq!(stale.generation, replaced.generation);
    assert_eq!(stale.state, SessionState::Stopped);
    close(session);
}

/// Native scanning runs at 3x in both directions, restores pause/speed, and rejects stale holds.
#[test]
fn hold_scan_restores_transport_and_stays_on_current_track() {
    use resolibre_core::session::ScanDirection;
    let fixture = Fixture::new(30);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![
            fixture.path.clone(),
            fixture.path.clone(),
        ]))
        .unwrap();
    let loaded = wait(&session, revision, |s| {
        s.seekable && s.state == SessionState::Playing
    });
    let generation = loaded.generation;
    let revision = session
        .submit(Command::Seek {
            generation,
            seconds: 12.0,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.position.is_some_and(|p| (p - 12.0).abs() < 0.5)
    });
    for direction in [ScanDirection::Forward, ScanDirection::Backward] {
        let revision = session
            .submit(Command::Scan {
                generation,
                direction: Some(direction),
            })
            .unwrap();
        wait(&session, revision, |s| s.scanning == Some(direction));
        std::thread::sleep(Duration::from_millis(200));
        let before = session.snapshot().position.unwrap();
        std::thread::sleep(Duration::from_millis(600));
        let after = session.snapshot();
        let moved = after.position.unwrap() - before;
        let expected = if direction == ScanDirection::Forward {
            1.8
        } else {
            -1.8
        };
        assert!((moved - expected).abs() < 0.45, "scan movement: {moved}");
        assert_eq!(after.current, Some(0));
        let revision = session
            .submit(Command::Scan {
                generation,
                direction: None,
            })
            .unwrap();
        wait(&session, revision, |s| {
            s.scanning.is_none() && s.state == SessionState::Playing
        });
        std::thread::sleep(Duration::from_millis(200));
        let before = session.snapshot().position.unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let moved = session.snapshot().position.unwrap() - before;
        assert!((moved - 0.5).abs() < 0.25, "normal speed: {moved}");
    }
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| s.state == SessionState::Paused);
    let revision = session
        .submit(Command::Seek {
            generation,
            seconds: 0.5,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.position.is_some_and(|p| (p - 0.5).abs() < 0.25)
    });
    let revision = session
        .submit(Command::Scan {
            generation,
            direction: Some(ScanDirection::Backward),
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.scanning.is_some() && s.position.is_some_and(|p| p < 0.05)
    });
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(session.snapshot().generation, generation);
    let revision = session
        .submit(Command::Scan {
            generation,
            direction: None,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.scanning.is_none() && s.state == SessionState::Paused
    });
    let revision = session
        .submit(Command::Seek {
            generation,
            seconds: 29.5,
        })
        .unwrap();
    wait(&session, revision, |s| s.position.is_some_and(|p| p > 29.0));
    let revision = session
        .submit(Command::Scan {
            generation,
            direction: Some(ScanDirection::Forward),
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.scanning.is_some() && s.position.is_some_and(|p| p >= 29.9)
    });
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(session.snapshot().generation, generation);
    let revision = session
        .submit(Command::Scan {
            generation,
            direction: None,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.scanning.is_none() && s.state == SessionState::Paused
    });
    assert_eq!(session.snapshot().current, Some(0));
    let revision = session.submit(Command::Next).unwrap();
    let next = wait(&session, revision, |s| {
        s.current == Some(1) && s.state == SessionState::Paused
    });
    assert!(next.scanning.is_none());
    let revision = session
        .submit(Command::Scan {
            generation,
            direction: Some(ScanDirection::Forward),
        })
        .unwrap();
    let stale = wait(&session, revision, |_| true);
    assert!(stale.scanning.is_none());
    assert_eq!(stale.generation, next.generation);
    let generation = next.generation;
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| s.state == SessionState::Playing);
    let revision = session
        .submit(Command::Seek {
            generation,
            seconds: 29.5,
        })
        .unwrap();
    wait(&session, revision, |s| s.position.is_some_and(|p| p > 29.0));
    let revision = session
        .submit(Command::Scan {
            generation,
            direction: Some(ScanDirection::Forward),
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.scanning.is_some() && s.position.is_some_and(|p| p >= 29.9)
    });
    assert_eq!(session.snapshot().current, Some(1));
    let revision = session
        .submit(Command::Scan {
            generation,
            direction: None,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.scanning.is_none() && s.state == SessionState::Ended
    });
    close(session);
}

/// The worker owns glides without a player, persists their target and lets manual controls cancel them.
#[test]
fn volume_glide_retargets_cancels_and_survives_idle() {
    let session = Session::start(AudioOutput::Silent).unwrap();
    let target = Volume::from_percent(100.0).unwrap();
    let revision = session.submit(Command::GlideVolume(target)).unwrap();
    let moving = wait(&session, revision, |s| s.volume.percent() > 55.0);
    assert!(moving.volume.percent() < 95.0);
    assert_eq!(moving.saved.volume, target);
    assert_eq!(moving.state, SessionState::Idle);
    let before = moving.volume.percent();
    let revision = session
        .submit(Command::GlideVolume(Volume::from_percent(0.0).unwrap()))
        .unwrap();
    let retargeted = wait(&session, revision, |_| true);
    assert!(
        (retargeted.volume.percent() - before).abs() < 3.0,
        "retarget must not jump"
    );
    wait(&session, revision, |s| s.volume.percent() < before - 5.0);
    let manual = Volume::from_percent(37.0).unwrap();
    let revision = session.submit(Command::Volume(manual)).unwrap();
    wait(&session, revision, |s| s.volume == manual);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(session.snapshot().volume, manual);
    assert_eq!(session.snapshot().saved.volume, manual);
    let started = Instant::now();
    let revision = session.submit(Command::GlideVolume(target)).unwrap();
    wait(&session, revision, |s| s.volume == target);
    assert!(started.elapsed() >= Duration::from_millis(1950));
    assert!(started.elapsed() < Duration::from_millis(2500));
    close(session);
}

/// A native player keeps the same glide through pause and track replacement, without resetting gain intent.
#[test]
fn volume_glide_continues_across_native_pause_and_next() {
    let fixture = Fixture::new(10);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![
            fixture.path.clone(),
            fixture.path.clone(),
        ]))
        .unwrap();
    wait(&session, revision, |s| s.state == SessionState::Playing);
    let target = Volume::from_percent(90.0).unwrap();
    let revision = session.submit(Command::GlideVolume(target)).unwrap();
    wait(&session, revision, |s| s.volume.percent() > 51.0);
    let revision = session.submit(Command::TogglePause).unwrap();
    let paused = wait(&session, revision, |s| s.state == SessionState::Paused);
    wait(&session, revision, |s| {
        s.volume.percent() > paused.volume.percent() + 2.0
    });
    let revision = session.submit(Command::Next).unwrap();
    let next = wait(&session, revision, |s| {
        s.current == Some(1) && s.state == SessionState::Paused
    });
    assert_eq!(next.saved.volume, target);
    let complete = wait(&session, revision, |s| s.volume == target);
    assert_eq!(complete.state, SessionState::Paused);
    assert!(complete.error.is_none());
    close(session);
}

/// Back restarts without branching history, then navigates during the first second; pause survives.
#[test]
fn previous_restarts_then_navigates() {
    let fixture = Fixture::new(20);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone(); 2]))
        .unwrap();
    wait(&session, revision, |s| {
        s.state == SessionState::Playing && s.seekable
    });
    let revision = session.submit(Command::Next).unwrap();
    let second = wait(&session, revision, |s| {
        s.current == Some(1) && s.state == SessionState::Playing && s.seekable
    });
    assert!(second.previous_navigates);
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |s| s.state == SessionState::Paused);
    let revision = session
        .submit(Command::Seek {
            generation: second.generation,
            seconds: 5.0,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.seekable && s.position.is_some_and(|p| p > 4.5)
    });
    let revision = session.submit(Command::Previous).unwrap();
    let restarted = wait(&session, revision, |s| {
        s.seekable && s.position.is_some_and(|p| p < 0.1)
    });
    assert_eq!(restarted.current, Some(1));
    assert_eq!(restarted.generation, second.generation);
    assert_eq!(restarted.state, SessionState::Paused);
    assert!(restarted.previous_navigates);
    let revision = session.submit(Command::Previous).unwrap();
    let first = wait(&session, revision, |s| s.current == Some(0) && s.seekable);
    assert_eq!(first.state, SessionState::Paused);
    let revision = session
        .submit(Command::Seek {
            generation: first.generation,
            seconds: 5.0,
        })
        .unwrap();
    let later = wait(&session, revision, |s| {
        s.seekable && s.position.is_some_and(|p| p > 4.5)
    });
    assert!(later.can_previous, "first track must allow restart");
    assert!(!later.previous_navigates);
    let revision = session.submit(Command::Previous).unwrap();
    let restart = wait(&session, revision, |s| {
        s.seekable && s.position.is_some_and(|p| p < 0.1)
    });
    assert_eq!(restart.generation, first.generation);
    assert!(!restart.can_previous);
    assert!(!restart.previous_navigates);
    let revision = session.submit(Command::Previous).unwrap();
    let unchanged = wait(&session, revision, |_| true);
    assert_eq!(unchanged.current, Some(0));
    assert_eq!(unchanged.generation, first.generation);
    let revision = session.submit(Command::Next).unwrap();
    let second = wait(&session, revision, |s| s.current == Some(1) && s.seekable);
    let revision = session
        .submit(Command::Seek {
            generation: second.generation,
            seconds: 5.0,
        })
        .unwrap();
    wait(&session, revision, |s| {
        s.seekable && s.position.is_some_and(|p| p > 4.5)
    });
    session.submit(Command::Previous).unwrap();
    let revision = session.submit(Command::Previous).unwrap();
    wait(&session, revision, |s| {
        s.current == Some(0) && s.state == SessionState::Paused
    });
    close(session);
}

/// Full original-stream analysis measures BPM without changing source bytes or confusing replacement generations.
#[test]
fn full_analysis_measures_beats_and_rejects_stale_results() {
    use resolibre_audio::session::AnalysisStatus;
    let mut fixture = Fixture::new(32);
    for frame in 0..32 * 8000 {
        let time = frame as f64 / 8000.0;
        let phase = (time - 0.25).rem_euclid(0.5);
        let sample = if time >= 0.25 && phase < 0.035 {
            ((std::f64::consts::TAU * 900.0 * phase).sin() * (-phase * 110.0).exp() * 12000.0)
                as i16
        } else {
            0
        };
        fixture.bytes[44 + frame * 2..46 + frame * 2].copy_from_slice(&sample.to_le_bytes());
    }
    fs::write(&fixture.path, &fixture.bytes).unwrap();
    let before = fs::metadata(&fixture.path).unwrap().modified().unwrap();
    let other = Fixture::new(600);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone()]))
        .unwrap();
    let loaded = wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Playing
    });
    let revision = session
        .submit(Command::Analyze {
            generation: loaded.generation,
        })
        .unwrap();
    let complete = wait(&session, revision, |snapshot| {
        matches!(snapshot.analysis, AnalysisStatus::Complete(_))
    });
    let AnalysisStatus::Complete(report) = &complete.analysis else {
        unreachable!()
    };
    assert!((report.tempo.bpm.unwrap() - 120.0).abs() < 0.5);
    assert!(report.tempo.segments[0].beats.len() > 50);
    assert_eq!(report.channels, 1);
    assert_eq!(report.sample_rate_hz, 8000);
    assert!((report.duration_seconds - 32.0).abs() < 0.001);
    assert!(report.columns <= resolibre_core::analysis::MAX_COLUMNS);
    assert!(complete.position.unwrap() > loaded.position.unwrap_or(0.0));
    let revision = session
        .submit(Command::Open(vec![other.path.clone()]))
        .unwrap();
    let next = wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Playing && snapshot.generation > loaded.generation
    });
    let revision = session
        .submit(Command::Analyze {
            generation: loaded.generation,
        })
        .unwrap();
    wait(&session, revision, |snapshot| {
        matches!(snapshot.analysis, AnalysisStatus::Idle)
    });
    let revision = session
        .submit(Command::Analyze {
            generation: next.generation,
        })
        .unwrap();
    wait(&session, revision, |snapshot| {
        matches!(snapshot.analysis, AnalysisStatus::Running { .. })
    });
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Paused
    });
    let revision = session
        .submit(Command::CancelAnalysis {
            generation: next.generation,
        })
        .unwrap();
    wait(&session, revision, |snapshot| {
        matches!(snapshot.analysis, AnalysisStatus::Cancelled)
    });
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone()]))
        .unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Playing && matches!(snapshot.analysis, AnalysisStatus::Idle)
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(matches!(session.snapshot().analysis, AnalysisStatus::Idle));
    close(session);
    assert_eq!(
        fs::metadata(&fixture.path).unwrap().modified().unwrap(),
        before
    );
}

/// Automatic rhythm requires no Analyze command, prepares the actual next occurrence and reuses it on advance.
#[test]
fn rhythm_is_automatic_and_prepares_next_without_spectrogram() {
    use resolibre_audio::session::{AnalysisStatus, RhythmStatus};
    let mut fixtures = [Fixture::new(96), Fixture::new(96)];
    for (fixture, bpm) in fixtures.iter_mut().zip([120.0, 140.0]) {
        for frame in 0..96 * 8000 {
            let time = frame as f64 / 8000.0;
            let phase = (time - 0.25).rem_euclid(60.0 / bpm);
            let sample = if time >= 0.25 && phase < 0.035 {
                ((std::f64::consts::TAU * 900.0 * phase).sin() * (-phase * 110.0).exp() * 12000.0)
                    as i16
            } else {
                0
            };
            fixture.bytes[44 + frame * 2..46 + frame * 2].copy_from_slice(&sample.to_le_bytes());
        }
        fs::write(&fixture.path, &fixture.bytes).unwrap();
    }
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(
            fixtures
                .iter()
                .map(|fixture| fixture.path.clone())
                .collect(),
        ))
        .unwrap();
    let first = wait(&session, revision, |snapshot| {
        matches!(snapshot.rhythm, RhythmStatus::Ready(_))
    });
    assert!(
        first.next_rhythm.is_none(),
        "MIX off must not prepare the next track"
    );
    let revision = session.submit(Command::SetMix(true)).unwrap();
    let prepared = wait(&session, revision, |snapshot| {
        snapshot.next_rhythm.is_some()
    });
    assert!(matches!(first.analysis, AnalysisStatus::Idle));
    let RhythmStatus::Ready(report) = &first.rhythm else {
        unreachable!()
    };
    assert!((report.bpm.unwrap() - 120.0).abs() < 0.5);
    assert!(
        report
            .segments
            .iter()
            .all(|segment| segment.end_seconds <= 64.0)
    );
    assert!(
        report
            .segments
            .iter()
            .any(|segment| segment.beats.len() > 50)
    );
    let next = prepared.next_rhythm.unwrap();
    assert!(
        (next.report.bpm.unwrap() - 140.0).abs() < 0.5,
        "next BPM {:?}",
        next.report.bpm
    );
    let revision = session.submit(Command::Next).unwrap();
    let second = wait(&session, revision, |snapshot| {
        snapshot.current == Some(1) && matches!(snapshot.rhythm, RhythmStatus::Ready(_))
    });
    let RhythmStatus::Ready(report) = &second.rhythm else {
        unreachable!()
    };
    assert!(std::sync::Arc::ptr_eq(&next.report, report));
    assert!(matches!(second.analysis, AnalysisStatus::Idle));
    assert!(second.next_rhythm.is_none());
    let revision = session
        .submit(Command::Queue(QueueAction::Select(second.tracks[0].id)))
        .unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.current == Some(0) && matches!(snapshot.rhythm, RhythmStatus::Ready(_))
    });
    // An external rewrite invalidates the prepared report when its stream is opened for playback.
    fixtures[1].bytes = fixtures[0].bytes.clone();
    fs::write(&fixtures[1].path, &fixtures[1].bytes).unwrap();
    let revision = session.submit(Command::Next).unwrap();
    let changed = wait(&session, revision, |snapshot| {
        snapshot.current == Some(1) && matches!(snapshot.rhythm, RhythmStatus::Ready(_))
    });
    let RhythmStatus::Ready(report) = changed.rhythm else {
        unreachable!()
    };
    assert!((report.bpm.unwrap() - 120.0).abs() < 0.5);
    assert!(!std::sync::Arc::ptr_eq(&next.report, &report));
    close(session);
}

/// Real native overlap advances exactly once, retains controls and adopts the already-playing next source.
#[test]
fn mix_prepares_overlaps_and_adopts_without_replaying_intro() {
    use resolibre_audio::session::MixStatus;
    let fixtures = [Fixture::rhythmic(120.0), Fixture::rhythmic(124.0)];
    let session = Session::start(AudioOutput::Silent).unwrap();
    session.submit(Command::SetMix(true)).unwrap();
    let revision = session
        .submit(Command::Open(
            fixtures.iter().map(|item| item.path.clone()).collect(),
        ))
        .unwrap();
    let ready = wait(&session, revision, |snapshot| {
        snapshot.mix_status == MixStatus::Ready
    });
    assert_eq!(
        ready.mix_beat_ratio,
        Some(resolibre_core::transition::BeatRatio::OneToOne)
    );
    let revision = session
        .submit(Command::Seek {
            generation: ready.generation,
            seconds: 10.0,
        })
        .unwrap();
    let overlap = wait(&session, revision, |snapshot| {
        matches!(
            snapshot.mix_status,
            MixStatus::BeatMixing | MixStatus::TempoMixing | MixStatus::Crossfading
        )
    });
    assert_eq!(overlap.current, Some(0));
    session
        .submit(Command::GlideVolume(Volume::from_percent(30.0).unwrap()))
        .unwrap();
    let revision = session.submit(Command::TogglePause).unwrap();
    let paused = wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Paused
    });
    std::thread::sleep(Duration::from_millis(150));
    assert!((session.snapshot().position.unwrap() - paused.position.unwrap()).abs() < 0.1);
    let revision = session.submit(Command::TogglePause).unwrap();
    let next = wait(&session, revision, |snapshot| {
        snapshot.current == Some(1) && snapshot.state == SessionState::Playing
    });
    assert!(
        next.position.unwrap() > 2.0,
        "incoming intro must not restart at handover"
    );
    assert_eq!(next.generation, ready.generation + 1);
    assert!(
        (next.volume.percent() - 30.0).abs() < 0.1,
        "master volume survives both streams and handover"
    );
    let revision = session.submit(Command::SetMix(false)).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.mix_status == MixStatus::Off
            && snapshot.next_rhythm.is_none()
            && snapshot.mix_beat_ratio.is_none()
    });
    // A promoted stream may retain its unity-speed MIX filter; scanning must restore its own graph.
    let revision = session
        .submit(Command::Scan {
            generation: next.generation,
            direction: Some(resolibre_core::session::ScanDirection::Forward),
        })
        .unwrap();
    wait(&session, revision, |snapshot| snapshot.scanning.is_some());
    std::thread::sleep(Duration::from_millis(200));
    let before = session.snapshot().position.unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let moved = session.snapshot().position.unwrap() - before;
    assert!((moved - 1.2).abs() < 0.4, "scan after MIX: {moved}");
    let revision = session
        .submit(Command::Scan {
            generation: next.generation,
            direction: None,
        })
        .unwrap();
    wait(&session, revision, |snapshot| snapshot.scanning.is_none());
    close(session);
}

/// A promoted stream retains its return across pause and stale seeks; an intentional seek cancels it.
#[test]
fn mix_tempo_return_survives_handover_and_pause() {
    use resolibre_audio::session::MixStatus;
    let fixtures = [Fixture::rhythmic(120.0), Fixture::rhythmic(142.0)];
    let session = Session::start(AudioOutput::Silent).unwrap();
    session.submit(Command::SetMix(true)).unwrap();
    let revision = session
        .submit(Command::Open(
            fixtures.iter().map(|item| item.path.clone()).collect(),
        ))
        .unwrap();
    let ready = wait(&session, revision, |snapshot| {
        snapshot.mix_status == MixStatus::Ready
    });
    let revision = session
        .submit(Command::Seek {
            generation: ready.generation,
            seconds: ready.mix_region.unwrap().start,
        })
        .unwrap();
    let promoted = wait(&session, revision, |snapshot| {
        snapshot.current == Some(1) && snapshot.mix_status == MixStatus::TempoMixing
    });
    assert_eq!(promoted.generation, ready.generation + 1);
    assert!(promoted.mix_region.is_none());
    assert!(promoted.mix_beat_ratio.is_none());
    let revision = session.submit(Command::TogglePause).unwrap();
    let paused = wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Paused
    });
    // A wall-clock ramp would finish while paused; the source-clock ramp must remain pending.
    std::thread::sleep(Duration::from_secs_f64(
        resolibre_core::transition::TEMPO_RETURN_DURATION_SECONDS + 0.1,
    ));
    let frozen = session.snapshot();
    assert_eq!(frozen.mix_status, MixStatus::TempoMixing);
    assert!((frozen.position.unwrap() - paused.position.unwrap()).abs() < 0.1);
    let revision = session
        .submit(Command::Seek {
            generation: ready.generation,
            seconds: 0.0,
        })
        .unwrap();
    let stale = wait(&session, revision, |snapshot| snapshot.revision >= revision);
    assert_eq!(stale.mix_status, MixStatus::TempoMixing);
    assert_eq!(stale.generation, promoted.generation);

    let revision = session
        .submit(Command::Seek {
            generation: promoted.generation,
            seconds: 8.0,
        })
        .unwrap();
    // Buffered native timing can lag a paused seek; verify actual speed separately after resume.
    wait(&session, revision, |snapshot| {
        snapshot.mix_status != MixStatus::TempoMixing
            && snapshot
                .position
                .is_some_and(|position| (position - 8.0).abs() < 0.25)
    });
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Playing
    });
    std::thread::sleep(Duration::from_millis(200));
    let before = session.snapshot().position.unwrap();
    std::thread::sleep(Duration::from_secs(1));
    let moved = session.snapshot().position.unwrap() - before;
    assert!(
        (moved - 1.0).abs() < 0.1,
        "seek must restore normal playback speed: {moved}"
    );
    close(session);
}

/// Repeated seeks keep a muted prepared transition, including stale-generation requests.
#[test]
fn mix_ready_survives_repeated_seeks() {
    use resolibre_audio::session::MixStatus;
    let fixtures = [Fixture::rhythmic(120.0), Fixture::rhythmic(124.0)];
    let session = Session::start(AudioOutput::Silent).unwrap();
    session.submit(Command::SetMix(true)).unwrap();
    let revision = session
        .submit(Command::Open(
            fixtures.iter().map(|item| item.path.clone()).collect(),
        ))
        .unwrap();
    let ready = wait(&session, revision, |snapshot| {
        snapshot.mix_status == MixStatus::Ready
    });
    let region = ready
        .mix_region
        .expect("ready transition has measured bounds");
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Paused
    });
    for seconds in [4.0, 2.0, 7.0, 1.0] {
        let revision = session
            .submit(Command::Seek {
                generation: ready.generation,
                seconds,
            })
            .unwrap();
        let observed = wait(&session, revision, |_| true);
        assert_eq!(
            observed.mix_status,
            MixStatus::Ready,
            "seek must retain preparation"
        );
        assert_eq!(observed.mix_region, Some(region));
        wait(&session, revision, |snapshot| {
            snapshot
                .position
                .is_some_and(|position| (position - seconds).abs() < 0.3)
        });
    }
    let revision = session
        .submit(Command::Seek {
            generation: ready.generation.wrapping_sub(1),
            seconds: 12.0,
        })
        .unwrap();
    let observed = wait(&session, revision, |_| true);
    assert_eq!(observed.mix_status, MixStatus::Ready);
    assert_eq!(observed.current, Some(0));
    assert_eq!(observed.mix_region, Some(region));
    session
        .submit(Command::Seek {
            generation: ready.generation,
            seconds: region.start - 0.4,
        })
        .unwrap();
    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |snapshot| {
        matches!(
            snapshot.mix_status,
            MixStatus::Crossfading | MixStatus::TempoMixing | MixStatus::BeatMixing
        )
    });
    let revision = session
        .submit(Command::Seek {
            generation: ready.generation,
            seconds: 2.0,
        })
        .unwrap();
    let rearmed = wait(&session, revision, |snapshot| {
        snapshot.mix_status == MixStatus::Ready
    });
    assert_eq!(
        rearmed.current,
        Some(0),
        "seeking out of overlap must not advance the queue"
    );
    assert_eq!(rearmed.mix_region, Some(region));
    let revision = session.submit(Command::SetMix(false)).unwrap();
    let disabled = wait(&session, revision, |snapshot| {
        snapshot.mix_status == MixStatus::Off
    });
    assert_eq!(disabled.mix_region, None);
    close(session);
}

/// Disabling MIX during overlap restores current playback; shuffle lookahead is the same occurrence Next adopts.
#[test]
fn mix_off_during_overlap_and_shuffle_lookahead_are_authoritative() {
    use resolibre_audio::session::{MixStatus, RhythmStatus};
    let fixtures = [
        Fixture::rhythmic(120.0),
        Fixture::rhythmic(124.0),
        Fixture::rhythmic(120.0),
    ];
    let session = Session::start(AudioOutput::Silent).unwrap();
    session.submit(Command::SetMix(true)).unwrap();
    session
        .submit(Command::Queue(QueueAction::SetShuffle(true)))
        .unwrap();
    let revision = session
        .submit(Command::Open(
            fixtures.iter().map(|item| item.path.clone()).collect(),
        ))
        .unwrap();
    let prepared = wait(&session, revision, |snapshot| {
        snapshot.next_rhythm.is_some() && snapshot.mix_status == MixStatus::Ready
    });
    let expected = prepared.next_rhythm.as_ref().unwrap().queue_id;
    let revision = session.submit(Command::Next).unwrap();
    let current = wait(&session, revision, |snapshot| {
        snapshot
            .current
            .is_some_and(|index| snapshot.tracks[index].id == expected)
            && matches!(snapshot.rhythm, RhythmStatus::Ready(_))
            && snapshot.mix_status == MixStatus::Ready
    });
    let revision = session
        .submit(Command::Seek {
            generation: current.generation,
            seconds: 10.0,
        })
        .unwrap();
    wait(&session, revision, |snapshot| {
        matches!(
            snapshot.mix_status,
            MixStatus::BeatMixing | MixStatus::TempoMixing | MixStatus::Crossfading
        )
    });
    let revision = session.submit(Command::SetMix(false)).unwrap();
    let off = wait(&session, revision, |snapshot| {
        snapshot.mix_status == MixStatus::Off
            && snapshot.next_rhythm.is_none()
            && snapshot.mix_beat_ratio.is_none()
    });
    assert_eq!(off.current, current.current);
    assert_eq!(off.generation, current.generation);
    assert!(matches!(off.rhythm, RhythmStatus::Ready(_)));
    assert_eq!(off.state, SessionState::Playing);
    let revision = session.submit(Command::Stop).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Stopped
    });
    close(session);
}

/// Actual session restart restores both BPM and on-demand plots from the same explicitly scoped cache.
#[test]
fn persistent_analysis_survives_session_restart_and_serves_automatic_rhythm() {
    use resolibre_audio::session::{AnalysisStatus, RhythmStatus};
    let fixture = Fixture::rhythmic(120.0);
    let cache_directory = fixture.directory.join("analysis");
    let session =
        Session::start_with_analysis_cache(AudioOutput::Silent, Some(cache_directory.clone()))
            .unwrap();
    let revision = session
        .submit(Command::Open(vec![fixture.path.clone()]))
        .unwrap();
    wait(&session, revision, |snapshot| {
        matches!(snapshot.rhythm, RhythmStatus::Ready(_))
    });
    let revision = session
        .submit(Command::Analyze {
            generation: session.snapshot().generation,
        })
        .unwrap();
    let completed = wait(&session, revision, |snapshot| {
        matches!(snapshot.analysis, AnalysisStatus::Complete(_))
    });
    let AnalysisStatus::Complete(original) = completed.analysis else {
        panic!("complete analysis")
    };
    drop(session);
    assert!(cache_directory.join("analysis.sqlite3").is_file());
    let restarted =
        Session::start_with_analysis_cache(AudioOutput::Silent, Some(cache_directory)).unwrap();
    let revision = restarted
        .submit(Command::Open(vec![fixture.path.clone()]))
        .unwrap();
    let ready = wait(&restarted, revision, |snapshot| {
        matches!(snapshot.rhythm, RhythmStatus::Ready(_))
    });
    let RhythmStatus::Ready(rhythm) = ready.rhythm else {
        panic!("automatic cached rhythm")
    };
    assert_eq!(*rhythm, original.tempo);
    let revision = restarted
        .submit(Command::ViewAnalysis {
            generation: restarted.snapshot().generation,
        })
        .unwrap();
    let ready = wait(&restarted, revision, |snapshot| {
        matches!(snapshot.analysis, AnalysisStatus::Complete(_))
    });
    let AnalysisStatus::Complete(restored) = ready.analysis else {
        panic!("cached spectrogram")
    };
    assert_eq!(restored.power_db, original.power_db);
    drop(restarted);
}

/// Whole-library work includes silent/nonplaying rows and continues past missing inputs without transport edits.
#[test]
fn library_analysis_reports_each_occurrence_and_continues_after_failure() {
    use resolibre_audio::session::LibraryAnalysisStatus as Status;
    let first = Fixture::new(2);
    let second = Fixture::rhythmic(120.0);
    let session = Session::start_with_analysis_cache(
        AudioOutput::Silent,
        Some(first.directory.join("analysis")),
    )
    .unwrap();
    let revision = session
        .submit(Command::Add(vec![
            first.path.clone(),
            first.directory.join("missing.flac"),
            second.path.clone(),
        ]))
        .unwrap();
    let before = wait(&session, revision, |snapshot| snapshot.tracks.len() == 3);
    let revision = session.submit(Command::AnalyzeAll).unwrap();
    let after = wait(&session, revision, |snapshot| {
        snapshot.analysis_pending == 0 && snapshot.library_analysis.len() == 3
    });
    let ids: Vec<_> = before.tracks.iter().map(|track| track.id).collect();
    assert_eq!(after.library_analysis[&ids[0]], Status::Complete);
    assert_eq!(after.library_analysis[&ids[1]], Status::Failed);
    assert_eq!(after.library_analysis[&ids[2]], Status::Complete);
    assert_eq!(after.state, before.state);
    assert_eq!(after.current, before.current);
    assert_eq!(after.generation, before.generation);
    // A failure is retryable under the same occurrence ID when the source becomes available.
    fs::copy(&first.path, first.directory.join("missing.flac")).unwrap();
    let revision = session
        .submit(Command::AnalyzeTrack { id: ids[1] })
        .unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.analysis_pending == 0
            && snapshot.library_analysis.get(&ids[1]) == Some(&Status::Complete)
    });
    let revision = session
        .submit(Command::AnalyzeTrack { id: u64::MAX })
        .unwrap();
    let after = wait(&session, revision, |_| true);
    assert_eq!(
        after.library_analysis.len(),
        3,
        "unknown IDs cannot open media"
    );
    close(session);
}

/// Removing a running occurrence and cancelling a batch cannot publish stale badges or change playback.
#[test]
fn library_analysis_removal_cancellation_and_navigation_are_isolated() {
    use resolibre_audio::session::LibraryAnalysisStatus as Status;
    let fixture = Fixture::new(120);
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Open(vec![
            fixture.path.clone(),
            fixture.path.clone(),
        ]))
        .unwrap();
    let before = wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Playing
    });
    let current = before.tracks[0].id;
    let other = before.tracks[1].id;
    let revision = session.submit(Command::AnalyzeTrack { id: other }).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.library_analysis.get(&other) == Some(&Status::Running)
    });
    let revision = session
        .submit(Command::Queue(QueueAction::Remove(other)))
        .unwrap();
    let after = wait(&session, revision, |snapshot| {
        snapshot.analysis_pending == 0 && !snapshot.library_analysis.contains_key(&other)
    });
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.tracks[after.current.unwrap()].id, current);
    assert_eq!(after.state, SessionState::Playing);
    let revision = session.submit(Command::AnalyzeAll).unwrap();
    wait(&session, revision, |snapshot| snapshot.analysis_pending > 0);
    let revision = session.submit(Command::CancelAllAnalysis).unwrap();
    let after = wait(&session, revision, |snapshot| {
        snapshot.analysis_pending == 0
    });
    assert!(after.library_analysis.is_empty());
    assert_eq!(after.state, SessionState::Playing);
    close(session);
}

/// Playback preempts library decoding and measures the full current timeline with MIX off before resuming plots.
#[test]
fn current_track_rhythm_and_plot_preempt_library_work() {
    use resolibre_audio::session::{LibraryAnalysisStatus as Status, RhythmStatus};
    let background = Fixture::new(600);
    let mut current = Fixture::new(128);
    for frame in 0..128 * 8000 {
        let seconds = frame as f64 / 8000.0;
        let bpm = if seconds < 64.0 { 120.0 } else { 140.0 };
        let phase = (seconds - 0.25).rem_euclid(60.0 / bpm);
        let value = if seconds >= 0.25 && phase < 0.035 {
            ((std::f64::consts::TAU * 900.0 * phase).sin() * (-phase * 110.0).exp() * 12000.0)
                as i16
        } else {
            0
        };
        current.bytes[44 + frame * 2..46 + frame * 2].copy_from_slice(&value.to_le_bytes());
    }
    fs::write(&current.path, &current.bytes).unwrap();
    let session = Session::start(AudioOutput::Silent).unwrap();
    let revision = session
        .submit(Command::Add(vec![
            background.path.clone(),
            current.path.clone(),
        ]))
        .unwrap();
    let added = wait(&session, revision, |snapshot| snapshot.tracks.len() == 2);
    let background_id = added.tracks[0].id;
    let current_id = added.tracks[1].id;
    let revision = session.submit(Command::AnalyzeAll).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.library_analysis.get(&background_id) == Some(&Status::Running)
    });
    let revision = session
        .submit(Command::Queue(QueueAction::Select(current_id)))
        .unwrap();
    let ready = wait(&session, revision, |snapshot| {
        if snapshot.library_analysis.get(&current_id) == Some(&Status::Running) {
            let RhythmStatus::Ready(report) = &snapshot.rhythm else {
                panic!("current BPM must precede plots");
            };
            assert!(
                report
                    .segments
                    .iter()
                    .any(|segment| segment.end_seconds > 100.0),
                "current timeline must extend beyond the excerpt"
            );
        }
        matches!(&snapshot.rhythm, RhythmStatus::Ready(report) if report.segments.iter().any(|segment| segment.end_seconds > 100.0))
    });
    assert!(!ready.mix_enabled);
    let RhythmStatus::Ready(report) = ready.rhythm else {
        unreachable!()
    };
    assert!(report.variable);
    assert!((report.bpm_at(10.0).unwrap() - 120.0).abs() < 1.0);
    assert!((report.bpm_at(100.0).unwrap() - 140.0).abs() < 1.0);
    let completed = wait(&session, revision, |snapshot| {
        snapshot.library_analysis.get(&current_id) == Some(&Status::Complete)
    });
    assert_eq!(completed.current, Some(1));
    assert_eq!(completed.state, SessionState::Playing);
    assert_ne!(
        completed.library_analysis.get(&background_id),
        Some(&Status::Complete),
        "current plot must overtake the old library job"
    );
    close(session);
}

/// Natural playback must enter an overlap before EOF, without a seek command priming either stream.
#[test]
fn mix_starts_during_natural_playback_without_seeking() {
    use resolibre_audio::session::MixStatus;
    let fixtures = [Fixture::rhythmic(120.0), Fixture::rhythmic(124.0)];
    let session = Session::start(AudioOutput::Silent).unwrap();
    session
        .submit(Command::Edges(
            resolibre_core::playback_edges::EdgeAction::ToggleTrim,
        ))
        .unwrap();
    session
        .submit(Command::Queue(resolibre_core::queue::QueueAction::SetShuffle(
            true,
        )))
        .unwrap();
    session.submit(Command::SetMix(true)).unwrap();
    session
        .submit(Command::Open(
            fixtures
                .iter()
                .map(|fixture| fixture.path.clone())
                .collect(),
        ))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut ready = false;
    let mut overlapped = false;
    loop {
        let snapshot = session.snapshot();
        ready |= matches!(
            snapshot.mix_status,
            MixStatus::Ready | MixStatus::ReadyCrossfade
        );
        overlapped |= matches!(
            snapshot.mix_status,
            MixStatus::BeatMixing | MixStatus::TempoMixing | MixStatus::Crossfading
        );
        if snapshot.current == Some(1) {
            assert!(ready, "transition must prepare");
            assert!(overlapped, "READY must lead to overlap before natural EOF");
            assert!(
                snapshot.position.is_some_and(|position| position > 2.0),
                "handover must adopt the playing stream"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "natural mix deadline: {:?}",
            snapshot.mix_status
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    close(session);
}

/// Analyze All after restart reuses a valid full report, while stale input still needs a new hash.
#[test]
fn library_batch_reuses_persistent_full_analysis_after_restart() {
    use resolibre_audio::session::LibraryAnalysisStatus;
    let fixture = Fixture::new(2);
    let directory = fixture.directory.join("batch-cache");
    let session =
        Session::start_with_analysis_cache(AudioOutput::Silent, Some(directory.clone())).unwrap();
    let revision = session
        .submit(Command::Add(vec![fixture.path.clone()]))
        .unwrap();
    wait(&session, revision, |snapshot| snapshot.tracks.len() == 1);
    let revision = session.submit(Command::AnalyzeAll).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.analysis_pending == 0
            && snapshot
                .library_analysis
                .values()
                .any(|status| *status == LibraryAnalysisStatus::Complete)
    });
    close(session);
    let identity = resolibre_storage::audio_identity::identify(
        fs::File::open(&fixture.path).unwrap(),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let mut database = resolibre_storage::AnalysisCache::open(&directory).unwrap();
    let mut stored = database.track(&identity).unwrap().unwrap();
    stored.power_db.fill(-42.0);
    database.save_track(&identity, &stored).unwrap();
    drop(database);
    let restarted =
        Session::start_with_analysis_cache(AudioOutput::Silent, Some(directory.clone())).unwrap();
    let revision = restarted
        .submit(Command::Add(vec![
            fixture.path.clone(),
            fixture.path.clone(),
        ]))
        .unwrap();
    wait(&restarted, revision, |snapshot| snapshot.tracks.len() == 2);
    let revision = restarted.submit(Command::AnalyzeAll).unwrap();
    wait(&restarted, revision, |snapshot| {
        snapshot.analysis_pending == 0
            && snapshot.library_analysis.len() == 2
            && snapshot
                .library_analysis
                .values()
                .all(|status| *status == LibraryAnalysisStatus::Complete)
    });
    close(restarted);
    let database = resolibre_storage::AnalysisCache::open(&directory).unwrap();
    assert_eq!(
        database.track(&identity).unwrap().unwrap().power_db,
        stored.power_db,
        "batch cache hits must not decode and overwrite valid reports"
    );
}

/// Disabling MIX while paused in the outgoing approach must restore normal speed before resume.
#[test]
fn mix_off_while_paused_before_overlap_restores_tempo() {
    use resolibre_audio::session::MixStatus;

    let fixtures = [Fixture::rhythmic(120.0), Fixture::rhythmic(142.0)];
    let session = Session::start(AudioOutput::Silent).unwrap();
    session.submit(Command::SetMix(true)).unwrap();
    let revision = session
        .submit(Command::Open(
            fixtures
                .iter()
                .map(|fixture| fixture.path.clone())
                .collect(),
        ))
        .unwrap();
    let ready = wait(&session, revision, |snapshot| {
        snapshot.mix_status == MixStatus::Ready
    });
    let overlap_start = ready.mix_region.unwrap().start;
    let revision = session
        .submit(Command::Seek {
            generation: ready.generation,
            seconds: overlap_start - 2.2,
        })
        .unwrap();
    let approaching = wait(&session, revision, |snapshot| {
        snapshot
            .position
            .is_some_and(|position| position > overlap_start - 2.0)
    });
    assert_eq!(approaching.mix_status, MixStatus::Ready);

    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Paused
    });
    let revision = session.submit(Command::SetMix(false)).unwrap();
    let disabled = wait(&session, revision, |snapshot| {
        snapshot.mix_status == MixStatus::Off
    });
    assert_eq!(disabled.state, SessionState::Paused);
    assert_eq!(disabled.current, Some(0));
    assert!(disabled.mix_region.is_none());

    let revision = session.submit(Command::TogglePause).unwrap();
    wait(&session, revision, |snapshot| {
        snapshot.state == SessionState::Playing
    });
    std::thread::sleep(Duration::from_millis(300));
    let position = session.snapshot().position.unwrap();
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(3));
    let elapsed = started.elapsed().as_secs_f64();
    let resumed = session.snapshot();
    let moved = resumed.position.unwrap() - position;
    assert!(
        (moved - elapsed).abs() < 0.07,
        "tempo after MIX-off: {moved} in {elapsed}"
    );
    assert_eq!(resumed.current, Some(0));
    close(session);
}
