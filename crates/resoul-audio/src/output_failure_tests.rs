//! Real adapter worker for the private audio-server failure harness.

use super::{AudioOutput, LocalPlayer, PlaybackError};
use resolibre_core::{AudioFormats, PlaybackEvent};
use std::ffi::CString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Pins only this test build to the parent's owned local endpoint and sink.
pub(super) fn configure(player: &mut LocalPlayer) -> Result<(), PlaybackError> {
    let directory = std::env::current_dir().expect("owned working directory");
    let endpoint =
        CString::new(format!("unix:{}/native", directory.display())).expect("owned socket address");
    player.set_option(c"ao", c"pulse")?;
    player.set_option(c"pulse-host", &endpoint)?;
    player.set_option(c"audio-device", c"pulse/resolibre-test")?;
    Ok(())
}

/// Reports observed lifecycle, never the parent's expected outcome or fault time.
/// The parent bounds the entire process, including native open and destruction.
#[test]
#[ignore = "run through python3 tools/check_output_failure.py"]
fn play_fixture() -> Result<(), PlaybackError> {
    let directory = std::fs::metadata(".").expect("owned working directory");
    assert_eq!(directory.permissions().mode() & 0o777, 0o700);
    let input = Path::new("private-input.wav");
    let metadata = input.symlink_metadata().expect("owned fixture");
    assert!(metadata.is_file());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o444);

    let mut player = match LocalPlayer::open(input, AudioOutput::PrivatePulse) {
        Ok(player) => player,
        Err(error) => {
            println!("output-error: {error}");
            println!("output-dropped");
            return Ok(());
        }
    };

    loop {
        match player.poll_event() {
            Ok(Some(PlaybackEvent::Loaded)) => println!("output-loaded"),
            Ok(Some(PlaybackEvent::Ended)) => {
                println!("output-eof");
                break;
            }
            Err(error) => {
                println!("output-error: {error}");
                break;
            }
            Ok(None) => {}
        }
    }

    assert_eq!(player.audio_formats()?, AudioFormats::default());
    println!("output-formats-cleared");
    drop(player);
    println!("output-dropped");
    Ok(())
}
