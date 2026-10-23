use super::*;
use crate::profile_storage;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    /// Creates only owned disposable application data; tests never resolve the user's profile.
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "resolibre-reset-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    /// Removes only this test fixture after all retained directory descriptors have been released.
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

/// The complete known profile clears while original music, unrelated profiles and lock inodes survive.
#[test]
fn clears_known_data_without_touching_music() {
    let fixture = Fixture::new();
    let profile = fixture.0.join("profile");
    let guard = profile_storage::share_profile(&profile).unwrap();
    let session = profile_storage::lock_session(&profile).unwrap();
    for folder in ["analysis", "eq-presets"] {
        fs::create_dir(profile.join(folder)).unwrap();
    }
    for name in [
        "session-v1.bin",
        "equalizer-v1.txt",
        ".session-v1.bin-42.tmp",
        "analysis/analysis.sqlite3",
        "analysis/analysis.sqlite3-journal",
        "eq-presets/preset-1.resolibre-eq",
    ] {
        fs::write(profile.join(name), b"application data").unwrap();
    }
    let music = fixture.0.join("original.flac");
    fs::write(&music, b"untouched music").unwrap();
    guard.make_exclusive().unwrap();
    clear(&profile).unwrap();
    assert_eq!(fs::read(music).unwrap(), b"untouched music");
    assert!(!profile.join("session-v1.bin").exists());
    assert_eq!(fs::read_dir(profile.join("analysis")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(profile.join("eq-presets")).unwrap().count(), 0);
    assert!(profile.join("session.lock").exists());
    assert!(profile_storage::share_profile(&profile).is_err());
    drop(session);
    drop(guard);
    assert!(profile_storage::share_profile(&profile).is_ok());
}

/// Shared profile users block reset; links, nested directories and unknown files fail before any deletion.
#[test]
fn rejects_competing_windows_and_unsafe_entries() {
    let fixture = Fixture::new();
    let first = profile_storage::share_profile(&fixture.0).unwrap();
    let second = profile_storage::share_profile(&fixture.0).unwrap();
    assert!(first.make_exclusive().is_err());
    drop(second);
    first.make_exclusive().unwrap();
    fs::write(
        fixture.0.join("session-v1.bin"),
        b"keep until validation passes",
    )
    .unwrap();
    fs::write(fixture.0.join("unknown.wav"), b"music").unwrap();
    assert!(clear(&fixture.0).is_err());
    assert!(fixture.0.join("session-v1.bin").exists());
    fs::remove_file(fixture.0.join("unknown.wav")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/tmp", fixture.0.join("analysis")).unwrap();
        assert!(clear(&fixture.0).is_err());
        fs::remove_file(fixture.0.join("analysis")).unwrap();
        std::os::unix::fs::symlink(
            fixture.0.join("session-v1.bin"),
            fixture.0.join("equalizer-v1.txt"),
        )
        .unwrap();
        assert!(clear(&fixture.0).is_err());
        assert!(fixture.0.join("session-v1.bin").exists());
        fs::remove_file(fixture.0.join("equalizer-v1.txt")).unwrap();
    }
    fs::hard_link(
        fixture.0.join("session-v1.bin"),
        fixture.0.join("equalizer-v1.txt"),
    )
    .unwrap();
    assert!(clear(&fixture.0).is_err());
}
