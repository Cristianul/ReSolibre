use super::*;

/// Plans stay bounded, cover the middle of long tracks and never require simultaneous PCM windows.
#[test]
fn excerpts_are_bounded_and_non_overlapping() {
    for rate in [44_100, 48_000, 96_000, 192_000] {
        for seconds in [2.0, 14.5, 15.0, 30.0, 60.0, 209.720875, 14_400.0] {
            let windows = plan(rate, seconds);
            assert!((1..=6).contains(&windows.len()));
            let mut previous_end = 0;
            for (start, end) in windows {
                assert!(start >= previous_end && end > start);
                assert!(end - start <= (f64::from(rate) * WINDOW_SECONDS) as u64);
                assert!(end <= (seconds * f64::from(rate)).round() as u64);
                previous_end = end;
            }
        }
    }
}

/// Silent audio never initializes the model; cancellation and incomplete data cannot become evidence.
#[test]
fn silence_and_cancellation_are_explicit() {
    let cancel = AtomicBool::new(false);
    let mut detector = Excerpts::new(44_100, 1, 2.0);
    for _ in 0..20 {
        detector.push(&vec![0.0; 4410], &cancel).unwrap();
    }
    assert!(detector.scanner.is_none());
    assert_eq!(detector.next, 1);
    assert_eq!(detector.finish(), ProvenanceReport::InsufficientAudio);
    let mut detector = Excerpts::new(44_100, 2, 60.0);
    cancel.store(true, Ordering::Relaxed);
    assert!(detector.push(&[0.1, -0.1], &cancel).is_err());
    assert!(detector.samples.is_empty());
}

/// A real restricted child must reject scratch output instead of terminating the player with SIGXFSZ.
#[cfg(unix)]
#[test]
fn process_file_quota_is_recoverable() {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args([
        "track_analysis::provenance::tests::scratch_file_quota_child",
        "--exact",
        "--ignored",
    ]);
    command.env("RESOLIBRE_SCRATCH_QUOTA_TEST", "1");
    // SAFETY: only async-signal-safe setrlimit runs between fork and exec; no shared parent state changes.
    unsafe {
        command.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: 2 * 1024 * 1024,
                rlim_max: 2 * 1024 * 1024,
            };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "restricted detector child failed: {:?}",
        output.status
    );
}

/// Runs only in the restricted owned child so the main test process never changes resource limits.
#[cfg(unix)]
#[test]
#[ignore = "invoked by process_file_quota_is_recoverable"]
fn scratch_file_quota_child() {
    assert_eq!(std::env::var("RESOLIBRE_SCRATCH_QUOTA_TEST").unwrap(), "1");
    let samples = vec![0.1; 44_100 * 2 * 14];
    assert!(scratch_wave(44_100, 2, &samples).is_err());
}

/// The private WAV preserves float samples, native rate and stereo phase without quantization or gain.
#[test]
fn scratch_wave_preserves_native_pcm() {
    let samples = [0.125, -0.125, 1.1, -1.1];
    let mut wave = scratch_wave(96_000, 2, &samples).unwrap();
    let mut decoded = Vec::new();
    original_audio::pcm::stream(
        &mut wave,
        &AtomicBool::new(false),
        std::time::Instant::now(),
        Duration::from_secs(1),
        |rate, channels, values| {
            assert_eq!((rate, channels), (96_000, 2));
            decoded.extend_from_slice(values);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(decoded, samples);
}

/// Runs the identical detector path on an owned 14.5-second 44.1 kHz stereo float WAV, including on Windows.
#[test]
#[ignore = "requires RESOUL_PROVENANCE_PCM containing the documented owned PCM fixture"]
fn measure_decoded_provenance() {
    let path = std::env::var_os("RESOUL_PROVENANCE_PCM").expect("owned float WAV required");
    let mut input = File::open(path).unwrap();
    let cancel = AtomicBool::new(false);
    let mut detector = Excerpts::new(44_100, 2, WINDOW_SECONDS);
    let started = std::time::Instant::now();
    original_audio::pcm::stream_prefix(
        &mut input,
        &cancel,
        started,
        DEADLINE,
        Some(15),
        |rate, channels, samples| {
            assert_eq!((rate, channels), (44_100, 2));
            detector.push(samples, &cancel)
        },
    )
    .unwrap();
    assert_eq!(detector.next, detector.plan.len());
    let report = detector.finish();
    println!(
        "provenance={report:?} verdict={:?} elapsed={:.3}s",
        report.verdict(),
        started.elapsed().as_secs_f64()
    );
    assert!(matches!(report, ProvenanceReport::Measured { .. }));
    if let Ok(expected) = std::env::var("RESOUL_PROVENANCE_EXPECT") {
        assert_eq!(format!("{:?}", report.verdict()), expected);
    }
}

/// Exercises the actual embedded model on an explicitly supplied private recording without publishing media.
#[test]
#[ignore = "requires RESOUL_PROVENANCE_PROBE; opt-in model/native performance check"]
fn measure_authorized_provenance() {
    let path = std::env::var_os("RESOUL_PROVENANCE_PROBE").expect("authorized input required");
    let file = File::open(path).unwrap();
    let key = SourceKey::read(&file, 0).unwrap();
    let started = std::time::Instant::now();
    let mut job = super::super::TrackJob::start(file, key, None).unwrap();
    loop {
        if let Some(result) = job.poll() {
            let report = result.unwrap();
            println!(
                "provenance={:?} verdict={:?} elapsed={:.3}s",
                report.provenance,
                report.provenance.verdict(),
                started.elapsed().as_secs_f64()
            );
            assert!(report.provenance.is_valid());
            assert_ne!(report.provenance, ProvenanceReport::Unavailable);
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(400));
        std::thread::sleep(Duration::from_millis(10));
    }
}
