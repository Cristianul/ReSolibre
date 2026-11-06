//! Owned native Windows fixtures cover source isolation and the real playback clock.

use crate::{AudioOutput, LocalPlayer};
use resolibre_core::session::Volume;
use resolibre_core::spectrum::live::LiveLevels;
use resolibre_core::{PlaybackEvent, SeekPosition};
use std::time::{Duration, Instant};

/// Generates two distinct tone regions; opposite stereo phase must retain measured energy.
fn source(channels: u32, bits: u32) -> (tempfile::TempDir, std::path::PathBuf, Vec<u8>) {
    source_at_rate(channels, bits, 8000)
}

/// Keeps source construction identical for native-rate format and resource checks.
fn source_at_rate(
    channels: u32,
    bits: u32,
    rate: u32,
) -> (tempfile::TempDir, std::path::PathBuf, Vec<u8>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("generated.wav");
    let bytes_per_sample = bits / 8;
    let size = rate * 8 * channels * bytes_per_sample;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + size).to_le_bytes());
    bytes.extend(b"WAVEfmt \x10\0\0\0\x01\0");
    bytes.extend((channels as u16).to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * channels * bytes_per_sample).to_le_bytes());
    bytes.extend((channels as u16 * bytes_per_sample as u16).to_le_bytes());
    bytes.extend((bits as u16).to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    for frame in 0..rate * 8 {
        let frequency = if frame < rate * 4 { 250.0 } else { 1500.0 };
        let sample = ((std::f64::consts::TAU * frequency * f64::from(frame) / f64::from(rate))
            .sin()
            * 12000.0) as i16;
        for channel in 0..channels {
            let sample = if channel % 2 == 0 { sample } else { -sample };
            if bits == 16 {
                bytes.extend(sample.to_le_bytes());
            } else {
                let precise = (i32::from(sample) << 16) + (frame % 1024) as i32;
                bytes.extend(precise.to_le_bytes());
            }
        }
    }
    std::fs::write(&path, &bytes).unwrap();
    (directory, path, bytes)
}

/// Encodes only the generated source; its fixed input format avoids depending on probe heuristics.
fn encode_flac(path: &std::path::Path) -> std::path::PathBuf {
    let encoded = path.with_extension("flac");
    let status = crate::platform::helper("ffmpeg")
        .args(["-nostdin", "-v", "quiet", "-f", "wav", "-i"])
        .arg(path)
        .args(["-c:a", "flac", "-sample_fmt", "s32"])
        .arg(&encoded)
        .status()
        .unwrap();
    assert!(status.success(), "generated FLAC encoding failed");
    encoded
}

/// Native-rate compressed stereo uses every applicable band without downmixing or rejecting playable audio.
#[test]
fn native_high_rate_stereo_uses_original_format() {
    for rate in [44100, 96000, 192000] {
        let (_directory, path, original) = source_at_rate(2, 32, rate);
        let encoded = encode_flac(&path);
        let encoded_bytes = std::fs::read(&encoded).unwrap();
        let mut player = LocalPlayer::open_for_session(
            &encoded,
            AudioOutput::Silent,
            false,
            Volume::from_percent(0.0).unwrap(),
        )
        .unwrap();
        let started = Instant::now();
        while player.try_event().unwrap() != Some(PlaybackEvent::Loaded) {
            assert!(started.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(5));
        }
        let levels = measured(&mut player);
        assert!(
            levels
                .levels
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        );
        assert_eq!(
            player
                .audio_formats()
                .unwrap()
                .source
                .unwrap()
                .sample_rate_hz,
            rate
        );
        drop(player);
        assert!(std::fs::read(path).unwrap() == original);
        assert!(std::fs::read(encoded).unwrap() == encoded_bytes);
    }
}

