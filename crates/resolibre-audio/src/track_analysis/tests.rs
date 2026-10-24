use super::*;
use std::time::Instant;

/// Measures an explicitly supplied local recording without logging its path, tags, PCM or beat timeline.
#[test]
#[ignore = "requires RESOUL_ANALYSIS_PROBE pointing to an authorized local recording"]
fn measure_authorized_recording() {
    let path = std::env::var_os("RESOUL_ANALYSIS_PROBE").expect("probe input required");
    let file = File::open(path).expect("probe input unavailable");
    let key = SourceKey::read(&file, 0).unwrap();
    let retained = file.try_clone().unwrap();
    let started = Instant::now();
    if std::env::var_os("RESOUL_RHYTHM_ONLY").is_some() {
        let mut job = crate::rhythm_analysis::RhythmJob::start(file, key, false, None).unwrap();
        loop {
            if let Some(result) = job.poll() {
                let (_, report) = result.unwrap();
                let beats: usize = report
                    .segments
                    .iter()
                    .map(|segment| segment.beats.len())
                    .sum();
                println!(
                    "rhythm seconds={:.3} bpm={:?} variable={} observed_beats={beats}",
                    started.elapsed().as_secs_f64(),
                    report.bpm,
                    report.variable
                );
                assert_eq!(SourceKey::read(&retained, 0).unwrap(), key);
                return;
            }
            assert!(started.elapsed() < Duration::from_secs(15));
            thread::sleep(Duration::from_millis(5));
        }
    }
    let mut job = TrackJob::start(file, key, None).unwrap();
    loop {
        if let Some(result) = job.poll() {
            let report = result.unwrap();
            println!(
                "analysis seconds={:.3} source_seconds={:.3} bpm={:?} variable={} segments={}",
                started.elapsed().as_secs_f64(),
                report.duration_seconds,
                report.tempo.bpm,
                report.tempo.variable,
                report.tempo.segments.len()
            );
            for segment in &report.tempo.segments {
                println!(
                    "segment bpm={:.2} regularity={:.3} beats={}",
                    segment.bpm,
                    segment.confidence,
                    segment.beats.len()
                );
            }
            assert_eq!(SourceKey::read(&retained, 0).unwrap(), key);
            break;
        }
        assert!(
            started.elapsed() < DEADLINE + Duration::from_secs(5),
            "probe deadline"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Creates synthetic PCM with known tones and pulses; fixtures contain no user recordings.
fn source_wav(path: &std::path::Path, seconds: usize) {
    let rate = 44100_u32;
    let size = seconds as u32 * rate * 2;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + size).to_le_bytes());
    bytes.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(b"\x02\0\x10\0data");
    bytes.extend(size.to_le_bytes());
    for frame in 0..seconds * rate as usize {
        let time = frame as f64 / f64::from(rate);
        let phase = (time - 0.25).rem_euclid(0.5);
        let pulse = if time >= 0.25 && phase < 0.04 {
            (-phase * 100.0).exp() * 0.4
        } else {
            0.0
        };
        let value = (std::f64::consts::TAU * 1000.0 * time).sin() * (0.1 + pulse)
            + (std::f64::consts::TAU * 15000.0 * time).sin() * 0.1;
        bytes.extend(((value * 30000.0) as i16).to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

/// Encodes only owned fixtures using explicit arguments; native diagnostics never enter test reports.
fn encode(input: &std::path::Path, output: &std::path::Path, options: &[&str]) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-v", "quiet", "-y", "-i"])
        .arg(input)
        .args(options)
        .arg(output)
        .status()
        .unwrap();
    assert!(status.success(), "owned fixture encoding failed");
}

/// Waits with a deadline; the job itself validates identity and never publishes an incomplete decode.
fn analyze(path: &std::path::Path) -> TrackAnalysis {
    let file = File::open(path).unwrap();
    let key = SourceKey::read(&file, 0).unwrap();
    let mut job = TrackJob::start(file, key, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(result) = job.poll() {
            return result.unwrap();
        }
        assert!(Instant::now() < deadline, "analysis deadline");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Real lossless/lossy-transcoded/filtered/upsampled inputs yield measurements, never provenance certificates.
#[test]
fn encoded_variants_preserve_measured_bandwidth_and_source_bytes() {
    let directory =
        std::env::temp_dir().join(format!("resolibre-analysis-formats-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("source.misleading");
    source_wav(&source, 12);
    let before = std::fs::read(&source).unwrap();
    let lossless = directory.join("original.flac");
    let mp3 = directory.join("lossy.mp3");
    let transcoded = directory.join("transcoded.flac");
    let filtered = directory.join("filtered.flac");
    let upsampled = directory.join("upsampled.flac");
    encode(&source, &lossless, &["-c:a", "flac"]);
    for (path, bits) in [(&lossless, 16), (&upsampled, 24)] {
        if bits == 24 {
            encode(&source, path, &["-c:a", "flac", "-sample_fmt", "s32"]);
        }
        let mut player = crate::LocalPlayer::open(path, crate::AudioOutput::Silent).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if player.try_event().unwrap() == Some(resolibre_core::PlaybackEvent::Loaded) {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            player.stream_info().unwrap().unwrap().bits_per_sample,
            Some(bits)
        );
    }
    encode(&source, &mp3, &["-c:a", "libmp3lame", "-b:a", "96k"]);
    encode(&mp3, &transcoded, &["-c:a", "flac"]);
    encode(
        &source,
        &filtered,
        &[
            "-af",
            "lowpass=f=3000:p=2,lowpass=f=3000:p=2",
            "-c:a",
            "flac",
        ],
    );
    encode(&filtered, &upsampled, &["-ar", "96000", "-c:a", "flac"]);
    let original = analyze(&lossless);
    let filtered_result = analyze(&filtered);
    let upsampled_result = analyze(&upsampled);
    assert!(original.sustained_bandwidth_hz.unwrap() > 14900.0);
    assert!(
        filtered_result.sustained_bandwidth_hz.unwrap() < original.sustained_bandwidth_hz.unwrap()
    );
    assert_eq!(upsampled_result.sample_rate_hz, 96000);
    assert!(upsampled_result.sustained_bandwidth_hz.unwrap() < 20000.0);
    for path in [&source, &mp3, &transcoded] {
        let bytes = std::fs::read(path).unwrap();
        let report = analyze(path);
        assert!((report.duration_seconds - 12.0).abs() < 0.1);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    assert_eq!(std::fs::read(&source).unwrap(), before);
    std::fs::remove_dir_all(directory).unwrap();
}

/// Malformed media, cancelled work and changed identity fail privately rather than returning partial measurements.
#[test]
fn malformed_cancelled_and_changed_sources_fail_closed() {
    let path =
        std::env::temp_dir().join(format!("resolibre-analysis-invalid-{}", std::process::id()));
    std::fs::write(&path, b"invalid media").unwrap();
    let file = File::open(&path).unwrap();
    let key = SourceKey::read(&file, 0).unwrap();
    let mut job = TrackJob::start(file, key, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = job.poll() {
            assert!(result.is_err());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    source_wav(&path, 32);
    let file = File::open(&path).unwrap();
    let old_key = SourceKey::read(&file, 0).unwrap();
    std::fs::write(&path, b"changed media").unwrap();
    let mut job = TrackJob::start(file, old_key, None).unwrap();
    job.cancel();
    let started = Instant::now();
    loop {
        if let Some(result) = job.poll() {
            assert!(result.is_err());
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        thread::sleep(Duration::from_millis(5));
    }
    std::fs::remove_file(path).unwrap();
}

/// Native on-demand decoding preserves channel energy and dimensions instead of using a stereo output downmix.
#[test]
fn native_analysis_preserves_multichannel_phase_and_retry() {
    let directory = std::env::temp_dir().join(format!(
        "resolibre-analysis-channels-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let mono = directory.join("mono.wav");
    source_wav(&mono, 1);
    let bytes = std::fs::read(&mono).unwrap();
    let expected = analyze(&mono);
    for channels in [2_u16, 6] {
        let path = directory.join(format!("{channels}.wav"));
        let size = (bytes.len() as u32 - 44) * u32::from(channels);
        let mut multi = bytes[..44].to_vec();
        multi[4..8].copy_from_slice(&(36 + size).to_le_bytes());
        multi[22..24].copy_from_slice(&channels.to_le_bytes());
        multi[28..32].copy_from_slice(&(44100 * 2 * u32::from(channels)).to_le_bytes());
        multi[32..34].copy_from_slice(&(2 * channels).to_le_bytes());
        multi[40..44].copy_from_slice(&size.to_le_bytes());
        for sample in bytes[44..].as_chunks::<2>().0 {
            let value = i16::from_le_bytes(*sample);
            for channel in 0..channels {
                multi.extend((if channel % 2 == 0 { value } else { -value }).to_le_bytes());
            }
        }
        std::fs::write(&path, &multi).unwrap();
        let report = analyze(&path);
        assert_eq!(report.channels, usize::from(channels));
        assert_eq!(report.sample_rate_hz, 44100);
        assert_eq!(report.duration_seconds, expected.duration_seconds);
        assert!(
            report
                .power_db
                .iter()
                .zip(&expected.power_db)
                .all(|(observed, expected)| (observed - expected).abs() < 0.001)
        );
    }
    // Reusing a retained descriptor must restart at the beginning, even after a completed decode.
    let file = File::open(&mono).unwrap();
    let key = SourceKey::read(&file, 0).unwrap();
    for _ in 0..2 {
        let mut job = TrackJob::start(file.try_clone().unwrap(), key, None).unwrap();
        let started = Instant::now();
        loop {
            if let Some(result) = job.poll() {
                assert_eq!(result.unwrap().duration_seconds, 1.0);
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(5));
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

/// A video container's default audio stream is selected by identity; a mismatched stream is rejected.
#[test]
fn native_analysis_uses_selected_video_audio_stream() {
    let directory =
        std::env::temp_dir().join(format!("resolibre-analysis-streams-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("source.wav");
    let video = directory.join("video.mkv");
    source_wav(&source, 1);
    encode(
        &source,
        &video,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=16x16:r=1:d=1",
            "-map",
            "1:v",
            "-map",
            "0:a",
            "-map",
            "0:a",
            "-c:v",
            "ffv1",
            "-c:a",
            "flac",
            "-filter:a:1",
            "lowpass=f=3000:p=2,lowpass=f=3000:p=2",
            "-disposition:a:0",
            "0",
            "-disposition:a:1",
            "default",
        ],
    );
    for stream in [2, 0] {
        let file = File::open(&video).unwrap();
        let key = SourceKey::read(&file, stream).unwrap();
        let mut job = TrackJob::start(file, key, None).unwrap();
        let started = Instant::now();
        loop {
            if let Some(result) = job.poll() {
                if stream == 2 {
                    assert!(result.unwrap().sustained_bandwidth_hz.unwrap() < 14900.0);
                } else {
                    assert!(result.is_err());
                }
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(5));
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

/// Original PCM reads start independently of a retained descriptor's cursor, including repeated edge jobs.
#[test]
fn original_decoder_retries_have_independent_cursors() {
    use std::io::{Seek, SeekFrom};
    let path = std::env::temp_dir().join(format!(
        "resolibre-analysis-cursor-{}.wav",
        std::process::id()
    ));
    source_wav(&path, 1);
    let mut file = File::open(&path).unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    let key = SourceKey::read(&file, 0).unwrap();
    for _ in 0..2 {
        let cancel = AtomicBool::new(false);
        let result = original_audio::decode(
            file.try_clone().unwrap(),
            key,
            &cancel,
            Duration::from_secs(5),
            |reader, started| {
                original_audio::pcm::decode(reader, &cancel, started, Duration::from_secs(5))
            },
        )
        .unwrap();
        assert_eq!(result.duration, 1.0);
    }
    std::fs::remove_file(path).unwrap();
}

/// Tiny streams retain their exact sample count; cancellation at different native startup/drain boundaries is bounded.
#[test]
fn tiny_streams_and_native_cancellation_are_bounded() {
    let path = std::env::temp_dir().join(format!(
        "resolibre-analysis-tiny-{}.wav",
        std::process::id()
    ));
    source_wav(&path, 1);
    let original = std::fs::read(&path).unwrap();
    for frames in [1_u32, 47, 2049] {
        let mut bytes = original[..44 + frames as usize * 2].to_vec();
        bytes[4..8].copy_from_slice(&(36 + frames * 2).to_le_bytes());
        bytes[40..44].copy_from_slice(&(frames * 2).to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        let report = analyze(&path);
        assert!((report.duration_seconds - f64::from(frames) / 44100.0).abs() < 1e-9);
        assert!(report.tempo.bpm.is_none());
    }
    source_wav(&path, 96);
    for delay in [0, 1, 20, 60] {
        let file = File::open(&path).unwrap();
        let key = SourceKey::read(&file, 0).unwrap();
        let mut job = TrackJob::start(file, key, None).unwrap();
        thread::sleep(Duration::from_millis(delay));
        let started = Instant::now();
        job.cancel();
        loop {
            if let Some(result) = job.poll() {
                assert!(result.is_err());
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "native cancellation deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    std::fs::remove_file(path).unwrap();
}

/// Runs the real job with an explicit isolated cache, retaining progress to distinguish reuse from decoding.
fn analyze_cached(
    path: &std::path::Path,
    directory: &std::path::Path,
) -> (TrackAnalysis, f64, Duration) {
    analyze_cached_mode(path, directory, false)
}

/// Exercises cache reuse and explicit recomputation through the same production worker.
fn analyze_cached_mode(
    path: &std::path::Path,
    directory: &std::path::Path,
    force: bool,
) -> (TrackAnalysis, f64, Duration) {
    let file = File::open(path).unwrap();
    let key = SourceKey::read(&file, 0).unwrap();
    let config = Arc::new(crate::analysis_cache::CacheConfig::new(
        directory.to_path_buf(),
    ));
    let mut job =
        TrackJob::start_input(TrackInput::Retained(file, key), Some(config), force).unwrap();
    let started = Instant::now();
    loop {
        if let Some(result) = job.poll() {
            return (result.unwrap(), job.progress(), started.elapsed());
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "cached job deadline"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// A real recording's retained-stream result survives another job and off-track badge reopening.
#[test]
#[ignore = "requires RESOUL_ANALYSIS_PROBE pointing to an authorized local recording"]
fn authorized_recording_reopens_without_decoding_after_another_track() {
    let path = std::path::PathBuf::from(
        std::env::var_os("RESOUL_ANALYSIS_PROBE").expect("authorized input required"),
    );
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path().join("cache");
    let config = Arc::new(crate::analysis_cache::CacheConfig::new(cache));
    let file = File::open(&path).unwrap();
    let key = SourceKey::read(&file, 0).unwrap();
    let retained = file.try_clone().unwrap();
    let mut first = TrackJob::start(file, key, Some(config.clone())).unwrap();
    let original = finish_probe(&mut first);
    assert!(first.progress() > 0.0);

    let other = directory.path().join("other.wav");
    source_wav(&other, 1);
    let mut second =
        TrackJob::start_input(TrackInput::Path(other), Some(config.clone()), false).unwrap();
    finish_probe(&mut second);

    let started = Instant::now();
    let mut reopened = TrackJob::start_input(TrackInput::Path(path), Some(config), false).unwrap();
    let loaded = finish_probe(&mut reopened);
    assert_eq!(
        reopened.progress(),
        0.0,
        "badge reopening must not decode the original again"
    );
    assert_eq!(loaded.power_db, original.power_db);
    assert_eq!(loaded.provenance, original.provenance);
    assert_eq!(SourceKey::read(&retained, 0).unwrap(), key);
    println!(
        "cached badge reopened in {:.3} s, decoded {:.1} s",
        started.elapsed().as_secs_f64(),
        reopened.progress()
    );
}

/// Waits for an explicitly authorized real-file probe using the production analysis deadline.
fn finish_probe(job: &mut TrackJob) -> TrackAnalysis {
    let deadline = Instant::now() + DEADLINE + Duration::from_secs(95);
    loop {
        if let Some(result) = job.poll() {
            return result.unwrap();
        }
        assert!(Instant::now() < deadline, "authorized cache probe deadline");
        thread::sleep(Duration::from_millis(5));
    }
}

/// A retagged original reuses durable measurements without any PCM/FFT pass; changed audio forces decoding.
#[test]
fn persistent_analysis_reuses_retagged_audio_and_rejects_replaced_content() {
    let directory =
        std::env::temp_dir().join(format!("resolibre-cache-analysis-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("source.wav");
    let original = directory.join("original.flac");
    let renamed = directory.join("renamed.flac");
    let cache = directory.join("cache");
    source_wav(&source, 12);
    encode(
        &source,
        &original,
        &["-c:a", "flac", "-metadata", "title=Before"],
    );
    let original_bytes = std::fs::read(&original).unwrap();
    let (first, progress, cold) = analyze_cached(&original, &cache);
    assert!(progress > 10.0);
    encode(
        &original,
        &renamed,
        &[
            "-c",
            "copy",
            "-metadata",
            "title=After",
            "-metadata",
            "artist=Changed",
        ],
    );
    let (second, progress, warm) = analyze_cached(&renamed, &cache);
    assert_eq!(
        progress, 0.0,
        "a content cache hit must not decode or run FFT"
    );
    assert_eq!(first.power_db, second.power_db);
    assert_eq!(first.tempo, second.tempo);
    println!("persistent analysis cold={cold:?}, retagged cache hit={warm:?}");
    encode(&source, &renamed, &["-af", "volume=0.1", "-c:a", "flac"]);
    let (changed, progress, _) = analyze_cached(&renamed, &cache);
    assert!(progress > 10.0);
    assert_ne!(first.power_db, changed.power_db);
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
    std::fs::remove_dir_all(directory).unwrap();
}

/// Missing/corrupt storage falls back to ordinary analysis; optional persistence cannot break playback work.
#[test]
fn persistent_analysis_storage_failure_remains_retryable() {
    let directory =
        std::env::temp_dir().join(format!("resolibre-cache-failure-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("source.wav");
    source_wav(&source, 2);
    let invalid_directory = directory.join("not-a-directory");
    std::fs::write(&invalid_directory, b"keep unchanged").unwrap();
    let (_, progress, _) = analyze_cached(&source, &invalid_directory);
    assert!(progress > 1.0);
    assert_eq!(
        std::fs::read(&invalid_directory).unwrap(),
        b"keep unchanged"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// Measures a song-length synthetic source with cold analysis and a reopened persistent cache.
#[test]
#[ignore = "song-length cache timing probe; generated synthetic audio only"]
fn persistent_cache_song_length_timing() {
    let directory =
        std::env::temp_dir().join(format!("resolibre-cache-timing-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("source.wav");
    let compressed = directory.join("source.flac");
    let cache = directory.join("cache");
    source_wav(&source, 240);
    encode(&source, &compressed, &["-c:a", "flac"]);
    let (_, decoded, cold) = analyze_cached(&compressed, &cache);
    assert_eq!(decoded, 240.0);
    let (_, decoded, warm) = analyze_cached(&compressed, &cache);
    assert_eq!(decoded, 0.0);
    println!(
        "240-second synthetic FLAC: bytes={}, cold={cold:?}, persistent hit={warm:?}",
        std::fs::metadata(&compressed).unwrap().len()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// Both current and provisional next-track jobs consult durable rhythm, while whole coverage never uses a prefix.
#[test]
fn persistent_rhythm_respects_coverage_and_selected_stream() {
    use crate::rhythm_analysis::RhythmJob;
    use resolibre_core::analysis::tempo::TempoReport;
    use resolibre_storage::{AnalysisCache, audio_identity};
    let directory =
        std::env::temp_dir().join(format!("resolibre-cache-rhythm-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("source.wav");
    source_wav(&source, 12);
    let cache_directory = directory.join("cache");
    let identity =
        audio_identity::identify(File::open(&source).unwrap(), &AtomicBool::new(false)).unwrap();
    let mut database = AnalysisCache::open(&cache_directory).unwrap();
    // A valid but deliberately different cached estimate makes accidental recalculation observable.
    let cached_report = TempoReport {
        activity: Default::default(),
        bpm: Some(137.0),
        variable: false,
        segments: Vec::new(),
    };
    database
        .save_rhythm(&identity, false, &cached_report)
        .unwrap();
    drop(database);
    for (stream, whole) in [(0, false), (u32::MAX, false), (0, true)] {
        let file = File::open(&source).unwrap();
        let key = SourceKey::read(&file, stream).unwrap();
        let config = Arc::new(crate::analysis_cache::CacheConfig::new(
            cache_directory.clone(),
        ));
        let mut job = RhythmJob::start(file, key, whole, Some(config)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(result) = job.poll() {
                let (key, report) = result.unwrap();
                assert_eq!(
                    key.stream, 0,
                    "lookahead must resolve the actual native stream"
                );
                if whole {
                    assert!((report.bpm.unwrap() - 120.0).abs() < 1.0);
                } else {
                    assert_eq!(report, cached_report);
                }
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
    }
    let database = AnalysisCache::open(&cache_directory).unwrap();
    assert!(
        (database
            .rhythm(&identity, true)
            .unwrap()
            .unwrap()
            .bpm
            .unwrap()
            - 120.0)
            .abs()
            < 1.0
    );
    drop(database);
    std::fs::remove_dir_all(directory).unwrap();
}

/// An explicit retry decodes again and atomically replaces both the full plot and older rhythm coverage.
#[test]
fn forced_analysis_replaces_cached_measurements() {
    let directory =
        std::env::temp_dir().join(format!("resolibre-forced-analysis-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("source.wav");
    source_wav(&source, 12);
    let cache = directory.join("cache");
    let (original, _, _) = analyze_cached(&source, &cache);
    assert!(original.tempo.bpm.is_some());
    let identity = resolibre_storage::audio_identity::identify(
        File::open(&source).unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut database = resolibre_storage::AnalysisCache::open(&cache).unwrap();
    let mut old = original.clone();
    old.tempo = Default::default();
    database.save_track(&identity, &old).unwrap();
    database.save_rhythm(&identity, true, &old.tempo).unwrap();
    drop(database);
    let (cached, progress, _) = analyze_cached(&source, &cache);
    assert_eq!(progress, 0.0);
    assert_eq!(cached.tempo, old.tempo);
    let (fresh, progress, _) = analyze_cached_mode(&source, &cache, true);
    assert!(
        progress >= 12.0,
        "explicit Analyze must run the decoder again"
    );
    assert_eq!(fresh.tempo, original.tempo);
    let database = resolibre_storage::AnalysisCache::open(&cache).unwrap();
    assert_eq!(
        database.track(&identity).unwrap().unwrap().tempo,
        fresh.tempo
    );
    assert_eq!(
        database.rhythm(&identity, true).unwrap().unwrap(),
        fresh.tempo
    );
    drop(database);
    std::fs::remove_dir_all(directory).unwrap();
}
