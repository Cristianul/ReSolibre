use super::*;
use crate::AudioOutput;
use crate::session::{Command as SessionCommand, Session};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicU64;
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    originals: Vec<(PathBuf, Vec<u8>)>,
}

impl Fixture {
    /// Generates private tagged audio/cover fixtures; originals are verified byte-for-byte on release.
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "resolibre-metadata-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        ffmpeg(
            &directory,
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=16x16:d=0.1",
                "-frames:v",
                "1",
                "-threads",
                "1",
                "cover.png",
            ],
        );
        for (name, codec) in [("tagged.flac", "flac"), ("tagged.mp3", "libmp3lame")] {
            ffmpeg(
                &directory,
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:duration=3",
                    "-i",
                    "cover.png",
                    "-map",
                    "0:a",
                    "-map",
                    "1:v",
                    "-c:a",
                    codec,
                    "-c:v",
                    "copy",
                    "-disposition:v",
                    "attached_pic",
                    "-metadata",
                    "artist=Artist 音楽",
                    "-metadata",
                    "title=Night Drive",
                    "-metadata",
                    "album=Late Hours",
                    "-metadata",
                    "date=2024",
                    "-metadata",
                    "disc=2/3",
                    name,
                ],
            );
        }
        ffmpeg(
            &directory,
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=3",
                "-c:a",
                "flac",
                "plain.flac",
            ],
        );
        let originals = ["tagged.flac", "tagged.mp3", "plain.flac"]
            .into_iter()
            .map(|name| {
                let path = directory.join(name);
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        Self {
            directory,
            originals,
        }
    }
}

