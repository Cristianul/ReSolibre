//! Native-clock regression using generated, muted media; it does not capture system audio.

use super::{PhaseAdjustment, PhaseAlignment};
use crate::{AudioOutput, LocalPlayer};
use resolibre_core::{SeekPosition, equalizer::Equalizer, session::Volume};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Fixture {
    path: PathBuf,
    original: Vec<u8>,
    modified: SystemTime,
}

impl Fixture {
    /// Creates only a uniquely named owned WAV; native players receive read-only retained handles.
    fn new() -> Self {
        let sequence = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "resolibre-phase-{}-{sequence}.wav",
            std::process::id()
        ));
        let rate = 8_000_u32;
        let frames = rate * 16;
        let data_size = frames * 2;
        let mut original = Vec::with_capacity(44 + data_size as usize);
        original.extend_from_slice(b"RIFF");
        original.extend_from_slice(&(36 + data_size).to_le_bytes());
        original.extend_from_slice(b"WAVEfmt ");
        original.extend_from_slice(&16_u32.to_le_bytes());
        original.extend_from_slice(&1_u16.to_le_bytes());
        original.extend_from_slice(&1_u16.to_le_bytes());
        original.extend_from_slice(&rate.to_le_bytes());
        original.extend_from_slice(&(rate * 2).to_le_bytes());
        original.extend_from_slice(&2_u16.to_le_bytes());
        original.extend_from_slice(&16_u16.to_le_bytes());
        original.extend_from_slice(b"data");
        original.extend_from_slice(&data_size.to_le_bytes());
        for frame in 0..frames {
            let phase = f64::from(frame) * std::f64::consts::TAU * 440.0 / f64::from(rate);
            let sample = (phase.sin() * 1_000.0) as i16;
            original.extend_from_slice(&sample.to_le_bytes());
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(&original).unwrap();
        drop(file);
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        Self {
            path,
            original,
            modified,
        }
    }

    /// Confirms that native startup, phase seeks and pitch correction preserve the source.
    fn assert_preserved(&self) {
        assert_eq!(std::fs::read(&self.path).unwrap(), self.original);
        assert_eq!(
            std::fs::metadata(&self.path).unwrap().modified().unwrap(),
            self.modified
        );
    }
}

impl Drop for Fixture {
    /// Removes the single fixture created by this test after native handles are dropped.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Real native seeks and stretcher reconstruction must converge after a deliberately late second start.
#[test]
fn delayed_native_start_converges_without_unbounded_seeks() {
    assert_native_phase_alignment(AudioOutput::Silent);
}

/// Device clocks have a separate latency; this check uses application silence and owned media only.
#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires a Windows audio device; generated source remains muted"]
fn windows_device_phase_alignment() {
    assert_native_phase_alignment(AudioOutput::Device);
}

/// Runs the production clock controller against two independent libmpv handles at equal nominal BPM.
fn assert_native_phase_alignment(output: AudioOutput) {
    let fixture = Fixture::new();
    let settings = Equalizer::default();
    let silence = Volume::from_percent(0.0).unwrap();
    let mut outgoing =
        LocalPlayer::open_with_equalizer(&fixture.path, output, true, silence, settings).unwrap();
    let mut incoming =
        LocalPlayer::open_with_equalizer(&fixture.path, output, true, silence, settings).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        outgoing.try_event().unwrap();
        incoming.try_event().unwrap();
        if outgoing.prepare_equalizer(settings).unwrap()
            && incoming.prepare_equalizer(settings).unwrap()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "native phase preparation deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    incoming.prepare_mix_tempo().unwrap();
    outgoing.set_paused(false).unwrap();
    std::thread::sleep(Duration::from_millis(420));
    incoming.set_paused(false).unwrap();

    let beginning = Instant::now();
    let mut phase = PhaseAlignment::default();
    phase.start(beginning);
    let mut seek_count = 0;
    let mut settled_observations = 0;
    let mut maximum_settled_error = 0.0_f64;
    let mut speed = 1.0;
    while beginning.elapsed() < Duration::from_secs(6) {
        outgoing.try_event().unwrap();
        incoming.try_event().unwrap();
        if !incoming.is_seeking().unwrap()
            && let Some(wanted) = outgoing.audio_position().unwrap()
            && let Some(actual) = incoming.audio_position().unwrap()
        {
            let can_seek = beginning.elapsed() < Duration::from_secs(1);
            let adjustment = phase.observe(wanted, actual, 1.0, can_seek, Instant::now());
            match adjustment {
                Some(PhaseAdjustment::Seek(target)) => {
                    incoming
                        .seek_with_equalizer(SeekPosition::from_seconds(target).unwrap(), settings)
                        .unwrap();
                    seek_count += 1;
                }
                Some(PhaseAdjustment::Follow {
                    speed: corrected, ..
                }) if (corrected - speed).abs() > 0.0001 => {
                    incoming.set_mix_speed(corrected).unwrap();
                    speed = corrected;
                }
                Some(PhaseAdjustment::Follow { .. }) | None => {}
            }
            if beginning.elapsed() > Duration::from_secs(4) {
                maximum_settled_error = maximum_settled_error.max((actual - wanted).abs());
                settled_observations += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        (1..=3).contains(&seek_count),
        "startup correction must remain bounded"
    );
    assert!(
        settled_observations > 50,
        "missing native clock observations"
    );
    assert!(
        maximum_settled_error <= 0.030,
        "native phase error: {maximum_settled_error:.3} seconds"
    );
    drop(incoming);
    drop(outgoing);
    fixture.assert_preserved();
}
