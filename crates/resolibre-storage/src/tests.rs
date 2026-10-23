use super::*;
use resolibre_core::analysis::{TrackAnalyzer, tempo::BeatSegment};
use std::{
    fs,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(std::path::PathBuf);

impl Fixture {
    /// Creates a unique owned test directory; tests never inspect the user's profile or music.
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "resolibre-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    /// Opens storage beneath a private child directory, matching the desktop composition.
    fn cache(&self) -> AnalysisCache {
        AnalysisCache::open(&self.0.join("cache")).unwrap()
    }
}

impl Drop for Fixture {
    /// Removes only artifacts created by this test instance.
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Supplies a stable audio digest with deliberately independent human-readable labels.
fn identity() -> AudioIdentity {
    AudioIdentity {
        digest: [42; 32],
        artist: "Artist".into(),
        title: "Title".into(),
    }
}

/// Makes a valid observed timeline whose coverage is controlled by the caller.
fn rhythm() -> TempoReport {
    TempoReport {
        activity: Default::default(),
        bpm: Some(120.0),
        variable: false,
        segments: vec![BeatSegment {
            start_seconds: 0.0,
            end_seconds: 2.0,
            bpm: 120.0,
            beats: vec![0.1, 0.6, 1.1, 1.6],
            confidence: 0.8,
            subdivision_ambiguous: false,
            beat_confidence: Some(0.8),
        }],
    }
}

/// Produces complete real analyzer output, avoiding a serializer-shaped mock spectrogram.
fn track() -> TrackAnalysis {
    let mut analyzer = TrackAnalyzer::new(8000, 1).unwrap();
    analyzer.push(&vec![0.0; 16000]).unwrap();
    analyzer.finish().unwrap()
}

/// Restart, retag, inconclusive results and coverage upgrades all share the same content authority.
#[test]
fn restart_reuses_results_and_keeps_labels_out_of_identity() {
    let fixture = Fixture::new();
    let mut cache = fixture.cache();
    let mut key = identity();
    let report = rhythm();
    cache.save_rhythm(&key, false, &report).unwrap();
    assert!(cache.rhythm(&key, true).unwrap().is_none());
    drop(cache);

    key.artist = "Revised artist".into();
    key.title = "Revised title".into();
    let mut cache = fixture.cache();
    assert_eq!(cache.rhythm(&key, false).unwrap(), Some(report.clone()));
    let labels: (String, String) = cache
        .connection
        .query_row("SELECT artist,title FROM analyses", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(labels, (key.artist.clone(), key.title.clone()));
    cache.save_rhythm(&key, true, &report).unwrap();
    assert_eq!(cache.rhythm(&key, true).unwrap(), Some(report));
    cache.save_track(&key, &track()).unwrap();
    drop(cache);
    let mut cache = fixture.cache();
    assert_eq!(cache.track(&key).unwrap().unwrap().duration_seconds, 2.0);
    key.digest[0] ^= 1;
    assert!(cache.track(&key).unwrap().is_none());
    cache
        .save_rhythm(&key, false, &TempoReport::default())
        .unwrap();
    assert_eq!(
        cache.rhythm(&key, false).unwrap(),
        Some(TempoReport::default())
    );
    cache.clear().unwrap();
    assert!(cache.rhythm(&key, false).unwrap().is_none());
}

/// File-fact hints cannot mark missing, corrupt, cleared or another profile's results as complete.
#[test]
fn track_source_hints_require_a_valid_full_report_in_the_same_profile() {
    let fixture = Fixture::new();
    let mut cache = fixture.cache();
    let key = identity();
    let facts = [7; 56];
    assert!(!cache.has_tracks().unwrap());
    cache.save_track_source(&facts, &key).unwrap();
    cache.save_rhythm(&key, true, &rhythm()).unwrap();
    assert!(
        !cache.has_track(&key).unwrap(),
        "rhythm alone has no full-analysis badge"
    );
    cache.save_track(&key, &track()).unwrap();
    drop(cache);

    let mut cache = fixture.cache();
    let hint = cache.track_source(&facts).unwrap().unwrap();
    assert_eq!(hint.digest, key.digest);
    assert!(cache.has_track(&hint).unwrap());
    assert!(cache.track_source(&[8; 56]).unwrap().is_none());
    assert!(cache.track_source(&[7; 55]).is_err());
    assert!(cache.save_track_source(&[7; 55], &key).is_err());
    let other_profile = Fixture::new();
    assert!(
        other_profile
            .cache()
            .track_source(&facts)
            .unwrap()
            .is_none()
    );
    cache.write(&key, 2, b"corrupt report").unwrap();
    assert!(!cache.has_track(&hint).unwrap());
    cache.save_track(&key, &track()).unwrap();
    cache.clear().unwrap();
    assert!(
        !cache.has_track(&hint).unwrap(),
        "cleared/evicted results cannot restore badges"
    );
}

/// Cached bytes are untrusted: invalid geometry, beats, JSON and unsupported schemas fail closed.
#[test]
fn corrupt_results_and_databases_do_not_authorize_measurements() {
    let fixture = Fixture::new();
    let mut cache = fixture.cache();
    let key = identity();
    let mut invalid = rhythm();
    invalid.segments[0].beats[1] = -1.0;
    assert!(cache.save_rhythm(&key, true, &invalid).is_err());
    cache
        .write(&key, 1, &serde_json::to_vec(&invalid).unwrap())
        .unwrap();
    assert!(cache.rhythm(&key, true).unwrap().is_none());
    cache.write(&key, 0, b"not json").unwrap();
    assert!(cache.rhythm(&key, false).unwrap().is_none());
    let mut invalid = track();
    invalid.columns = 513;
    cache
        .write(&key, 2, &serde_json::to_vec(&invalid).unwrap())
        .unwrap();
    assert!(cache.track(&key).unwrap().is_none());
    cache
        .connection
        .pragma_update(None, "user_version", 999)
        .unwrap();
    drop(cache);
    assert!(AnalysisCache::open(&fixture.0.join("cache")).is_err());
    fs::write(
        fixture.0.join("cache/analysis.sqlite3"),
        b"corrupt database",
    )
    .unwrap();
    assert!(AnalysisCache::open(&fixture.0.join("cache")).is_err());
}

/// Legacy reports are remeasured, and malformed activity can never authorize source-time cuts.
#[test]
fn activity_cache_migrates_by_reanalysis_and_validates_bounds() {
    let fixture = Fixture::new();
    let mut cache = fixture.cache();
    let key = identity();
    cache.save_rhythm(&key, true, &rhythm()).unwrap();
    cache
        .connection
        .pragma_update(None, "user_version", 1)
        .unwrap();
    drop(cache);
    let mut cache = fixture.cache();
    let version: i64 = cache
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3);
    assert!(cache.rhythm(&key, true).unwrap().is_none());
    cache.save_rhythm(&key, true, &rhythm()).unwrap();
    cache
        .connection
        .pragma_update(None, "user_version", 2)
        .unwrap();
    drop(cache);
    let mut cache = fixture.cache();
    assert!(
        cache.rhythm(&key, true).unwrap().is_none(),
        "old tempo results must be remeasured"
    );
    let mut legacy = serde_json::to_value(rhythm()).unwrap();
    legacy.as_object_mut().unwrap().remove("activity");
    cache
        .write(&key, 1, &serde_json::to_vec(&legacy).unwrap())
        .unwrap();
    assert!(cache.rhythm(&key, true).unwrap().is_none());
    let mut report = rhythm();
    report.activity.analyzed_seconds = 2.0;
    report.activity.first_sound = Some(0.1);
    report.activity.last_sound = Some(1.9);
    cache.save_rhythm(&key, true, &report).unwrap();
    assert_eq!(cache.rhythm(&key, true).unwrap(), Some(report.clone()));
    for (coverage, first, last) in [
        (f64::NAN, 0.1, 1.9),
        (2.0, -1.0, 1.9),
        (2.0, 0.1, 3.0),
        (2.0, 1.9, 0.1),
        (14_401.0, 0.1, 1.9),
    ] {
        report.activity.analyzed_seconds = coverage;
        report.activity.first_sound = Some(first);
        report.activity.last_sound = Some(last);
        assert!(cache.save_rhythm(&key, true, &report).is_err());
    }
}

/// Retention stays bounded and two profiles cannot see each other's audio results.
#[test]
fn retention_and_profile_isolation() {
    let first = Fixture::new();
    let second = Fixture::new();
    let mut cache = first.cache();
    let mut key = identity();
    for index in 0..=MAX_ROWS {
        key.digest[..8].copy_from_slice(&(index as u64).to_le_bytes());
        cache
            .save_rhythm(&key, false, &TempoReport::default())
            .unwrap();
    }
    let count: i64 = cache
        .connection
        .query_row("SELECT count(*) FROM analyses", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, MAX_ROWS as i64);
    assert!(second.cache().rhythm(&key, false).unwrap().is_none());
    assert!(cache.rhythm(&key, false).unwrap().is_some());
}

/// Links to private files or media are never accepted as cache databases or journals.
#[cfg(unix)]
#[test]
fn rejects_links_special_files_and_shared_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new();
    drop(fixture.cache());
    let path = fixture.0.join("cache/analysis.sqlite3");
    fs::remove_file(&path).unwrap();
    let source = fixture.0.join("original");
    fs::write(&source, b"unchanged music").unwrap();
    symlink(&source, &path).unwrap();
    assert!(AnalysisCache::open(&fixture.0.join("cache")).is_err());
    fs::remove_file(&path).unwrap();
    fs::hard_link(&source, &path).unwrap();
    assert!(AnalysisCache::open(&fixture.0.join("cache")).is_err());
    fs::remove_file(&path).unwrap();
    drop(fixture.cache());
    symlink(&source, fixture.0.join("cache/analysis.sqlite3-journal")).unwrap();
    assert!(AnalysisCache::open(&fixture.0.join("cache")).is_err());
    assert_eq!(fs::read(&source).unwrap(), b"unchanged music");
    fs::remove_file(fixture.0.join("cache/analysis.sqlite3-journal")).unwrap();
    fs::set_permissions(fixture.0.join("cache"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(AnalysisCache::open(&fixture.0.join("cache")).is_err());
}

/// Creates a tiny RIFF containing independent INFO tags and PCM samples, including odd chunk padding.
fn wav(title: &str, sample: i16) -> Vec<u8> {
    let mut body = b"WAVEfmt ".to_vec();
    body.extend_from_slice(&16_u32.to_le_bytes());
    body.extend_from_slice(&1_u16.to_le_bytes());
    body.extend_from_slice(&1_u16.to_le_bytes());
    body.extend_from_slice(&8000_u32.to_le_bytes());
    body.extend_from_slice(&16000_u32.to_le_bytes());
    body.extend_from_slice(&2_u16.to_le_bytes());
    body.extend_from_slice(&16_u16.to_le_bytes());
    let title = format!("{title}\0");
    let mut tags = b"INFOINAM".to_vec();
    tags.extend_from_slice(&(title.len() as u32).to_le_bytes());
    tags.extend_from_slice(title.as_bytes());
    if title.len() % 2 == 1 {
        tags.push(0);
    }
    body.extend_from_slice(b"LIST");
    body.extend_from_slice(&(tags.len() as u32).to_le_bytes());
    body.extend(tags);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&32000_u32.to_le_bytes());
    for _ in 0..16000 {
        body.extend_from_slice(&sample.to_le_bytes());
    }
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend(body);
    bytes
}

/// Metadata changes reuse the hash, PCM changes invalidate it, and cancellation never returns a partial key.
#[test]
fn hashes_audio_not_tags_or_paths_and_preserves_originals() {
    let fixture = Fixture::new();
    let path = fixture.0.join("misleading.flac");
    let original = wav("First title", 12);
    fs::write(&path, &original).unwrap();
    let cancel = AtomicBool::new(false);
    let first = audio_identity::identify(fs::File::open(&path).unwrap(), &cancel).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(first.title, "First title");
    fs::write(&path, wav("A longer updated title", 12)).unwrap();
    let updated = audio_identity::identify(fs::File::open(&path).unwrap(), &cancel).unwrap();
    assert_eq!(first.digest, updated.digest);
    assert_eq!(updated.title, "A longer updated title");
    fs::write(&path, wav("First title", 13)).unwrap();
    let changed = audio_identity::identify(fs::File::open(&path).unwrap(), &cancel).unwrap();
    assert_ne!(first.digest, changed.digest);
    cancel.store(true, Ordering::Relaxed);
    assert!(audio_identity::identify(fs::File::open(&path).unwrap(), &cancel).is_none());
    fs::write(&path, b"invalid media").unwrap();
    cancel.store(false, Ordering::Relaxed);
    assert!(audio_identity::identify(fs::File::open(&path).unwrap(), &cancel).is_none());
}

/// Real container remux/tag updates must preserve encoded audio identity across supported music formats.
#[test]
#[ignore = "requires ffmpeg; generates only owned synthetic media"]
fn real_formats_retag_without_reanalysis() {
    let fixture = Fixture::new();
    let source = fixture.0.join("source.wav");
    fs::write(&source, wav("Source", 12)).unwrap();
    let cancel = AtomicBool::new(false);
    for (extension, codec) in [
        ("flac", "flac"),
        ("mp3", "libmp3lame"),
        ("m4a", "aac"),
        ("ogg", "libvorbis"),
        ("opus", "libopus"),
    ] {
        let original = fixture.0.join(format!("original.{extension}"));
        let retagged = fixture.0.join(format!("retagged.{extension}"));
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(&source)
            .args([
                "-c:a",
                codec,
                "-metadata",
                "artist=Before",
                "-metadata",
                "title=Before",
            ])
            .arg(&original)
            .status()
            .unwrap();
        assert!(status.success(), "encode {extension}");
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(&original)
            .args([
                "-c",
                "copy",
                "-metadata",
                "artist=After",
                "-metadata",
                "title=A longer title",
                "-metadata:s:a:0",
                "artist=After",
                "-metadata:s:a:0",
                "title=A longer title",
            ])
            .arg(&retagged)
            .status()
            .unwrap();
        assert!(status.success(), "retag {extension}");
        let before = audio_identity::identify(fs::File::open(&original).unwrap(), &cancel)
            .unwrap_or_else(|| panic!("identity {extension}"));
        let after = audio_identity::identify(fs::File::open(&retagged).unwrap(), &cancel).unwrap();
        assert_eq!(before.digest, after.digest, "retagged {extension}");
        assert_eq!(after.artist, "After", "artist {extension}");
        assert_eq!(after.title, "A longer title", "title {extension}");
        if extension == "m4a" {
            // FFmpeg creates one version-0 edit list. Changing its media start must invalidate timing results.
            let mut edited = fs::read(&retagged).unwrap();
            let atom = edited
                .windows(4)
                .position(|bytes| bytes == b"elst")
                .unwrap();
            assert_eq!(edited[atom + 4], 0);
            let media_time = atom + 16;
            edited[media_time + 3] ^= 1;
            fs::write(&retagged, edited).unwrap();
            let edited =
                audio_identity::identify(fs::File::open(&retagged).unwrap(), &cancel).unwrap();
            assert_ne!(
                after.digest, edited.digest,
                "MP4 edit timing is audio interpretation"
            );
        }
    }
}

/// Oversized inputs, malformed chunk sizes and multiple audio programs never receive a partial/ambiguous key.
#[test]
fn identity_rejects_limits_and_malformed_containers() {
    let fixture = Fixture::new();
    let path = fixture.0.join("input");
    let file = fs::File::create(&path).unwrap();
    file.set_len(513 * 1024 * 1024).unwrap();
    drop(file);
    let cancel = AtomicBool::new(false);
    assert!(audio_identity::identify(fs::File::open(&path).unwrap(), &cancel).is_none());
    fs::write(&path, b"\0\0\0\x01ftyp\xff\xff\xff\xff\xff\xff\xff\xff").unwrap();
    assert!(audio_identity::identify(fs::File::open(&path).unwrap(), &cancel).is_none());
    let mut bytes = wav("test", 0);
    bytes.truncate(20);
    fs::write(&path, bytes).unwrap();
    assert!(audio_identity::identify(fs::File::open(&path).unwrap(), &cancel).is_none());
    let mut oversized_tags = wav("test", 0);
    oversized_tags.truncate(36);
    oversized_tags.extend_from_slice(b"LIST");
    oversized_tags.extend_from_slice(&(20 * 1024 * 1024_u32).to_le_bytes());
    oversized_tags.extend_from_slice(b"INFOINAM");
    oversized_tags.extend_from_slice(&(20 * 1024 * 1024_u32 - 12).to_le_bytes());
    oversized_tags.resize(20 * 1024 * 1024 + 44, 0);
    let size = oversized_tags.len() as u32 - 8;
    oversized_tags[4..8].copy_from_slice(&size.to_le_bytes());
    fs::write(&path, oversized_tags).unwrap();
    assert!(audio_identity::identify(fs::File::open(&path).unwrap(), &cancel).is_none());
}

/// Attached artwork is excluded, while multi-audio containers fall back rather than confusing stream identities.
#[test]
#[ignore = "requires ffmpeg; generated artwork and audio only"]
fn artwork_and_multiple_streams_are_not_audio_identity_shortcuts() {
    let fixture = Fixture::new();
    let source = fixture.0.join("source.wav");
    fs::write(&source, wav("Title", 12)).unwrap();
    let ppm = fixture.0.join("cover.ppm");
    let mut pixels = b"P6\n2 2\n255\n".to_vec();
    pixels.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
    fs::write(&ppm, pixels).unwrap();
    let original = fixture.0.join("original.flac");
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&source)
        .args(["-c:a", "flac"])
        .arg(&original)
        .status()
        .unwrap();
    assert!(status.success());
    let cover = fixture.0.join("cover.flac");
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&original)
        .arg("-i")
        .arg(&ppm)
        .args([
            "-map",
            "0:a",
            "-map",
            "1:v",
            "-c:a",
            "copy",
            "-c:v",
            "png",
            "-disposition:v",
            "attached_pic",
        ])
        .arg(&cover)
        .status()
        .unwrap();
    assert!(status.success());
    let cancel = AtomicBool::new(false);
    let before = audio_identity::identify(fs::File::open(&original).unwrap(), &cancel).unwrap();
    let after = audio_identity::identify(fs::File::open(&cover).unwrap(), &cancel).unwrap();
    assert_eq!(before.digest, after.digest);
    let multiple = fixture.0.join("multiple.mka");
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&source)
        .args(["-map", "0:a", "-map", "0:a", "-c:a", "flac"])
        .arg(&multiple)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(audio_identity::identify(fs::File::open(&multiple).unwrap(), &cancel).is_none());
}

