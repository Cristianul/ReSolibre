//! Real-file tests use isolated profiles; never the user's saved session or music.

use super::*;
use resolibre_core::equalizer::Equalizer;
use resolibre_core::queue::RepeatMode;
use resolibre_core::session::Volume;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;

/// Creates unique private test storage without depending on wall-clock timing.
fn directory() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("resolibre-session-{}-{id}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}

/// Includes duplicate occurrences, unavailable Unicode/non-UTF8 references and manual EQ.
fn sample() -> StoredSession {
    let path = PathBuf::from("/missing/音楽.flac");
    StoredSession {
        audio: SavedSession {
            paths: vec![
                path.clone(),
                PathBuf::from(std::ffi::OsString::from_vec(b"/missing/\xff.mp3".to_vec())),
                path,
            ]
            .into(),
            current: Some(2),
            volume: Volume::from_percent(23.0).unwrap(),
            equalizer: Equalizer::new(true, [3.0; 10], -1.0).unwrap(),
            edges: Default::default(),
            mix_enabled: false,
            shuffle: true,
            repeat: RepeatMode::One,
        },
        interface: InterfaceState::default(),
    }
}

/// Waits for one owned bounded file operation, without silently accepting a stuck job.
fn loaded(store: &mut SessionStore) -> Result<StoredSession, &'static str> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(saved) = store.poll(false)? {
            return Ok(saved);
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
}

/// Flushes pending updates, including any change arriving during an older transaction.
fn flushed(store: &mut SessionStore) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while store.busy() {
        store.poll(true).unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
}

/// V1/V2/V3 migrate without enabling MIX; V4 validates every persisted processing/interface flag.
#[test]
fn processing_intent_migrates_and_rejects_unknown_modes() {
    use resolibre_core::playback_edges::{FadeMode, PlaybackEdges};
    let mut saved = sample();
    for fade in [FadeMode::Off, FadeMode::In, FadeMode::Out, FadeMode::Both] {
        saved.audio.edges = PlaybackEdges { trim: true, fade };
        let bytes = format::encode(&saved).unwrap();
        assert_eq!(format::decode(&bytes).unwrap(), saved);
        let mut v3 = bytes[..bytes.len() - 1].to_vec();
        v3[15] = b'3';
        assert_eq!(format::decode(&v3).unwrap(), saved);
        let mut v2 = bytes[..bytes.len() - 4].to_vec();
        v2[15] = b'2';
        assert_eq!(format::decode(&v2).unwrap(), saved);
        for offset in 1..=4 {
            let mut invalid = bytes.clone();
            let length = invalid.len();
            invalid[length - offset] = 2;
            assert!(format::decode(&invalid).is_err());
        }
        for (offset, value) in [(5, 4), (6, 2)] {
            let mut invalid = bytes.clone();
            let length = invalid.len();
            invalid[length - offset] = value;
            assert!(format::decode(&invalid).is_err());
        }
        let mut legacy = bytes[..bytes.len() - 6].to_vec();
        legacy[15] = b'1';
        let restored = format::decode(&legacy).unwrap();
        let mut expected = saved.clone();
        expected.audio.edges = PlaybackEdges::default();
        assert_eq!(restored, expected);
    }
}

/// MIX survives restart while every older session version defaults to disabled.
#[test]
fn mix_intent_roundtrips_and_old_profiles_stay_disabled() {
    let mut saved = sample();
    saved.audio.mix_enabled = true;
    let bytes = format::encode(&saved).unwrap();
    assert_eq!(format::decode(&bytes).unwrap(), saved);
    let mut old = bytes[..bytes.len() - 1].to_vec();
    old[15] = b'3';
    saved.audio.mix_enabled = false;
    assert_eq!(format::decode(&old).unwrap(), saved);
}

