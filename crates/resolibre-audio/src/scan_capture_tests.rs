//! Pitch/duration evidence through the production scanning adapter, in a private child.

use crate::{AudioOutput, LocalPlayer, PlaybackError};
use resolibre_core::{
    PlaybackEvent, SeekPosition,
    equalizer::Equalizer,
    session::{ScanDirection, Volume},
};
use std::path::Path;
use std::time::{Duration, Instant};

/// Captures only a generated 12-second tone; the parent bounds files/lifetime and checks input hashes.
#[test]
#[ignore = "run through python3 tools/check_transport_scan.py"]
fn capture_scan() -> Result<(), PlaybackError> {
    let mode = std::env::var("RESOUL_SCAN_CASE").expect("capture parent required");
    let (extension, reverse) = mode.split_once('-').expect("fixed case");
    assert!(matches!(extension, "wav" | "flac" | "mp3"));
    assert!(matches!(reverse, "forward" | "backward"));
    let backward = reverse == "backward";
    let input = format!("private-input.{extension}");
    assert!(Path::new(&input).symlink_metadata().unwrap().is_file());
    assert!(Path::new("capture.wav").symlink_metadata().is_err());
    let settings = Equalizer::default();
    let mut player = LocalPlayer::open_with_equalizer(
        Path::new(&input),
        AudioOutput::CapturePcm { resample: true },
        true,
        Volume::from_percent(100.0).unwrap(),
        settings,
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "scan format deadline");
        assert_ne!(player.poll_event()?, Some(PlaybackEvent::Ended));
        if player.prepare_equalizer(settings)? {
            break;
        }
    }
    if backward {
        // Stay just inside the last sample so codecs cannot turn setup into EOF.
        player.seek_with_equalizer(SeekPosition::from_seconds(11.9).unwrap(), settings)?;
        loop {
            assert!(Instant::now() < deadline, "reverse seek deadline");
            assert_ne!(player.poll_event()?, Some(PlaybackEvent::Ended));
            if player
                .timing()?
                .position
                .is_some_and(|position| position > 11.7)
            {
                break;
            }
        }
    }
    let direction = if backward {
        ScanDirection::Backward
    } else {
        ScanDirection::Forward
    };
    player.set_scan(Some(direction), settings, false)?;
    // The app holds the edge; this capture needs a finalized file instead.
    player.playback_command(&["set", "keep-open", "no"], "capture EOF")?;
    loop {
        assert!(Instant::now() < deadline, "scan EOF deadline");
        if player.poll_event()? == Some(PlaybackEvent::Ended) {
            break;
        }
    }
    drop(player);
    Ok(())
}
