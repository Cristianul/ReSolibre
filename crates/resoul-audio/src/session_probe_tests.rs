//! Drives the production session through the existing private marker/monitor protocol.

use super::{Action, AllocatorInfo, mallinfo2, now_ns, parse};
use crate::{
    AudioOutput,
    session::{Command, Session},
};
use resolibre_core::session::{SessionState, Volume};
use std::io;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Waits for completed Stop cleanup rather than acknowledging only the queued request.
fn stop(session: &Session) -> bool {
    let Ok(revision) = session.submit(Command::Stop) else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let snapshot = session.snapshot();
        if snapshot.revision >= revision && snapshot.state == SessionState::Stopped {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

/// Maps the closed probe vocabulary to production typed session commands.
fn apply(session: &Session, action: &Action) -> bool {
    let snapshot = session.snapshot();
    let command = match action {
        Action::EdgesReady => {
            let expected = if snapshot.edges.needs_analysis() {
                crate::session::EdgeStatus::Ready
            } else {
                crate::session::EdgeStatus::Off
            };
            return snapshot.edge_status == expected && snapshot.state == SessionState::Playing;
        }
        Action::Equalizer(db) => {
            let mut bands = [0.0; 10];
            bands[5] = *db;
            let settings = resolibre_core::equalizer::Equalizer::new(true, bands, 0.0)
                .expect("validated probe gain");
            Command::Equalizer(resolibre_core::equalizer::EqAction::Load(settings))
        }
        Action::Edges(settings) => {
            Command::Edges(resolibre_core::playback_edges::EdgeAction::Set(*settings))
        }
        Action::Open => Command::Open(vec![PathBuf::from("private-input.wav")]),
        Action::Pause(paused) => {
            if !matches!(
                snapshot.state,
                SessionState::Playing | SessionState::Paused | SessionState::Loading
            ) {
                return false;
            }
            if (*paused && snapshot.state == SessionState::Paused)
                || (!*paused && snapshot.state == SessionState::Playing)
            {
                return true;
            }
            Command::TogglePause
        }
        Action::Seek(position) => Command::Seek {
            generation: snapshot.generation,
            seconds: position.seconds(),
        },
        Action::Drop | Action::Quit => return stop(session),
        Action::Memory => return true,
        Action::Trim => return false,
    };
    session.submit(command).is_ok()
}

/// Uses the same process/packet/deadline bounds as the retained direct adapter probe.
pub(super) fn serve(socket: &UnixDatagram) -> io::Result<()> {
    let cache = if std::env::var_os("RESOUL_PROBE_CACHE").is_some() {
        Some(std::env::current_dir()?.join("analysis-cache"))
    } else {
        None
    };
    let session = Session::start_with_analysis_cache(AudioOutput::PrivatePulse, cache)?;
    session
        .submit(Command::Volume(Volume::from_percent(100.0).expect("unity")))
        .expect("initial control");
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut last_sequence = 0;
    let mut terminal_generation = 0;
    let mut analyzed_generation = 0;
    let analyze = std::env::var_os("RESOUL_PROBE_ANALYSIS").is_some();
    while Instant::now() < deadline {
        for _ in 0..64 {
            let mut packet = [0_u8; 65];
            let size = match socket.recv(&mut packet) {
                Ok(size) => size,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            };
            let Some((sequence, action)) = parse(&packet[..size]) else {
                socket.send(b"reject")?;
                continue;
            };
            if sequence <= last_sequence {
                socket.send(b"reject")?;
                continue;
            }
            last_sequence = sequence;
            let entered = now_ns();
            let success = apply(&session, &action);
            let returned = now_ns();
            if matches!(action, Action::Memory) {
                // SAFETY: glibc returns the documented repr(C) counters by value.
                let info: AllocatorInfo = unsafe { mallinfo2() };
                socket.send(
                    format!(
                        "memory {} {} {} {}",
                        info.arena, info.allocated_bytes, info.free_bytes, info.mapped_bytes
                    )
                    .as_bytes(),
                )?;
            }
            socket.send(
                format!("ack {sequence} {entered} {returned} {}", u8::from(success)).as_bytes(),
            )?;
            if matches!(action, Action::Quit) {
                return Ok(());
            }
        }
        let snapshot = session.snapshot();
        if analyze
            && snapshot.state == SessionState::Playing
            && snapshot.generation != analyzed_generation
        {
            analyzed_generation = snapshot.generation;
            session
                .submit(Command::Analyze {
                    generation: snapshot.generation,
                })
                .expect("analysis intent");
        }
        if analyze
            && matches!(
                snapshot.analysis,
                crate::session::AnalysisStatus::Unavailable
            )
        {
            return Err(io::Error::other("probe analysis failed"));
        }
        if snapshot.generation != terminal_generation
            && matches!(snapshot.state, SessionState::Ended | SessionState::Failed)
        {
            terminal_generation = snapshot.generation;
            let event = if snapshot.state == SessionState::Ended {
                "eof"
            } else {
                "error"
            };
            socket.send(format!("event {event} {}", now_ns()).as_bytes())?;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "worker deadline"))
}
