//! Real libmpv integration using generated audio and an explicit silent sink.

use resolibre_audio::{AudioOutput, LocalPlayer, PlaybackError};
use resolibre_core::{AudioFormat, AudioFormats, PlaybackEvent, SampleFormat, SeekPosition};
use std::ffi::OsString;
use std::fs::{self, DirBuilder};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
}

impl Fixture {
    /// Creates an isolated private directory and a read-only caller-owned fixture.
    fn new(contents: &[u8]) -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("resolibre-audio-{}-{sequence}", std::process::id()));
        DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .expect("private fixture directory");

        let path = directory.join("local ü audio.wav");
        fs::write(&path, contents).expect("write owned fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).expect("read-only fixture");
        Self { directory, path }
    }
}

impl Drop for Fixture {
    /// Removes only this test's generated directory after its player has stopped.
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Generates one second of mono PCM silence, avoiding bundled music or codecs.
fn silent_wav() -> Vec<u8> {
    let data_size = 16_000_u32;
    let mut bytes = Vec::with_capacity(44 + data_size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&8000_u32.to_le_bytes());
    bytes.extend_from_slice(&16000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    bytes.resize(44 + data_size as usize, 0);
    bytes
}

/// Generates changing tones with opposite-phase channels to expose stale seeks or destructive mixing.
fn changing_tone_wav(channels: u16) -> Vec<u8> {
    let mut bytes = silent_wav();
    bytes.truncate(44);
    let size = 4 * 16000_u32 * u32::from(channels);
    bytes[4..8].copy_from_slice(&(36 + size).to_le_bytes());
    bytes[22..24].copy_from_slice(&channels.to_le_bytes());
    bytes[28..32].copy_from_slice(&(16000 * u32::from(channels)).to_le_bytes());
    bytes[32..34].copy_from_slice(&(2 * channels).to_le_bytes());
    bytes[40..44].copy_from_slice(&size.to_le_bytes());

    for frame in 0..32000 {
        let frequency = if frame < 16000 { 250.0 } else { 1500.0 };
        let phase = std::f64::consts::TAU * frequency * f64::from(frame) / 8000.0;
        let sample = (phase.sin() * 8000.0) as i16;
        for channel in 0..channels {
            let channel_sample = if channel % 2 == 0 { sample } else { -sample };
            bytes.extend_from_slice(&channel_sample.to_le_bytes());
        }
    }
    bytes
}

/// Waits for measured non-silent bands while draining native events and enforcing a real-time deadline.
fn wait_spectrum(player: &mut LocalPlayer) -> resolibre_core::spectrum::live::LiveLevels {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        player.try_event().unwrap();
        if let Some(levels) = player.live_spectrum().expect("live analysis available")
            && levels.levels.iter().any(|value| *value > 0.5)
        {
            return levels;
        }
        assert!(Instant::now() < deadline, "no real spectrum data");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A seek changes measured frequency, pause freezes it, and repeated engines never exhaust native slots.
#[test]
fn live_spectrum_tracks_seek_pause_and_replacement() {
    for cycle in 0..18 {
        let channels = [1, 2, 6][cycle % 3];
        let fixture = Fixture::new(&changing_tone_wav(channels));
        let mut player = LocalPlayer::open(&fixture.path, AudioOutput::Silent).unwrap();
        wait_for(&mut player, PlaybackEvent::Loaded);
        let initial = wait_spectrum(&mut player);
        let low_peak = initial
            .levels
            .iter()
            .position(|level| *level > 0.5)
            .unwrap();
        player
            .seek(SeekPosition::from_seconds(2.5).unwrap())
            .unwrap();
        let after = wait_spectrum(&mut player);
        let high_peak = after.levels.iter().position(|level| *level > 0.5).unwrap();
        assert!(after.epoch > initial.epoch);
        assert!(
            high_peak > low_peak + 4,
            "old low-frequency samples survived seek"
        );
        player.set_paused(true).unwrap();
        let frozen = player.live_spectrum().unwrap().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(player.live_spectrum().unwrap().unwrap(), frozen);
    }
}

/// Drains events to a particular lifecycle boundary with a test-only deadline.
fn wait_for(player: &mut LocalPlayer, expected: PlaybackEvent) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if player.poll_event().expect("native playback event") == Some(expected) {
            return;
        }
    }

    panic!("playback event deadline exceeded: {expected:?}");
}

/// Decoding and RAII teardown must preserve all original bytes and modification time.
#[test]
fn reaches_eof_without_changing_original() {
    let contents = silent_wav();
    let fixture = Fixture::new(&contents);
    let before = fs::metadata(&fixture.path).expect("fixture metadata");
    let mut player = LocalPlayer::open(&fixture.path, AudioOutput::Silent).expect("open wav");
    assert_eq!(player.stream_info().expect("before load"), None);

    wait_for(&mut player, PlaybackEvent::Loaded);
    wait_for(&mut player, PlaybackEvent::Ended);
    drop(player);

    let after = fs::metadata(&fixture.path).expect("fixture metadata after playback");
    assert_eq!(fs::read(&fixture.path).expect("read original"), contents);
    assert_eq!(
        before.modified().expect("mtime"),
        after.modified().expect("mtime")
    );
    assert_eq!(after.permissions().mode() & 0o777, 0o444);
}

/// Keeps the opened inode even if its non-UTF-8 pathname is replaced during play.
#[test]
fn retains_open_file_when_path_is_replaced() {
    let contents = silent_wav();
    let fixture = Fixture::new(&contents);
    let name = OsString::from_vec(b"local-\xff.wav".to_vec());
    let path = fixture.directory.join(name);
    fs::rename(&fixture.path, &path).expect("create non-UTF-8 filename");

    let mut player = LocalPlayer::open(&path, AudioOutput::Silent).expect("open wav");
    player.set_paused(true).expect("pause before replacement");
    wait_for(&mut player, PlaybackEvent::Loaded);
    fs::rename(&path, &fixture.path).expect("move owned fixture");
    fs::write(&path, b"replacement must not be read").expect("replace fixture path");
    player
        .seek(SeekPosition::from_seconds(0.0).expect("valid start"))
        .expect("seek original");
    player.set_paused(false).expect("resume original");
    wait_for(&mut player, PlaybackEvent::Ended);
    drop(player);

    assert_eq!(
        fs::read(&fixture.path).expect("retained original"),
        contents
    );
    assert_eq!(
        fs::read(&path).expect("replacement"),
        b"replacement must not be read"
    );
}

/// Verifies native pause and seek state; this does not claim audible seek latency.
#[test]
fn pauses_seeks_and_resumes() {
    let fixture = Fixture::new(&silent_wav());
    let mut player = LocalPlayer::open(&fixture.path, AudioOutput::Silent).expect("open wav");
    wait_for(&mut player, PlaybackEvent::Loaded);
    player.set_paused(true).expect("pause");

    let paused_position = player.position().expect("paused position");
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        assert_ne!(
            player.poll_event().expect("paused event"),
            Some(PlaybackEvent::Ended)
        );
    }
    let later_position = player.position().expect("still paused");
    assert!((later_position.seconds() - paused_position.seconds()).abs() < 0.02);