/// Missing filter metadata expires honestly; sources below the displayed range remain valid and dark.
#[test]
fn missing_measurements_expire_but_empty_bands_remain_valid() {
    let capture = Capture::new();
    capture.option("config", "no");
    capture.option("terminal", "no");
    // SAFETY: the uniquely owned client is configured and lives until both consumers are dropped.
    assert!(unsafe { crate::ffi::mpv_initialize(capture.0.as_ptr()) } >= 0);
    for rate in [48000, 40] {
        let graph = super::graph::Graph::new(rate, 1, "mono").unwrap();
        let mut spectrum = super::LiveSpectrum::new(capture.0).unwrap();
        spectrum.blocked = vec![None; graph.groups.len()];
        spectrum.graph = Some(graph);
        let first = spectrum.poll(Some(0.0), false, false).unwrap();
        assert!(first.levels.iter().all(|value| *value == 0.0));
        spectrum.last_poll = None;
        let later = spectrum.poll(Some(4.0), false, false);
        if rate == 40 {
            assert_eq!(later.unwrap(), first);
        } else {
            assert_eq!(later.unwrap_err(), "Live spectrum measurements unavailable");
        }
    }
}

/// Reads process CPU only; the owned probe never inspects another application or system audio.
fn cpu_seconds() -> f64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    assert_ne!(
        // SAFETY: GetCurrentProcess identifies this process; all FILETIME outputs are live and correctly sized.
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        },
        0
    );
    let ticks =
        |time: FILETIME| u64::from(time.dwLowDateTime) | (u64::from(time.dwHighDateTime) << 32);
    (ticks(kernel) + ticks(user)) as f64 / 10_000_000.0
}

