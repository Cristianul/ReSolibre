//! Private fixed-case capture worker for tools/check_equalizer.py.

use crate::{AudioOutput, LocalPlayer, PlaybackError};
use resolibre_core::{PlaybackEvent, equalizer::Equalizer, session::Volume};
use std::{
    path::Path,
    time::{Duration, Instant},
};

/// Captures only owned fixtures; the parent bounds process lifetime/files and checks preservation.
#[test]
#[ignore = "run through tools/check_equalizer.py"]
fn capture_equalizer() -> Result<(), PlaybackError> {
    let mode: usize = std::env::var("RESOUL_EQ_CASE")
        .expect("capture parent")
        .parse()
        .expect("numeric case");
    assert!(mode < 18);
    let input = Path::new("private-input.wav");
    assert!(input.symlink_metadata().unwrap().is_file());
    assert!(Path::new("capture.wav").symlink_metadata().is_err());
    let mut bands = [0.0; 10];
    let mut preamp = 0.0;
    let mut enabled = true;
    match mode {
        0..=9 => bands[mode] = 6.0,
        10 => preamp = 2.0,
        11 => {
            bands = [12.0; 10];
            enabled = false;
        }
        12 | 13 => bands = [12.0; 10],
        14 | 16 | 17 => {
            bands = [12.0; 10];
            preamp = -2.0;
        }
        15 => {
            bands[7] = 12.0;
            bands[8] = 12.0;
            bands[9] = 12.0;
        }
        _ => unreachable!(),
    }
    let settings = Equalizer::new(enabled, bands, preamp).unwrap();
    let mut player = LocalPlayer::open_with_equalizer(
        input,
        AudioOutput::CapturePcm { resample: false },
        false,
        Volume::from_percent(100.0).unwrap(),
        settings,
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    // Static settings were installed before load. Do not pause the untimed PCM
    // sink: an entire short low-rate input can otherwise reach EOF during setup.
    loop {
        assert!(Instant::now() < deadline);
        if player.poll_event()? == Some(PlaybackEvent::Ended) {
            break;
        }
    }
    drop(player);
    Ok(())
}