    let target = SeekPosition::from_seconds(0.6).expect("valid target");
    player.seek(target).expect("seek");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        player.poll_event().expect("seek events");
        let position = player.position().expect("seek position");
        if (position.seconds() - target.seconds()).abs() < 0.02 {
            break;
        }
        assert!(Instant::now() < deadline, "seek position deadline exceeded");
    }

    player.set_paused(false).expect("resume");
    wait_for(&mut player, PlaybackEvent::Ended);
}

/// Reads real decoder/output properties and clears readings at lifecycle boundaries.
#[test]
fn reports_formats_only_during_loaded_playback() {
    let fixture = Fixture::new(&silent_wav());
    let mut player = LocalPlayer::open(&fixture.path, AudioOutput::Silent).expect("open wav");
    assert_eq!(
        player.audio_formats().expect("before load"),
        AudioFormats::default()
    );
    player.set_paused(true).expect("pause for inspection");
    wait_for(&mut player, PlaybackEvent::Loaded);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let formats = player.audio_formats().expect("format properties");
        if let (Some(source), Some(output)) = (formats.source, formats.output) {
            assert_eq!(
                source,
                AudioFormat {
                    sample_rate_hz: 8000,
                    channels: 1,
                    sample_format: SampleFormat::Signed16,
                }
            );
            // The null sink accepts the decoded format, but proves no real output.
            assert_eq!(output, source);
            let info = player.stream_info().expect("stream info").expect("loaded");
            assert_eq!(info.codec, resolibre_core::track_info::AudioCodec::Pcm);
            assert_eq!(info.sample_rate_hz, Some(8000));
            assert_eq!(info.channels, Some(1));
            break;
        }
        assert!(Instant::now() < deadline, "format deadline exceeded");
        player.poll_event().expect("initialization events");
    }

    player.set_paused(false).expect("resume");
    wait_for(&mut player, PlaybackEvent::Ended);
    assert_eq!(
        player.audio_formats().expect("after EOF"),
        AudioFormats::default()
    );
    assert_eq!(player.stream_info().expect("after EOF"), None);
    drop(player);
    assert_eq!(fs::read(&fixture.path).expect("original"), silent_wav());
}

