//! Test-only command worker. The parent owns its private socket and deadline.

#[path = "session_probe_tests.rs"]
mod session_probe;

use super::{AudioOutput, LocalPlayer};
use resolibre_core::{PlaybackEvent, SeekPosition};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::time::{Duration, Instant};

/// Linux x86_64 timespec; this probe has the same platform restriction as the adapter.
#[repr(C)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

/// glibc mallinfo2 layout from the reference system's malloc.h, test-only.
#[repr(C)]
struct AllocatorInfo {
    arena: usize,
    free_chunks: usize,
    fast_chunks: usize,
    mapped_chunks: usize,
    mapped_bytes: usize,
    unused: usize,
    fast_free_bytes: usize,
    allocated_bytes: usize,
    free_bytes: usize,
    releasable_top_bytes: usize,
}

unsafe extern "C" {
    /// Writes CLOCK_MONOTONIC time into the caller's live timespec.
    fn clock_gettime(clock: i32, value: *mut Timespec) -> i32;
    /// Returns glibc allocation counters by value without exposing addresses.
    fn mallinfo2() -> AllocatorInfo;
    /// Diagnostic only: ask glibc to release free pages; never used by production.
    fn malloc_trim(pad: usize) -> i32;
}

/// Uses the Linux monotonic clock also selected by Python's monotonic_ns.
fn now_ns() -> u64 {
    let mut value = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    // SAFETY: Linux x86_64 layout, valid aligned storage, no retained pointer.
    assert_eq!(unsafe { clock_gettime(1, &mut value) }, 0);
    value.seconds as u64 * 1_000_000_000 + value.nanoseconds as u64
}

/// Closed operations keep malformed traffic away from native command strings.
#[derive(Debug)]
enum Action {
    Equalizer(f64),
    Edges(resolibre_core::playback_edges::PlaybackEdges),
    EdgesReady,
    Open,
    Pause(bool),
    Seek(SeekPosition),
    Drop,
    Quit,
    Memory,
    Trim,
}

/// Validates one complete packet before changing any player state.
fn parse(packet: &[u8]) -> Option<(u64, Action)> {
    if packet.len() > 64 {
        return None;
    }
    let text = std::str::from_utf8(packet).ok()?;
    let mut fields = text.split_ascii_whitespace();
    let sequence = fields.next()?.parse::<u64>().ok()?;
    let action = match fields.next()? {
        "eq" => {
            let db: f64 = fields.next()?.parse().ok()?;
            if !db.is_finite() || !(-12.0..=12.0).contains(&db) {
                return None;
            }
            Action::Equalizer(db)
        }
        "edges" => {
            use resolibre_core::playback_edges::{FadeMode, PlaybackEdges};
            let trim = match fields.next()? {
                "0" => false,
                "1" => true,
                _ => return None,
            };
            let fade = match fields.next()? {
                "0" => FadeMode::Off,
                "1" => FadeMode::In,
                "2" => FadeMode::Out,
                "3" => FadeMode::Both,
                _ => return None,
            };
            Action::Edges(PlaybackEdges { trim, fade })
        }
        "edges-ready" => Action::EdgesReady,
        "open" => Action::Open,
        "pause" => Action::Pause(true),
        "resume" => Action::Pause(false),
        "seek" => {
            let seconds = fields.next()?.parse::<f64>().ok()?;
            Action::Seek(SeekPosition::from_seconds(seconds)?)
        }
        "drop" => Action::Drop,
        "quit" => Action::Quit,
        "memory" => Action::Memory,
        "trim" => Action::Trim,
        _ => return None,
    };
    if fields.next().is_some() || sequence == 0 {
        return None;
    }
    Some((sequence, action))
}

/// Executes on the owning thread; errors become numeric protocol outcomes only.
fn apply(player: &mut Option<LocalPlayer>, action: &Action) -> bool {
    match action {
        Action::Equalizer(_) | Action::Edges(_) | Action::EdgesReady => false,
        Action::Memory => true,
        Action::Trim => {
            // SAFETY: glibc owns all affected free pages; no pointers enter or escape.
            unsafe { malloc_trim(0) };
            true
        }
        Action::Open => {
            if player.is_some() {
                return false;
            }
            match LocalPlayer::open(Path::new("private-input.wav"), AudioOutput::PrivatePulse) {
                Ok(opened) => {
                    *player = Some(opened);
                    true
                }
                Err(_) => false,
            }
        }
        Action::Drop | Action::Quit => {
            *player = None;
            true
        }
        Action::Pause(paused) => {
            let Some(active) = player else { return false };
            active.set_paused(*paused).is_ok()
        }
        Action::Seek(position) => {
            let Some(active) = player else { return false };
            active.seek(*position).is_ok()
        }
    }
}

/// Reports EOF and failure separately, and verifies terminal format clearing.
fn poll(player: &mut Option<LocalPlayer>, socket: &UnixDatagram) -> io::Result<()> {
    let Some(active) = player else {
        std::thread::sleep(Duration::from_millis(1));
        return Ok(());
    };
    let terminal = match active.poll_event() {
        Ok(Some(PlaybackEvent::Ended)) => Some("eof"),
        Err(_) => Some("error"),
        _ => None,
    };
    if let Some(event) = terminal {
        assert_eq!(active.audio_formats().ok(), Some(Default::default()));
        // Destroy before announcing terminal completion. No stale events survive.
        *player = None;
        socket.send(format!("event {event} {}", now_ns()).as_bytes())?;
    }
    Ok(())
}

/// Processes at most 64 commands per turn; replay/oversize packets cannot execute.
fn serve(socket: &UnixDatagram) -> io::Result<()> {
    let mut player = None;
    let mut last_sequence = 0;
    let deadline = Instant::now() + Duration::from_secs(90);
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
            let success = apply(&mut player, &action);
            let returned = now_ns();
            if matches!(action, Action::Memory) {
                // SAFETY: the reference glibc ABI returns this repr(C) struct by value.
                let info = unsafe { mallinfo2() };
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
        poll(&mut player, socket)?;
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "worker deadline"))
}

/// Runs only inside the parent-owned 0700 fixture and connected local socket pair.
#[test]
#[ignore = "run through python3 tools/check_playback_responsiveness.py"]
fn control_fixture() -> io::Result<()> {
    assert_eq!(std::fs::metadata(".")?.permissions().mode() & 0o777, 0o700);
    let socket = UnixDatagram::bind("worker.sock")?;
    socket.connect("controller.sock")?;
    socket.set_nonblocking(true)?;
    if std::env::var_os("RESOUL_SESSION_PROBE").is_some() {
        session_probe::serve(&socket)
    } else {
        serve(&socket)
    }
}

/// Invalid numbers, extra fields, oversized packets and native syntax are rejected.
#[test]
fn rejects_invalid_control_packets() {
    for packet in [
        b"1 seek NaN".as_slice(),
        b"1 seek inf",
        b"1 eq NaN",
        b"1 eq 13",
        b"1 eq 2 extra",
        b"1 edges 2 0",
        b"1 edges 1 4",
        b"1 edges 1 -1",
        b"1 edges 1 0 extra",
        b"1 seek -1",
        b"0 pause",
        b"1 open extra",
        b"1 loadfile secret",
        b"\xff",
        &[b'x'; 65],
    ] {
        assert!(parse(packet).is_none());
    }
    assert!(matches!(parse(b"7 seek 103"), Some((7, Action::Seek(_)))));
}
