//! Generated containers exercise real audio decoding with video disabled and originals unchanged.
//! Fixtures: FFmpeg lavfi blue 16x16 at 5 fps + 440 Hz sine at 48 kHz, duration 0.5 s.
//! MP4 uses MPEG-4/AAC; MKV remuxes both streams, M4A keeps AAC, video-only keeps MPEG-4.
//! WebM transcodes the same generated inputs using libvpx/libopus. No third-party media.

use crate::{AudioOutput, LocalPlayer, ffi};
use resolibre_core::PlaybackEvent;
use std::time::{Duration, Instant};

/// Asserts video selection and output remain disabled even when the container has a video stream.
fn assert_video_disabled(player: &LocalPlayer) {
    let mut configured = 1_i32;
    // SAFETY: the live handle and static name outlive this aligned FORMAT_FLAG output slot.
    let result = unsafe {
        ffi::mpv_get_property(
            player.handle.as_ptr(),
            c"vo-configured".as_ptr(),
            ffi::FORMAT_FLAG,
            (&mut configured as *mut i32).cast(),
        )
    };
    assert_eq!(result, 0);
    assert_eq!(configured, 0);

    let mut width = 0_i64;
    // SAFETY: FORMAT_INT64 writes only into this aligned i64; no video payload is requested.
    let result = unsafe {
        ffi::mpv_get_property(
            player.handle.as_ptr(),
            c"video-params/w".as_ptr(),
            ffi::FORMAT_INT64,
            (&mut width as *mut i64).cast(),
        )
    };
    assert_eq!(result, ffi::ERROR_PROPERTY_UNAVAILABLE);
}

/// MP4/MKV/WebM audio and M4A decode to EOF; a video-only file cannot start audio playback.
#[test]
fn containers_play_audio_only_and_preserve_sources() {
    let fixtures: &[(&str, &[u8], bool)] = &[
        (
            "mp4",
            include_bytes!("../tests/fixtures/tone-video.mp4"),
            true,
        ),
        (
            "mkv",
            include_bytes!("../tests/fixtures/tone-video.mkv"),
            true,
        ),
        (
            "webm",
            include_bytes!("../tests/fixtures/tone-video.webm"),
            true,
        ),
        ("m4a", include_bytes!("../tests/fixtures/tone.m4a"), true),
        (
            "silent.mp4",
            include_bytes!("../tests/fixtures/video-only.mp4"),
            false,
        ),
    ];
    for (extension, contents, has_audio) in fixtures {
        let path =
            std::env::temp_dir().join(format!("resolibre-media-{}.{extension}", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, contents).unwrap();
        drop(file);
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut player = LocalPlayer::open(&path, AudioOutput::Silent).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed_audio = false;
        loop {
            assert!(
                Instant::now() < deadline,
                "container playback timed out: {extension}"
            );
            match player.poll_event() {
                Ok(Some(PlaybackEvent::Ended)) => {
                    assert!(*has_audio);
                    assert!(observed_audio, "must actually decode audio: {extension}");
                    break;
                }
                Ok(_) => {
                    assert_video_disabled(&player);
                    if player.audio_formats().unwrap().output.is_some() {
                        observed_audio = true;
                    }
                }
                Err(error) => {
                    assert!(!has_audio, "{extension}: {error}");
                    assert!(!observed_audio);
                    break;
                }
            }
        }
        drop(player);
        assert_eq!(std::fs::read(&path).unwrap(), *contents);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        std::fs::remove_file(path).unwrap();
    }
}