/// Metadata survives reopening, stays profile-scoped and rejects malformed or oversized values.
#[test]
fn metadata_restart_bounds_and_corruption() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    let key = [1_u8; 56];
    let record = metadata::Metadata {
        title: Some("Title".into()),
        search_text: "artist\ntitle".into(),
        duration_seconds: Some(103),
    };
    let mut cache = fixture.cache();
    cache.prepare_metadata().unwrap();
    cache.save_metadata(&key, &record).unwrap();
    drop(cache);
    let cache = fixture.cache();
    assert_eq!(cache.metadata(&key).unwrap(), Some(record));
    assert!(cache.metadata(&[2; 56]).unwrap().is_none());
    let isolated = other.cache();
    isolated.prepare_metadata().unwrap();
    assert!(isolated.metadata(&key).unwrap().is_none());
    cache
        .connection
        .execute("UPDATE metadata_v1 SET duration=-1", [])
        .unwrap();
    assert!(cache.metadata(&key).is_err());
    cache
        .connection
        .execute("UPDATE metadata_v1 SET duration=5,title=char(8238)", [])
        .unwrap();
    assert!(cache.metadata(&key).unwrap().is_none());
    let mut cache = fixture.cache();
    let oversized = metadata::Metadata {
        title: None,
        search_text: "x".repeat(8193),
        duration_seconds: None,
    };
    assert!(cache.save_metadata(&key, &oversized).is_err());
    assert!(cache.metadata(&[]).is_err());
    cache
        .connection
        .execute_batch(
            "DELETE FROM metadata_v1;
        WITH RECURSIVE rows(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<4096)
        INSERT INTO metadata_v1 SELECT CAST(printf('%056d',n) AS BLOB),'old','old',1,0 FROM rows;",
        )
        .unwrap();
    let newest = metadata::Metadata {
        title: Some("new".into()),
        search_text: "new".into(),
        duration_seconds: Some(2),
    };
    cache.save_metadata(&key, &newest).unwrap();
    let count: i64 = cache
        .connection
        .query_row("SELECT count(*) FROM metadata_v1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 4096);
    assert_eq!(cache.metadata(&key).unwrap(), Some(newest));
}