/// Actual codecs win over misleading extensions, and optional reads never change sources.
#[test]
fn stream_info_uses_encoded_codec_instead_of_filename() {
    use resolibre_core::track_info::AudioCodec;
    let fixture = Fixture::new(&silent_wav());
    for (encoder, container, filename, expected) in [
        ("flac", "flac", "encoded.mp3", AudioCodec::Flac),
        ("libmp3lame", "mp3", "encoded.flac", AudioCodec::Mp3),
    ] {
        let path = fixture.directory.join(filename);
        let status = std::process::Command::new("ffmpeg")
            .env_remove("FFREPORT")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&fixture.path)
            .args(["-ar", "44100", "-ac", "2", "-c:a", encoder, "-f", container])
            .arg(&path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("fixture encoder");
        assert!(status.success(), "fixture encoding");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let original = fs::read(&path).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut player = LocalPlayer::open(&path, AudioOutput::Silent).unwrap();
        player.set_paused(true).unwrap();
        wait_for(&mut player, PlaybackEvent::Loaded);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let info = player.stream_info().unwrap().unwrap();
            if info.sample_rate_hz.is_some() {
                assert_eq!(info.codec, expected);
                assert_eq!(info.sample_rate_hz, Some(44100));
                assert_eq!(info.channels, Some(2));
                break;
            }
            assert!(Instant::now() < deadline, "stream information deadline");
            player.poll_event().unwrap();
        }
        drop(player);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

/// Native parsers must reject malformed input and disguised local/network playlists.
#[test]
fn rejects_untrusted_non_audio_without_modifying_it() {
    for contents in [
        b"not audio".as_slice(),
        b"RIFF\xff\xff\xff\xffWAVEfmt ",
        b"#EXTM3U\nhttp://127.0.0.1:9/private-token\n",
        b"#EXTM3U\n/etc/passwd\n",
        b"ffconcat version 1.0\nfile /etc/passwd\n",
        b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nhttp://127.0.0.1:9/private-token\n",
    ] {
        let fixture = Fixture::new(contents);
        let mut player =
            LocalPlayer::open(&fixture.path, AudioOutput::Silent).expect("submit file");
        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            match player.poll_event() {
                Err(PlaybackError::Native {
                    operation: "playback",
                    code,
                }) => {
                    assert!(code < 0);
                    break;
                }
                Err(error) => panic!("unexpected category: {error}"),
                Ok(event) => assert_eq!(event, None, "untrusted file must never load"),
            }
            assert!(Instant::now() < deadline, "rejection deadline exceeded");
        }

        drop(player);
        assert_eq!(
            fs::read(&fixture.path).expect("read rejected input"),
            contents
        );
    }
}

/// Directories, missing files, URLs and special files fail before native creation.
#[test]
fn rejects_invalid_local_inputs_with_private_errors() {
    let fixture = Fixture::new(b"");
    for path in [
        fixture.directory.clone(),
        fixture.directory.join("missing-private-name"),
        PathBuf::from("https://example.invalid/private-token"),
        PathBuf::from("/dev/null"),
    ] {
        let error = match LocalPlayer::open(&path, AudioOutput::Silent) {
            Err(error) => error,
            Ok(_) => panic!("invalid local input accepted"),
        };
        let message = error.to_string();
        assert!(!message.contains("private"));
        assert!(!message.contains("/"));
    }
}

/// A late seek must preserve pending EOF, while calls after delivered EOF still fail normally.
#[test]
fn late_seek_does_not_replace_terminal_event_with_command_failure() {
    let fixture = Fixture::new(&silent_wav());
    let mut player = LocalPlayer::open(&fixture.path, AudioOutput::Silent).unwrap();
    player.set_paused(true).unwrap();
    wait_for(&mut player, PlaybackEvent::Loaded);
    let deadline = Instant::now() + Duration::from_secs(5);
    while player.audio_formats().unwrap().output.is_none() {
        player.poll_event().unwrap();
        assert!(Instant::now() < deadline, "output initialization deadline");
    }
    player.set_paused(false).unwrap();
    player
        .seek(SeekPosition::from_seconds(1.0).unwrap())
        .unwrap();
    // Deliberately hold client event consumption until the native engine is idle.
    std::thread::sleep(Duration::from_millis(150));
    for _ in 0..8 {
        player
            .seek(SeekPosition::from_seconds(1.0).unwrap())
            .unwrap();
    }
    wait_for(&mut player, PlaybackEvent::Ended);
    assert!(matches!(
        player.seek(SeekPosition::from_seconds(0.0).unwrap()),
        Err(PlaybackError::Native {
            operation: "seek",
            code: -12
        })
    ));
}
