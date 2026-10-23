//! Windows session and storage regressions use owned profiles and native path units.

use super::*;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

/// Preserves Unicode and unpaired UTF-16 units while rejecting foreign/truncated sessions.
#[test]
fn windows_session_round_trip_and_version_isolation() {
    let mut units: Vec<_> = r"C:\music\".encode_utf16().collect();
    units.extend([0xd800, b'.' as u16, b'w' as u16, b'a' as u16, b'v' as u16]);
    let mut stored = StoredSession::default();
    stored.audio.paths = vec![
        PathBuf::from(r"C:\music\Muzică.wav"),
        PathBuf::from(OsString::from_wide(&units)),
    ]
    .into();
    stored.audio.current = Some(1);
    let encoded = format::encode(&stored).unwrap();
    assert_eq!(format::decode(&encoded).unwrap(), stored);
    for length in 0..encoded.len() {
        assert!(format::decode(&encoded[..length]).is_err());
    }
    let mut foreign = b"ReSolibre-Session-4\n".to_vec();
    foreign.extend_from_slice(&encoded[b"ReSolibre-Windows-Session-4\n".len()..]);
    assert!(format::decode(&foreign).is_err());
    assert!(crate::path_encoding::decode(&[1]).is_none());
    assert!(crate::path_encoding::decode(&[0, 0]).is_none());
}

/// Accepts the core's maximum native path length despite its larger UTF-16 representation.
#[test]
fn windows_session_preserves_maximum_path_length() {
    let mut stored = StoredSession::default();
    let maximum = format!("C:\\{}", "a".repeat(4093));
    stored.audio.paths = vec![PathBuf::from(&maximum)].into();
    let encoded = format::encode(&stored).unwrap();
    assert_eq!(format::decode(&encoded).unwrap(), stored);

    stored.audio.paths = vec![PathBuf::from(format!("{maximum}a"))].into();
    assert!(format::encode(&stored).is_err());
}

/// Atomic replacement persists the newest session and locks exclude a competing writer.
#[test]
fn windows_profile_round_trip_and_exclusive_writer() {
    let root = std::env::temp_dir().join(format!("resolibre-win-session-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let lease = profile_storage::lock_session(&root).unwrap();
    assert!(profile_storage::lock_session(&root).is_err());
    profile_storage::write(&root, FILE_NAME, b"first").unwrap();
    profile_storage::write(&root, FILE_NAME, b"second").unwrap();
    assert_eq!(
        profile_storage::read(&root, FILE_NAME, 32)
            .unwrap()
            .unwrap(),
        b"second"
    );
    assert!(profile_storage::read(&root, FILE_NAME, 1).is_err());
    drop(lease);
    std::fs::remove_dir_all(root).unwrap();
}
