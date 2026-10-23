//! Real Windows/libmpv regression checks with owned WAV fixtures and a silent output device.

use crate::{AudioOutput, LocalPlayer, original_audio};
use resolibre_core::session::Volume;
use resolibre_core::{PlaybackEvent, SeekPosition};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
    bytes: Vec<u8>,
}

impl Fixture {
    /// Creates a three-minute stereo source to exercise sustained PCM transport, not just headers.
    fn long_stereo() -> Self {
        let mut fixture = Self::new();
        let size = 180 * 48000 * 4_u32;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt \x10\0\0\0\x01\0\x02\0");
        bytes.extend(48000_u32.to_le_bytes());
        bytes.extend(192000_u32.to_le_bytes());
        bytes.extend(b"\x04\0\x10\0data");
        bytes.extend(size.to_le_bytes());
        bytes.extend([0xcd, 0x0c, 0x33, 0xf3].repeat(size as usize / 4));
        fs::write(&fixture.path, &bytes).unwrap();
        fixture.bytes = bytes;
        fixture
    }

    /// Creates ten seconds of owned mono PCM, including Unicode and spaces in the file name.
    fn new() -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "resolibre-win-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("Muzică test.wav");
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(160036_u32.to_le_bytes());
        bytes.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
        bytes.extend(8000_u32.to_le_bytes());
        bytes.extend(16000_u32.to_le_bytes());
        bytes.extend(b"\x02\0\x10\0data");
        bytes.extend(160000_u32.to_le_bytes());
        for index in 0..80000 {
            let sample =
                ((f64::from(index) * std::f64::consts::TAU * 440.0 / 8000.0).sin() * 1000.0) as i16;
            bytes.extend(sample.to_le_bytes());
        }
        fs::write(&path, &bytes).unwrap();
        Self {
            directory,
            path,
            bytes,
        }
    }
}

