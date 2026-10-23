//! Private digital captures verify actual trim/fade output, including seek-reconstructed envelopes.
use crate::{
    AudioOutput, LocalPlayer,
    edge_analysis::{AnalysisJob, SourceKey},
};
use resolibre_core::{
    PlaybackEvent, SeekPosition,
    equalizer::Equalizer,
    playback_edges::{FadeMode, PlaybackEdges},
    session::Volume,
};
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

/// Measures one parent-generated signal without sending sound to the user's device.
#[test]
#[ignore = "run through tools/check_playback_edges.py"]
fn capture_edges() {
    let mode: u32 = std::env::var("RESOUL_EDGES_CASE").unwrap().parse().unwrap();
    assert!(mode <= 7);
    let settings = PlaybackEdges {
        trim: mode != 4,
        fade: match mode {
            0 | 6 | 7 => FadeMode::Off,
            1 => FadeMode::In,
            2 => FadeMode::Out,
            _ => FadeMode::Both,
        },
    };
    let mut player = LocalPlayer::open_with_equalizer(
        Path::new("private-input.wav"),
        AudioOutput::CapturePcm { resample: false },
        true,
        Volume::from_percent(100.0).unwrap(),
        Equalizer::default(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline);
        if player.try_event().unwrap() == Some(PlaybackEvent::Loaded) {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    while !player.prepare_equalizer(Equalizer::default()).unwrap() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let input = player.analysis_input().unwrap();
    let key = SourceKey::read(&input, player.audio_stream_index().unwrap()).unwrap();
    let mut job = AnalysisJob::start(input, key).unwrap();
    let analysis = loop {
        if let Some(result) = job.poll() {
            break result.unwrap();
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    };
    let region = analysis.region(settings);
    player
        .apply_edges(Some((region, settings.fade)), Equalizer::default(), true)
        .unwrap();
    if mode == 5 {
        player
            .seek_with_equalizer(
                SeekPosition::from_seconds(region.end - 0.5).unwrap(),
                Equalizer::default(),
            )
            .unwrap();
    }
    player.set_paused(false).unwrap();
    loop {
        assert!(Instant::now() < deadline);
        if player.poll_event().unwrap() == Some(PlaybackEvent::Ended) {
            break;
        }
    }
}
