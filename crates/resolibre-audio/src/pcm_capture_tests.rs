//! Worker for tools/check_pcm_conversion.py; never run against user media.

use super::{AudioOutput, LocalPlayer, PlaybackError};
use resolibre_core::{AudioFormat, AudioFormats, PlaybackEvent};
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

/// Emits only typed format fields; no filenames, tags or native messages escape.
fn report_format(boundary: &str, format: AudioFormat) {
    println!(
        "pcm-{boundary}: {} {} {:?}",
        format.sample_rate_hz, format.channels, format.sample_format
    );
}

/// Captures the fixed fixture with parent-enforced process and file-size limits.
/// The parent creates a private working directory and reaps us before removing it.
#[test]
#[ignore = "run through python3 tools/check_pcm_conversion.py"]
fn capture_fixture() -> Result<(), PlaybackError> {
    let mode = std::env::var("RESOUL_PCM_CASE").expect("capture parent required");
    assert!(matches!(
        mode.as_str(),
        "rate"
            | "source"
            | "tap-source"
            | "tap-stalled"
            | "bad-output"
            | "volume-0"
            | "volume-50"
            | "volume-100"
    ));
    let directory = std::fs::metadata(".").expect("private working directory");
    assert_eq!(directory.permissions().mode() & 0o777, 0o700);
    let input = Path::new("private-input.wav");
    let destination = Path::new("capture.wav");
    assert!(input.symlink_metadata().expect("owned fixture").is_file());
    if mode == "bad-output" {
        assert!(
            destination
                .symlink_metadata()
                .expect("owned directory")
                .is_dir()
        );
    } else {
        // Unlike exists(), this also rejects a dangling destination symlink.
        assert!(matches!(
            destination.symlink_metadata(),
            Err(error) if error.kind() == ErrorKind::NotFound
        ));
    }

    let mut player = if let Some(percent) = mode.strip_prefix("volume-") {
        let volume = resolibre_core::session::Volume::from_percent(
            percent.parse().expect("fixed volume mode"),
        )
        .expect("validated volume");
        LocalPlayer::open_for_session(
            input,
            AudioOutput::CapturePcm { resample: false },
            true,
            volume,
        )?
    } else {
        LocalPlayer::open(
            input,
            AudioOutput::CapturePcm {
                resample: !matches!(mode.as_str(), "source" | "tap-source" | "tap-stalled"),
            },
        )?
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "capture format deadline");
        assert_ne!(player.poll_event()?, Some(PlaybackEvent::Ended));
        let formats = player.audio_formats()?;
        if let (Some(source), Some(output)) = (formats.source, formats.output) {
            report_format("source", source);
            report_format("output", output);
            break;
        }
    }

    if mode.starts_with("tap-") {
        assert!(player.live_spectrum().expect("bundled tap").is_some());
    }
    player.set_paused(false)?;
    loop {
        assert!(Instant::now() < deadline, "capture EOF deadline");
        if player.poll_event()? == Some(PlaybackEvent::Ended) {
            break;
        }
        if mode == "tap-source" {
            player.live_spectrum().expect("consuming tap");
        }
    }
    assert_eq!(player.audio_formats()?, AudioFormats::default());
    drop(player);
    println!("pcm-finalized");
    Ok(())
}
