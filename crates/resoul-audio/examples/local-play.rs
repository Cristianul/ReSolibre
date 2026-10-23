//! One local track to EOF. Ctrl-C terminates the standalone process.

#[cfg(target_env = "gnu")]
#[path = "support/process_memory.rs"]
mod process_memory;

use resolibre_audio::{AudioOutput, LocalPlayer};
use resolibre_core::{AudioFormat, AudioFormats, PlaybackEvent};
use std::path::PathBuf;
use std::process::ExitCode;

/// Parses one local path, reports safe lifecycle messages and handles failure once.
fn main() -> ExitCode {
    // SAFETY: this standalone entry point has not started any worker threads.
    #[cfg(target_env = "gnu")]
    if let Err(error) = unsafe { process_memory::configure() } {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }

    let mut arguments = std::env::args_os().skip(1);
    let Some(path) = arguments.next() else {
        eprintln!("Usage: local-play <local-audio-file>");
        return ExitCode::FAILURE;
    };

    if arguments.next().is_some() {
        eprintln!("Usage: local-play <local-audio-file>");
        return ExitCode::FAILURE;
    }

    match play(PathBuf::from(path)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Keeps draining native events until EOF; errors unwind and release the engine.
fn play(path: PathBuf) -> Result<(), resolibre_audio::PlaybackError> {
    let mut player = LocalPlayer::open(&path, AudioOutput::Device)?;
    let mut previous_formats = AudioFormats::default();
    let mut source_reported = false;
    let mut output_reported = false;

    loop {
        match player.poll_event()? {
            Some(PlaybackEvent::Loaded) => println!("info: local audio loaded"),
            Some(PlaybackEvent::Ended) => {
                if !source_reported || !output_reported {
                    println!("warn: audio format unavailable before EOF");
                }
                println!("info: playback ended");
                return Ok(());
            }
            None => {}
        }

        let formats = player.audio_formats()?;

        // Decoder properties can disappear while the output drains before EOF.
        // Remember evidence separately from the current property availability.
        source_reported |= formats.source.is_some();
        output_reported |= formats.output.is_some();

        if formats.source != previous_formats.source {
            print_format("source-decoded", formats.source);
        }
        if formats.output != previous_formats.output {
            print_format("output-audio-api", formats.output);
        }
        previous_formats = formats;
    }
}

/// Prints only typed format values; neither boundary is a physical DAC reading.
fn print_format(boundary: &str, format: Option<AudioFormat>) {
    match format {
        Some(format) => println!(
            "info: {boundary}: {} Hz, {} channels, {:?}",
            format.sample_rate_hz, format.channels, format.sample_format
        ),
        None => println!("info: {boundary}: unavailable"),
    }
}
