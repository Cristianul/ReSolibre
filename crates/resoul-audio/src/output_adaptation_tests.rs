//! Controlled output negotiation with real libmpv and a simulated silent sink.

use super::{AudioOutput, LocalPlayer, PlaybackError};
use resolibre_core::{AudioFormat, AudioFormats, PlaybackEvent, SampleFormat};
use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
const SOURCE: AudioFormat = AudioFormat {
    sample_rate_hz: 44_100,
    channels: 1,
    sample_format: SampleFormat::Signed16,
};
const RESTRICTED_OUTPUT: AudioFormat = AudioFormat {
    sample_rate_hz: 44_100,
    channels: 2,
    sample_format: SampleFormat::Float32,
};

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
}

impl Fixture {
    /// Creates a private, uniquely owned directory containing read-only test data.
    fn new(contents: &[u8]) -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "resolibre-adaptation-{}-{sequence}",
            std::process::id()
        ));
        DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .expect("create private fixture directory");

        let path = directory.join("private-source.wav");
        fs::write(&path, contents).expect("write generated fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444))
            .expect("make fixture read-only");
        Self { directory, path }
    }

    /// Checks original bytes, modification time and permissions after native teardown.
    fn assert_preserved(&self, contents: &[u8], before: &fs::Metadata) {
        let after = fs::metadata(&self.path).expect("fixture metadata after playback");
        assert_eq!(fs::read(&self.path).expect("read original"), contents);
        assert_eq!(
            before.modified().expect("mtime"),
            after.modified().expect("mtime")
        );
        assert_eq!(after.permissions().mode() & 0o777, 0o444);
    }
}

impl Drop for Fixture {
    /// Removes only this test's generated files, after the player has been dropped.
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Generates two seconds of quiet 440 Hz mono PCM16 without external media.
fn tone_wav() -> Vec<u8> {
    let frames = SOURCE.sample_rate_hz * 2;
    let data_size = frames * 2;
    let mut bytes = Vec::with_capacity(44 + data_size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&SOURCE.sample_rate_hz.to_le_bytes());
    bytes.extend_from_slice(&(SOURCE.sample_rate_hz * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());

    for frame in 0..frames {
        let seconds = f64::from(frame) / f64::from(SOURCE.sample_rate_hz);
        let sample =
            (0.08 * f64::from(i16::MAX) * (std::f64::consts::TAU * 440.0 * seconds).sin()) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }

    bytes
}

/// Requires matching format evidence, advancing engine position and eventual EOF.
/// Missing readings during drain are allowed; no physical samples are measured.
fn play_to_end(fixture: &Fixture, output: AudioOutput, expected: AudioFormat) {
    let mut player = LocalPlayer::open(&fixture.path, output).expect("open generated tone");
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut loaded = false;
    let mut observed_formats = false;
    let mut first_position = None;
    let mut advanced = false;

    loop {
        assert!(
            Instant::now() < deadline,
            "adaptation playback deadline exceeded"
        );
        match player.poll_event().expect("adaptation event") {
            Some(PlaybackEvent::Loaded) => loaded = true,
            Some(PlaybackEvent::Ended) => break,
            None => {}
        }

        let formats = player.audio_formats().expect("adaptation formats");
        if let Some(source) = formats.source {
            assert_eq!(source, SOURCE);
        }
        if let Some(actual_output) = formats.output {
            assert_eq!(actual_output, expected);
        }
        // Retain paired evidence once observed. Separate native reads can straddle EOF,
        // when format snapshots still exist but stream metadata has already disappeared.
        if !observed_formats && formats.source.is_some() && formats.output.is_some() {
            let info = player
                .stream_info()
                .expect("source information")
                .expect("loaded");
            assert_eq!(info.sample_rate_hz, Some(SOURCE.sample_rate_hz));
            assert_eq!(info.channels, Some(SOURCE.channels));
            observed_formats = true;
        }

        if loaded && let Ok(position) = player.position() {
            let start = *first_position.get_or_insert(position.seconds());
            if position.seconds() - start >= 0.5 {
                advanced = true;
            }
        }
    }

    assert!(loaded, "EOF without loaded event");
    assert!(observed_formats, "no paired decoder/output evidence");
    assert!(advanced, "no sustained engine progress");
    assert_eq!(
        player.audio_formats().expect("after EOF"),
        AudioFormats::default()
    );
    println!("validated simulated output: {SOURCE:?} -> {expected:?}; progress and EOF");
}

/// Proves conversion only for a restricted sink and checks per-engine isolation.
#[test]
fn adapts_to_restricted_sink_without_changing_default_policy() {
    let contents = tone_wav();
    let fixture = Fixture::new(&contents);
    let before = fs::metadata(&fixture.path).expect("fixture metadata");

    for (output, expected) in [
        (AudioOutput::Silent, SOURCE),
        (AudioOutput::RestrictedSilent, RESTRICTED_OUTPUT),
        (AudioOutput::Silent, SOURCE),
    ] {
        play_to_end(&fixture, output, expected);
        fixture.assert_preserved(&contents, &before);
    }
}

/// Output constraints must not turn malformed private input into successful EOF.
#[test]
fn restricted_sink_rejects_malformed_input_without_private_diagnostics() {
    let contents = b"private-invalid-media";
    let fixture = Fixture::new(contents);
    let before = fs::metadata(&fixture.path).expect("fixture metadata");
    let mut player = LocalPlayer::open(&fixture.path, AudioOutput::RestrictedSilent)
        .expect("request malformed fixture");
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        assert!(
            Instant::now() < deadline,
            "malformed input deadline exceeded"
        );
        match player.poll_event() {
            Err(error) => {
                assert!(matches!(
                    error,
                    PlaybackError::Native {
                        operation: "playback",
                        code,
                    } if code < 0
                ));
                let message = error.to_string();
                assert!(!message.contains("private"));
                assert!(!message.contains('/'));
                break;
            }
            Ok(Some(_)) => panic!("malformed input reached a successful lifecycle event"),
            Ok(None) => {}
        }
    }

    drop(player);
    fixture.assert_preserved(contents, &before);
}