impl Drop for Fixture {
    /// Confirms metadata inspection never rewrote music, then removes only owned temporary inputs.
    fn drop(&mut self) {
        for (path, bytes) in &self.originals {
            assert_eq!(fs::read(path).unwrap(), *bytes);
        }
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

/// Invokes the existing native fixture dependency without displaying paths or raw diagnostics.
fn ffmpeg(directory: &Path, arguments: &[&str]) {
    let status = Command::new("ffmpeg")
        .current_dir(directory)
        .args([
            "-hide_banner",
            "-loglevel",
            "quiet",
            "-nostdin",
            "-threads",
            "1",
            "-filter_threads",
            "1",
        ])
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "metadata fixture generation failed");
}

/// Waits only for the explicitly owned job, with a deadline independent of its internal timeout.
fn finish(mut job: Job) -> Result<Details, ()> {
    let started = Instant::now();
    loop {
        if let Some(result) = job.poll() {
            return result;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    }
}

/// Tags are case-insensitive bounded plain text; missing titles and non-audio media never masquerade as tracks.
#[test]
fn labels_and_cover_selection_are_bounded() {
    let mut document = json!({"streams":[{"codec_type":"audio", "tags":{"TITLE":"Song", "ARTIST":"A\nB\u{202e}"}}]});
    assert_eq!(track_title(&document).as_deref(), Some("A B – Song"));
    document["format"] = json!({"tags":{"title":"Container", "artist":"X"}});
    assert_eq!(track_title(&document).as_deref(), Some("X – Container"));
    document["format"]["tags"]["title"] = json!("x".repeat(10000));
    assert_eq!(track_title(&document).unwrap().chars().count(), 164);
    assert!(
        track_title(&json!({"streams":[{"codec_type":"audio", "tags":{"artist":"A"}}]})).is_none()
    );
    assert!(track_title(&json!({"format":{"tags":{"title":"Image"}},"streams":[]})).is_none());
    let mut image =
        json!({"streams":[{"index":1,"width":512,"height":512,"disposition":{"attached_pic":1}}]});
    assert_eq!(cover_stream(&image), Some(1));
    image["streams"][0]["width"] = json!(50000);
    assert!(cover_stream(&image).is_none());
    image["streams"][0]["width"] = json!(512);
    image["streams"][0]["disposition"]["attached_pic"] = json!(0);
    assert!(
        cover_stream(&image).is_none(),
        "ordinary video is not album art"
    );
}

/// Real MP3/FLAC tags and embedded PNG decode identically; absent tags/art and malformed data fall back.
#[test]
fn native_metadata_and_artwork_preserve_sources() {
    let fixture = Fixture::new();
    for name in ["tagged.mp3", "tagged.flac"] {
        let details = finish(Job::start(fixture.directory.join(name), true).unwrap()).unwrap();
        assert_eq!(details.title.as_deref(), Some("Artist 音楽 – Night Drive"));
        assert_eq!(details.duration_seconds, Some(3));
        for expected in ["artist 音楽", "night drive", "late hours", "2024", "2/3"] {
            assert!(details.search_text.contains(expected));
        }
        let cover = details.cover.expect("embedded cover");
        assert_eq!(cover.rgba.len(), COVER_BYTES);
        let center = &cover.rgba[COVER_BYTES / 2..COVER_BYTES / 2 + 4];
        assert!(center[0] > 200 && center[1] < 20 && center[2] < 20 && center[3] == 255);
    }
    let plain = finish(Job::start(fixture.directory.join("plain.flac"), true).unwrap()).unwrap();
    assert!(plain.title.is_none() && plain.cover.is_none());
    assert_eq!(plain.duration_seconds, Some(3));
    let broken = fixture.directory.join("broken.flac");
    fs::write(&broken, b"not a media file").unwrap();
    assert!(finish(Job::start(broken, true).unwrap()).is_err());
    let link = fixture.directory.join("link.flac");
    std::os::unix::fs::symlink(fixture.directory.join("tagged.flac"), &link).unwrap();
    assert!(finish(Job::start(link, true).unwrap()).is_err());
    let fifo = fixture.directory.join("fifo.mp3");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    assert!(finish(Job::start(fifo, true).unwrap()).is_err());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let playlist = fixture.directory.join("external.mp3");
    fs::write(
        &playlist,
        format!(
            "#EXTM3U\nhttp://{}/secret\n",
            listener.local_addr().unwrap()
        ),
    )
    .unwrap();
    assert!(finish(Job::start(playlist, true).unwrap()).is_err());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let job = Job::start(fixture.directory.join("tagged.mp3"), true).unwrap();
    let started = Instant::now();
    drop(job);
    assert!(started.elapsed() < Duration::from_secs(1));
}

/// Playlist labels update before playback; changing/removing tracks clears current art and rejects old work.
#[test]
fn session_metadata_follows_occurrence_identity() {
    let fixture = Fixture::new();
    let session = Session::start(AudioOutput::Silent).unwrap();
    session
        .submit(SessionCommand::Replace(vec![
            fixture.directory.join("tagged.flac"),
            fixture.directory.join("plain.flac"),
        ]))
        .unwrap();
    let started = Instant::now();
    loop {
        let snapshot = session.snapshot();
        if snapshot.cover.is_some()
            && snapshot.tracks[0].title == "Artist 音楽 – Night Drive"
            && snapshot
                .tracks
                .iter()
                .all(|track| track.duration_seconds == Some(3))
        {
            for expected in ["tagged.flac", "late hours", "2024", "2/3"] {
                assert!(snapshot.tracks[0].search_text.contains(expected));
            }
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    }
    let revision = session.submit(SessionCommand::Next).unwrap();
    let started = Instant::now();
    loop {
        let snapshot = session.snapshot();
        if snapshot.revision >= revision && snapshot.current == Some(1) {
            assert!(snapshot.cover.is_none());
            assert_eq!(snapshot.tracks[1].title, "plain.flac");
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    }
    session.submit(SessionCommand::Previous).unwrap();
    let revision = session
        .submit(SessionCommand::Replace(vec![
            fixture.directory.join("plain.flac"),
        ]))
        .unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(300) {
        let snapshot = session.snapshot();
        if snapshot.revision >= revision {
            assert!(snapshot.cover.is_none());
            assert_eq!(snapshot.tracks[0].title, "plain.flac");
        }
        thread::sleep(Duration::from_millis(5));
    }
    session.request_shutdown();
    let started = Instant::now();
    while !session.is_finished() {
        assert!(started.elapsed() < Duration::from_secs(2));
        thread::sleep(Duration::from_millis(5));
    }
}

/// Duration accepts only bounded finite values and prefers audio timing over container/video timing.
#[test]
fn duration_validation_and_audio_precedence() {
    for value in [
        json!(null),
        json!("N/A"),
        json!("NaN"),
        json!("inf"),
        json!(-1),
        json!(1e100),
        json!({}),
    ] {
        assert_eq!(duration_seconds(&value), None);
    }
    assert_eq!(duration_seconds(&json!("0")), Some(0));
    assert_eq!(duration_seconds(&json!("3661.9")), Some(3661));
    assert_eq!(duration_seconds(&json!(u32::MAX)), Some(u32::MAX));
    let mut document = json!({
        "streams": [{"codec_type":"video", "duration":"900"}, {"codec_type":"audio", "duration":"61.5"}],
        "format": {"duration":"120"}
    });
    assert_eq!(track_duration(&document), Some(61));
    document["streams"][1]["duration"] = json!("N/A");
    assert_eq!(track_duration(&document), Some(120));
    document["streams"] = json!([{"codec_type":"video", "duration":"900"}]);
    assert_eq!(track_duration(&document), None);
}

/// Search excludes unrelated/private tags and bounds malformed fields with container precedence.
#[test]
fn search_fields_are_allowlisted_and_sanitized() {
    let document = json!({
        "format": {"tags": {"ALBUM": "Container", "year": "2020", "discnumber": "2", "comment": "private secret"}},
        "streams": [{"codec_type": "audio", "tags": {
            "album": "Ignored", "artist": "Only Artist", "album_artist": "Group", "date": "2021-04",
            "title": "x".repeat(10000), "disc": "3\n\u{202e}"
        }}]
    });
    let search = search_tags(&document);
    for expected in [
        "container",
        "2020",
        "2021-04",
        "2",
        "3",
        "only artist",
        "group",
    ] {
        assert!(search.contains(expected));
    }
    assert!(!search.contains("ignored") && !search.contains("private"));
    assert!(!search.contains('\u{202e}'));
    assert!(search.chars().count() <= 8 * 160 + 7);
    assert_eq!(search_tags(&json!({"streams": []})), "");
    assert!(
        search_tags(&json!({"streams": [{"codec_type": "audio", "tags": {"artist": "No title"}}]}))
            .contains("no title")
    );
}

/// Restart reuse needs no probe: a seeded valid record is returned, while changed files miss.
#[test]
fn persistent_metadata_reuses_unchanged_files_and_rejects_replacements() {
    let fixture = Fixture::new();
    let path = fixture.directory.join("plain.flac");
    let config = Arc::new(crate::analysis_cache::CacheConfig::new(
        fixture.directory.join("cache"),
    ));
    let first =
        finish(Job::start_cached(path.clone(), false, Some(config.clone())).unwrap()).unwrap();
    assert!(first.duration_seconds.is_some());
    let key = SourceKey::read(&std::fs::File::open(&path).unwrap(), 0).unwrap();
    let mut database = config.metadata_database().unwrap();
    let record = resolibre_storage::metadata::Metadata {
        title: Some("Stored label".into()),
        search_text: "stored label".into(),
        duration_seconds: Some(123),
    };
    database
        .save_metadata(&key.metadata_key(), &record)
        .unwrap();
    drop(database);
    let restored =
        finish(Job::start_cached(path.clone(), false, Some(config.clone())).unwrap()).unwrap();
    assert_eq!(restored.duration_seconds, Some(123));
    assert_eq!(restored.title.as_deref(), Some("Stored label"));
    let mut warm = Job::warm(
        vec![(7, path.clone()), (8, fixture.directory.join("missing"))],
        config.clone(),
    )
    .unwrap();
    let started = Instant::now();
    let records = loop {
        if let Some(result) = warm.poll() {
            break result.unwrap();
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].0, 7);
    assert_eq!(records[0].1.duration_seconds, Some(123));
    // A new inode with identical bytes is deliberately reprobed for container metadata.
    let copy = fixture.directory.join("copy.flac");
    fs::copy(&path, &copy).unwrap();
    let changed = finish(Job::start_cached(copy, false, Some(config)).unwrap()).unwrap();
    assert_eq!(changed.duration_seconds, first.duration_seconds);
    assert_ne!(changed.title, restored.title);
    assert!(inspect_cached(path, false, &AtomicBool::new(true), None).is_err());
}

/// Measures maximum-queue cache hydration independently of native decoding and UI/GPU costs.
#[test]
#[ignore = "isolated metadata startup timing"]
fn cached_library_startup_timing() {
    let fixture = Fixture::new();
    let path = fixture.directory.join("plain.flac");
    let config = Arc::new(crate::analysis_cache::CacheConfig::new(
        fixture.directory.join("cache"),
    ));
    finish(Job::start_cached(path.clone(), false, Some(config.clone())).unwrap()).unwrap();
    let entries = (0..1024).map(|id| (id, path.clone())).collect();
    let started = Instant::now();
    let mut job = Job::warm(entries, config).unwrap();
    let records = loop {
        if let Some(result) = job.poll() {
            break result.unwrap();
        }
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(records.len(), 1024);
    assert!(
        records
            .iter()
            .all(|(_, details)| details.duration_seconds.is_some() && details.cover.is_none())
    );
    println!(
        "cached 1024 occurrences: {} ms",
        started.elapsed().as_millis()
    );
    let status = fs::read_to_string("/proc/self/status").unwrap();
    for line in status
        .lines()
        .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
    {
        println!("{line}");
    }
}