/// Strict decoding rejects all truncated records and invalid domain/path values atomically.
#[test]
fn format_roundtrip_and_rejection() {
    let saved = sample();
    let bytes = format::encode(&saved).unwrap();
    assert_eq!(format::decode(&bytes).unwrap(), saved);
    for (legacy, expected) in [(-24.0_f64, -2.0), (12.0, 2.0)] {
        let mut legacy_bytes = bytes[..bytes.len() - 6].to_vec();
        legacy_bytes[15] = b'1';
        legacy_bytes[26..34].copy_from_slice(&legacy.to_le_bytes());
        let mut expected_session = saved.clone();
        expected_session.audio.equalizer = Equalizer::new(true, [3.0; 10], expected).unwrap();
        assert_eq!(format::decode(&legacy_bytes).unwrap(), expected_session);
    }
    assert_eq!(
        format::decode(&format::encode(&StoredSession::default()).unwrap()).unwrap(),
        StoredSession::default()
    );
    for length in 0..bytes.len() {
        assert!(format::decode(&bytes[..length]).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(format::decode(&extra).is_err());
    let mut version = bytes.clone();
    version[15] = b'9';
    assert!(format::decode(&version).is_err());
    let mut invalid = bytes.clone();
    invalid[17..25].copy_from_slice(&f64::NAN.to_le_bytes());
    // The format header is 17 bytes; no NaN may reach Volume or EQ.
    assert!(format::decode(&invalid).is_err());
    assert!(format::decode(&vec![0; format::MAX_BYTES as usize + 1]).is_err());
    for (offset, replacement) in [
        (25, vec![2]), // EQ enable flag.
        (26, f64::INFINITY.to_le_bytes().to_vec()),
        (34, 13.0_f64.to_le_bytes().to_vec()), // Out-of-range band gain.
        (114, vec![2]),                        // Shuffle flag.
        (115, vec![3]),                        // Unknown repeat mode.
        (116, 3_u32.to_le_bytes().to_vec()),   // Invalid selected occurrence.
        (120, u32::MAX.to_le_bytes().to_vec()), // Huge occurrence count.
        (124, u32::MAX.to_le_bytes().to_vec()), // Huge path length.
        (128, vec![b'r']),                     // Relative path.
        (129, vec![0]),                        // NUL inside a path.
    ] {
        let mut invalid = bytes.clone();
        invalid[offset..offset + replacement.len()].copy_from_slice(&replacement);
        assert!(format::decode(&invalid).is_err());
    }
    let maximum = StoredSession {
        audio: SavedSession {
            paths: vec![PathBuf::from(format!("/{}", "a".repeat(1023))); 1024].into(),
            current: Some(1023),
            ..SavedSession::default()
        },
        interface: InterfaceState::default(),
    };
    assert_eq!(
        format::decode(&format::encode(&maximum).unwrap()).unwrap(),
        maximum
    );
    for paths in [
        vec![PathBuf::from("relative.mp3")],
        vec![PathBuf::from("https://host/audio")],
        vec![PathBuf::from("/bad\0name")],
        vec![PathBuf::from("/a"); 1025],
    ] {
        let invalid = StoredSession {
            audio: SavedSession {
                paths: paths.into(),
                ..saved.audio.clone()
            },
            ..saved.clone()
        };
        assert!(format::encode(&invalid).is_err());
    }
    let invalid = StoredSession {
        audio: SavedSession {
            current: Some(3),
            ..saved.audio.clone()
        },
        ..saved
    };
    assert!(format::encode(&invalid).is_err());
}

/// Autosave survives a new store, coalesces pending changes, preserves duplicates and locks writers.
#[test]
fn restart_latest_settings_and_profile_isolation() {
    let directory = directory();
    let mut store = SessionStore::start(directory.clone()).unwrap();
    assert_eq!(loaded(&mut store).unwrap(), StoredSession::default());
    let mut competing = SessionStore::start(directory.clone()).unwrap();
    assert!(loaded(&mut competing).is_err());
    assert!(!competing.busy());
    store.observe(sample());
    store.poll(false).unwrap();
    assert!(!directory.join(FILE_NAME).exists());
    let deadline = Instant::now() + Duration::from_secs(5);
    while store.busy() {
        store.poll(false).unwrap();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        format::decode(&fs::read(directory.join(FILE_NAME)).unwrap()).unwrap(),
        sample()
    );
    store.observe(StoredSession {
        audio: SavedSession {
            volume: Volume::from_percent(15.0).unwrap(),
            ..sample().audio
        },
        ..sample()
    });
    store.poll(true).unwrap();
    let latest = StoredSession {
        audio: SavedSession {
            volume: Volume::from_percent(0.0).unwrap(),
            ..sample().audio
        },
        interface: InterfaceState {
            spectrum: true,
            equalizer_visible: false,
            library_visible: false,
        },
    };
    store.observe(latest.clone());
    flushed(&mut store);
    assert_eq!(
        fs::metadata(directory.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(store);
    let mut restored = SessionStore::start(directory.clone()).unwrap();
    assert_eq!(loaded(&mut restored).unwrap(), latest);
    let other = self::directory();
    let mut isolated = SessionStore::start(other.clone()).unwrap();
    assert_eq!(loaded(&mut isolated).unwrap(), StoredSession::default());
    restored.observe(StoredSession::default());
    flushed(&mut restored);
    drop(restored);
    let mut empty = SessionStore::start(directory.clone()).unwrap();
    assert_eq!(loaded(&mut empty).unwrap(), StoredSession::default());
    drop(empty);
    drop(isolated);
    drop(competing);
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(other).unwrap();
}

/// Corruption is never replaced with empty defaults; failing writes retain the last good file.
#[test]
fn corrupt_and_failed_writes_preserve_previous_session() {
    let directory = directory();
    fs::write(directory.join(FILE_NAME), b"unknown version").unwrap();
    let mut store = SessionStore::start(directory.clone()).unwrap();
    assert!(loaded(&mut store).is_err());
    store.observe(sample());
    store.poll(true).unwrap();
    assert_eq!(
        fs::read(directory.join(FILE_NAME)).unwrap(),
        b"unknown version"
    );
    drop(store);
    let original = format::encode(&sample()).unwrap();
    fs::write(directory.join(FILE_NAME), &original).unwrap();
    let mut store = SessionStore::start(directory.clone()).unwrap();
    loaded(&mut store).unwrap();
    fs::write(
        directory.join(format!(".{FILE_NAME}-{}.tmp", std::process::id())),
        b"occupied",
    )
    .unwrap();
    store.observe(StoredSession::default());
    let deadline = Instant::now() + Duration::from_secs(5);
    while store.poll(true).is_ok() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(fs::read(directory.join(FILE_NAME)).unwrap(), original);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

/// Shared storage rejects links/FIFOs for both settings and locks without reading private targets.
#[test]
fn special_files_and_directories_are_rejected() {
    let directory = directory();
    for name in [FILE_NAME, "session.lock"] {
        // A prior rejected settings read still creates the independent regular lock file.
        let _ = fs::remove_file(directory.join(name));
        std::os::unix::fs::symlink("missing", directory.join(name)).unwrap();
        let mut store = SessionStore::start(directory.clone()).unwrap();
        assert!(loaded(&mut store).is_err());
        drop(store);
        fs::remove_file(directory.join(name)).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(directory.join(name))
                .status()
                .unwrap()
                .success()
        );
        let mut store = SessionStore::start(directory.clone()).unwrap();
        assert!(loaded(&mut store).is_err());
        drop(store);
        fs::remove_file(directory.join(name)).unwrap();
    }
    fs::remove_file(directory.join("session.lock")).ok();
    fs::remove_file(directory.join("profile.lock")).ok();
    fs::remove_dir(&directory).unwrap();
    std::os::unix::fs::symlink("missing", &directory).unwrap();
    let mut store = SessionStore::start(directory.clone()).unwrap();
    assert!(loaded(&mut store).is_err());
    drop(store);
    fs::remove_file(directory).unwrap();
}

/// Read-only and corrupt-session windows retain profile access, preventing reset beneath live readers.
#[test]
fn failed_session_load_retains_reset_protection() {
    let path = directory();
    let mut owner = SessionStore::start(path.clone()).unwrap();
    loaded(&mut owner).unwrap();
    let mut reader = SessionStore::start(path.clone()).unwrap();
    assert!(loaded(&mut reader).is_err());
    assert!(reader.profile_available());
    let (_, activity) = owner.reset_target().unwrap();
    assert!(activity.make_exclusive().is_err());
    drop(reader);
    activity.make_exclusive().unwrap();
    drop(activity);
    drop(owner);
    fs::write(path.join(FILE_NAME), b"corrupt").unwrap();
    let mut corrupt = SessionStore::start(path.clone()).unwrap();
    assert!(loaded(&mut corrupt).is_err());
    let (_, activity) = corrupt.reset_target().unwrap();
    activity.make_exclusive().unwrap();
    drop(activity);
    drop(corrupt);
    fs::remove_dir_all(path).unwrap();
}