impl Drop for Fixture {
    /// Removes only the uniquely owned temporary fixture after native handles have closed.
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

/// Real libmpv reads a retained Unicode input, seeks while paused, and denies concurrent music writes.
#[test]
fn playback_retains_read_only_unicode_file() {
    let fixture = Fixture::new();
    let modified = fs::metadata(&fixture.path).unwrap().modified().unwrap();
    let mut player = LocalPlayer::open_for_session(
        &fixture.path,
        AudioOutput::Silent,
        true,
        Volume::from_percent(25.0).unwrap(),
    )
    .unwrap();
    let started = Instant::now();
    while player.try_event().unwrap() != Some(PlaybackEvent::Loaded) {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_err()
    );
    player
        .seek(SeekPosition::from_seconds(0.25).unwrap())
        .unwrap();
    player.set_paused(false).unwrap();
    drop(player);
    assert_eq!(fs::read(&fixture.path).unwrap(), fixture.bytes);
    assert_eq!(
        fs::metadata(&fixture.path).unwrap().modified().unwrap(),
        modified
    );
}

/// Named-pipe decoding preserves source rate/channels and completes without modifying the original.
#[test]
fn native_analysis_uses_bounded_pipe() {
    let fixture = Fixture::new();
    let file = super::open_media(&fixture.path, false).unwrap();
    let key = original_audio::SourceKey::read(&file, u32::MAX).unwrap();
    let cancel = AtomicBool::new(false);
    let duration = Duration::from_secs(5);
    let report = original_audio::native::decode(file, key, &cancel, duration, |reader, started| {
        assert_eq!(reader.expected_format(), (8000, 1));
        original_audio::pcm::stream(reader, &cancel, started, duration, |rate, channels, _| {
            assert_eq!((rate, channels), (8000, 1));
            Ok(())
        })
    })
    .unwrap();
    assert_eq!(report, (8000, 80000));
    assert_eq!(fs::read(&fixture.path).unwrap(), fixture.bytes);
}

/// A normal stereo song must drain through the named pipe within the rhythm worker's deadline.
#[test]
fn native_stereo_analysis_meets_rhythm_deadline() {
    let fixture = Fixture::long_stereo();
    let file = super::open_media(&fixture.path, false).unwrap();
    let key = original_audio::SourceKey::read(&file, u32::MAX).unwrap();
    let cancel = AtomicBool::new(false);
    let deadline = Duration::from_secs(10);
    let started = Instant::now();
    let result = original_audio::native::decode(file, key, &cancel, deadline, |reader, start| {
        original_audio::pcm::stream(reader, &cancel, start, deadline, |_, _, _| Ok(()))
    });
    eprintln!("stereo native decode elapsed: {:?}", started.elapsed());
    assert_eq!(result.unwrap(), (48000, 180 * 48000));
}

/// Edge analysis must receive a full stereo song before the helper transport consumes its time budget.
#[test]
fn helper_stereo_analysis_meets_deadline() {
    let fixture = Fixture::long_stereo();
    let file = super::open_media(&fixture.path, false).unwrap();
    let key = original_audio::SourceKey::read(&file, 0).unwrap();
    let cancel = AtomicBool::new(false);
    let deadline = Duration::from_secs(10);
    let started = Instant::now();
    let result = original_audio::decode(file, key, &cancel, deadline, |reader, start| {
        original_audio::pcm::decode(reader, &cancel, start, deadline)
    });
    eprintln!("stereo helper decode elapsed: {:?}", started.elapsed());
    assert_eq!(result.unwrap().duration, 180.0);
}

/// Bundled ffprobe receives only the retained stdin handle; Unicode source paths never enter arguments.
#[test]
fn metadata_helper_reads_retained_input() {
    let fixture = Fixture::new();
    let mut job = crate::metadata::Job::start(fixture.path.clone(), false).unwrap();
    let started = Instant::now();
    let details = loop {
        if let Some(result) = job.poll() {
            break result.unwrap();
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(details.duration_seconds, Some(10));
    assert_eq!(fs::read(&fixture.path).unwrap(), fixture.bytes);
}

/// Named pipes, device names and alternate data streams cannot become ordinary music inputs.
#[test]
fn rejects_devices_and_special_inputs() {
    for path in [
        r"\\.\pipe\untrusted",
        r"\\?\GLOBALROOT\Device\Null",
        "//./pipe/untrusted",
        "NUL",
        "CON.wav",
        "COM1.wav",
        r"C:\music.wav:secret",
    ] {
        assert!(super::open_media(std::path::Path::new(path), false).is_err());
    }
    let fixture = Fixture::new();
    assert!(super::open_media(&fixture.directory, false).is_err());
    let mut reader = super::output_pipe().unwrap().0;
    reader.set_nonblocking(true).unwrap();
    let started = Instant::now();
    use std::io::Read;
    let _ = reader.read(&mut [0; 1]);
    assert!(started.elapsed() < Duration::from_secs(1));
}

/// Prefix and cancelled analysis tear down a potentially blocked native writer within the deadline.
#[test]
fn analysis_prefix_and_cancellation_release_input() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let file = super::open_media(&fixture.path, false).unwrap();
    let key = original_audio::SourceKey::read(&file, u32::MAX).unwrap();
    let started = Instant::now();
    original_audio::native::decode_prefix(
        file,
        key,
        &cancel,
        Duration::from_secs(5),
        |reader, _| {
            use std::io::Read;
            reader.read_exact(&mut [0; 44]).map_err(|_| ())
        },
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    cancel.store(true, Ordering::Relaxed);
    let file = super::open_media(&fixture.path, false).unwrap();
    assert!(
        original_audio::native::decode_prefix(
            file,
            key,
            &cancel,
            Duration::from_secs(5),
            |_, _| Ok(())
        )
        .is_err()
    );
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_ok()
    );
}

/// Data arriving after a read begins must wake it promptly, rather than paying the entire timeout per chunk.
#[test]
fn pipe_wakes_on_data_and_preserves_partial_reads() {
    use std::io::{Read, Write};
    let (mut reader, path) = super::windows::named_pipe().unwrap();
    let mut writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
    reader
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let producer = std::thread::spawn(move || {
        for value in 0..32_u8 {
            receive.recv().unwrap();
            std::thread::sleep(Duration::from_millis(2));
            writer.write_all(&[value, value]).unwrap();
        }
    });
    let started = Instant::now();
    for value in 0..32_u8 {
        send.send(()).unwrap();
        for _ in 0..2 {
            let mut byte = [0];
            loop {
                match reader.read(&mut byte) {
                    Ok(1) => break,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(started.elapsed() < Duration::from_secs(2));
                    }
                    result => panic!("expected pipe data: {result:?}"),
                }
            }
            assert_eq!(byte, [value]);
        }
    }
    producer.join().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "read timeouts must not throttle ready data"
    );
    assert_eq!(reader.read(&mut [0]).unwrap(), 0);
}

/// Dropping either an unconnected listener or an idle pending read cancels kernel I/O before freeing buffers.
#[test]
fn pending_pipe_operations_cancel_without_a_writer_response() {
    use std::io::Read;
    let started = Instant::now();
    let (reader, _) = super::windows::named_pipe().unwrap();
    drop(reader);
    let (mut reader, path) = super::windows::named_pipe().unwrap();
    let writer = fs::OpenOptions::new().write(true).open(path).unwrap();
    reader.set_nonblocking(true).unwrap();
    assert_eq!(
        reader.read(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(reader);
    drop(writer);
    assert!(started.elapsed() < Duration::from_secs(1));
}