/// Reports an isolated six-second playback sample; this does not replace the full percentile/resource gate.
#[test]
#[ignore = "set RESOLIBRE_LIVE_BENCHMARK to baseline/live; run separately without concurrent builds"]
fn measure_native_live_cpu() {
    let mode = std::env::var("RESOLIBRE_LIVE_BENCHMARK").unwrap();
    assert!(matches!(mode.as_str(), "baseline" | "live"));
    let (_directory, path, _) = source_at_rate(2, 32, 48000);
    let encoded = encode_flac(&path);
    let mut player = LocalPlayer::open_for_session(
        &encoded,
        AudioOutput::Silent,
        false,
        Volume::from_percent(0.0).unwrap(),
    )
    .unwrap();
    let started = Instant::now();
    while player.try_event().unwrap() != Some(PlaybackEvent::Loaded) {
        assert!(started.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(5));
    }
    let cpu = cpu_seconds();
    let started = Instant::now();
    let mut next = Instant::now();
    let mut observed = false;
    while started.elapsed() < Duration::from_secs(6) {
        player.try_event().unwrap();
        if mode == "live" && Instant::now() >= next {
            if let Some(levels) = player.live_spectrum().unwrap() {
                observed |= levels.levels.iter().any(|level| *level > 0.3);
            }
            next = Instant::now() + Duration::from_millis(50);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let percent = 100.0 * (cpu_seconds() - cpu) / started.elapsed().as_secs_f64();
    println!("windows-live mode={mode} cpu_percent_of_one_core={percent:.3}");
    assert!(mode == "baseline" || observed);
}

/// Waits for real measured power while draining native events; never accepts an invented timer signal.
fn measured(player: &mut LocalPlayer) -> LiveLevels {
    let started = Instant::now();
    loop {
        player.try_event().unwrap();
        if let Some(levels) = player.live_spectrum().unwrap()
            && levels.levels.iter().any(|level| *level > 0.3)
        {
            return levels;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "native live measurement deadline"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Measured bands follow frequency across paused seek, preserve anti-phase stereo and never depend on volume.
#[test]
fn native_bands_follow_seek_and_preserve_originals() {
    for channels in [1, 2] {
        let (_directory, path, bytes) = source(channels, 16);
        let mut player = LocalPlayer::open_for_session(
            &path,
            AudioOutput::Silent,
            false,
            Volume::from_percent(0.0).unwrap(),
        )
        .unwrap();
        let started = Instant::now();
        while player.try_event().unwrap() != Some(PlaybackEvent::Loaded) {
            assert!(started.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(5));
        }
        let first = measured(&mut player);
        let first_peak = first
            .levels
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        player.set_paused(true).unwrap();
        let frozen = player.live_spectrum().unwrap().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(player.live_spectrum().unwrap(), Some(frozen));
        player
            .seek(SeekPosition::from_seconds(5.0).unwrap())
            .unwrap();
        assert!(
            player
                .live_spectrum()
                .unwrap()
                .unwrap()
                .levels
                .iter()
                .all(|value| *value == 0.0)
        );
        player.set_paused(false).unwrap();
        let second = measured(&mut player);
        let second_peak = second
            .levels
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert!(second_peak > first_peak + 4);
        assert_ne!(first.epoch, second.epoch);

        // Simulate a runtime that stops publishing metadata; failure must retire its native work.
        let spectrum = player.live_spectrum.as_mut().unwrap();
        let group = &mut spectrum.graph.as_mut().unwrap().groups[0];
        let property = std::mem::replace(
            &mut group.property,
            std::ffi::CString::new("af-metadata/resolibre_live_missing").unwrap(),
        );
        spectrum.last_progress_clock = Some(-4.0);
        spectrum.last_poll = None;
        assert_eq!(
            player.live_spectrum().unwrap_err(),
            "Live spectrum measurements unavailable"
        );
        let spectrum = player.live_spectrum.as_mut().unwrap();
        let group = &mut spectrum.graph.as_mut().unwrap().groups[0];
        group.property = property;
        assert!(
            super::metadata::read(player.handle, group)
                .unwrap()
                .is_none()
        );
        let before = player.timing().unwrap().position.unwrap();
        std::thread::sleep(Duration::from_millis(80));
        assert!(player.timing().unwrap().position.unwrap() > before);
        drop(player);
        assert!(std::fs::read(path).unwrap() == bytes);
    }
}

/// A fresh child owns each capture directory; both live and stalled readers must preserve exact PCM.
#[test]
fn native_filter_preserves_pcm_with_a_stalled_reader() {
    for (channels, bits) in [(1, 16), (2, 32), (6, 16)] {
        let (directory, path, original) = source(channels, bits);
        let mut baseline = None;
        for mode in ["baseline", "live", "stalled"] {
            let output = directory.path().join(mode);
            std::fs::create_dir(&output).unwrap();
            std::fs::copy(&path, output.join("input.wav")).unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "live_spectrum::windows::tests::capture_worker",
                    "--exact",
                    "--ignored",
                    "--test-threads=1",
                ])
                .current_dir(&output)
                .env("RESOLIBRE_LIVE_CAPTURE", mode)
                .env("RESOLIBRE_LIVE_CAPTURE_BITS", bits.to_string())
                .env("RESOLIBRE_LIVE_CAPTURE_CHANNELS", channels.to_string())
                .output()
                .unwrap();
            let diagnostic = format!(
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            let reason = diagnostic
                .lines()
                .find(|line| {
                    line.contains("called `Result::unwrap()`") || line.contains("assertion failed")
                })
                .unwrap_or("no typed capture diagnostic");
            assert!(
                result.status.success(),
                "owned PCM capture failed for {channels} channels/{bits} bits/{mode}: {:?}: {reason}",
                result.status
            );
            let captured = std::fs::read(output.join("capture.wav")).unwrap();
            if let Some(baseline) = &baseline {
                assert!(
                    &captured == baseline,
                    "live graph changed PCM for {channels} channels/{bits} bits/{mode}"
                );
            } else {
                assert!(captured.len() > 44);
                baseline = Some(captured);
            }
            assert!(std::fs::read(output.join("input.wav")).unwrap() == original);
        }
    }
}

/// Runs only under the parent-owned temporary directory and never opens a real output device.
#[test]
#[ignore = "invoked only by native_filter_preserves_pcm_with_a_stalled_reader"]
fn capture_worker() {
    let mode = std::env::var("RESOLIBRE_LIVE_CAPTURE").unwrap();
    assert!(matches!(mode.as_str(), "baseline" | "live" | "stalled"));
    assert!(!std::path::Path::new("capture.wav").exists());
    let (cancel, deadline) = std::sync::mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if matches!(
            deadline.recv_timeout(Duration::from_secs(10)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            std::process::exit(13);
        }
    });
    let channels: u32 = std::env::var("RESOLIBRE_LIVE_CAPTURE_CHANNELS")
        .unwrap()
        .parse()
        .unwrap();
    let bits = std::env::var("RESOLIBRE_LIVE_CAPTURE_BITS").unwrap();
    let layout = match channels {
        1 => "mono",
        2 => "stereo",
        6 => "5.1",
        _ => panic!("owned channel count required"),
    };
    let graph = super::graph::Graph::new(8000, channels, layout).unwrap();
    let capture = Capture::new();
    for (name, value) in [
        ("config", "no"),
        ("terminal", "no"),
        ("load-scripts", "no"),
        ("ytdl", "no"),
        ("vid", "no"),
        ("audio-channels", "auto"),
        ("demuxer-lavf-format", "wav"),
        ("ao", "pcm"),
        ("ao-pcm-file", "capture.wav"),
        ("ao-pcm-append", "no"),
        ("audio-format", if bits == "32" { "s32" } else { "s16" }),
    ] {
        capture.option(name, value);
    }
    if mode != "baseline" {
        capture.option("af", &graph.filters);
    }
    // SAFETY: all options were copied into this uniquely owned, live client.
    assert!(unsafe { crate::ffi::mpv_initialize(capture.0.as_ptr()) } >= 0);
    let arguments = [
        c"loadfile".as_ptr(),
        c"input.wav".as_ptr(),
        std::ptr::null(),
    ];
    // SAFETY: the owned relative fixture and terminated argument vector outlive the synchronous call.
    assert!(unsafe { crate::ffi::mpv_command(capture.0.as_ptr(), arguments.as_ptr()) } >= 0);
    let started = Instant::now();
    loop {
        // SAFETY: the event belongs to this live client until its next wait call.
        let event = unsafe { &*crate::ffi::mpv_wait_event(capture.0.as_ptr(), 0.001) };
        if event.id == crate::ffi::EVENT_END_FILE {
            // SAFETY: END_FILE carries the documented payload, borrowed before the next wait.
            let end = unsafe { &*event.data.cast::<crate::ffi::EndFile>() };
            assert_eq!(end.reason, crate::ffi::END_REASON_EOF);
            break;
        }
        if mode == "live" {
            for group in &graph.groups {
                super::metadata::read(capture.0, group).unwrap();
            }
        }
        assert!(started.elapsed() < Duration::from_secs(6));
    }
    drop(capture);
    let _ = cancel.send(());
    watchdog.join().unwrap();
}

/// Owns only the capture child's native client, independently of production player wiring tests.
struct Capture(std::ptr::NonNull<crate::ffi::Handle>);

impl Capture {
    /// Allocates a single client which Drop always destroys, including assertion failures.
    fn new() -> Self {
        // SAFETY: mpv_create has no arguments and transfers one client allocation.
        Self(std::ptr::NonNull::new(unsafe { crate::ffi::mpv_create() }).unwrap())
    }

    /// Copies fixed capture settings or the validated owned graph before native initialization.
    fn option(&self, name: &str, value: &str) {
        let name = std::ffi::CString::new(name).unwrap();
        let value = std::ffi::CString::new(value).unwrap();
        assert!(
            // SAFETY: the client and both terminated strings remain alive through the copying call.
            unsafe {
                crate::ffi::mpv_set_option_string(self.0.as_ptr(), name.as_ptr(), value.as_ptr())
            } >= 0
        );
    }
}

impl Drop for Capture {
    /// Stops native processing before the parent compares or removes any generated capture.
    fn drop(&mut self) {
        // SAFETY: this owner holds the sole client and no native event borrows survive.
        unsafe { crate::ffi::mpv_terminate_destroy(self.0.as_ptr()) };
    }
}
