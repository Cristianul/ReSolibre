//! Compact presentation of observed audio facts without provenance claims.

use resolibre_audio::session::{EdgeStatus, Snapshot};
use resolibre_core::session::SessionState;
use resolibre_core::track_info::{CompressionKind, StreamInfo};

/// Keeps initialization, scan and processing notices visible instead of optional facts.
pub(crate) fn show(window: &crate::PlayerWindow, snapshot: &Snapshot) {
    let show_info = matches!(snapshot.state, SessionState::Playing | SessionState::Paused)
        && snapshot.scanning.is_none()
        && matches!(snapshot.edge_status, EdgeStatus::Off | EdgeStatus::Ready);
    let info = snapshot.stream_info.filter(|_| show_info);
    let lossless = info.is_some_and(|info| info.codec.compression() == CompressionKind::Lossless);
    let mut text = info
        .map(|info| {
            let codec = if lossless {
                "Lossless"
            } else {
                info.codec.label()
            };
            let dimensions = dimensions(info);
            if dimensions.is_empty() {
                codec.to_owned()
            } else {
                format!("{codec} · {dimensions}")
            }
        })
        .unwrap_or_default();
    if !text.is_empty() {
        let report = match &snapshot.analysis {
            resolibre_audio::session::AnalysisStatus::Complete(report) => Some(&report.tempo),
            _ => match &snapshot.rhythm {
                resolibre_audio::session::RhythmStatus::Ready(report) => Some(report.as_ref()),
                _ => None,
            },
        };
        if let Some(report) = report {
            if let Some(bpm) = snapshot
                .position
                .or_else(|| (!report.variable).then_some(0.0))
                .and_then(|seconds| report.bpm_at(seconds))
            {
                text.push_str(&format!(" · ≈ {bpm:.1} BPM"));
            } else {
                text.push_str(" · BPM unavailable");
            }
        } else if matches!(
            snapshot.rhythm,
            resolibre_audio::session::RhythmStatus::Detecting
        ) {
            text.push_str(" · BPM analyzing…");
        } else {
            text.push_str(" · BPM unavailable");
        }
    }
    window.set_lossless(lossless);
    window.set_technical_details(text.strip_prefix("Lossless · ").unwrap_or("").into());
    window.set_technical_info(text.into());
}

/// Formats source dimensions with exact Hz precision; no decoder bit depth is inferred.
pub(crate) fn describe(info: StreamInfo) -> String {
    let dimensions = dimensions(info);
    if dimensions.is_empty() {
        info.codec.label().to_owned()
    } else {
        format!("{} · {dimensions}", info.codec.label())
    }
}

/// Formats observed rate, encoded precision and channel count for both the diagnostic view and compact strip.
fn dimensions(info: StreamInfo) -> String {
    let mut fields = Vec::new();
    if let Some(rate) = info.sample_rate_hz.filter(|rate| *rate > 0) {
        let khz = f64::from(rate) / 1000.0;
        let decimal = format!("{khz:.3}");
        let compact = decimal.trim_end_matches('0').trim_end_matches('.');
        fields.push(format!("{compact} kHz"));
    }
    if let Some(bits) = info.bits_per_sample {
        fields.push(format!("{bits}-bit"));
    }
    if let Some(channels) = info.channels.filter(|channels| *channels > 0) {
        let label = match channels {
            1 => "Mono".to_owned(),
            2 => "Stereo".to_owned(),
            count => format!("{count} channels"),
        };
        fields.push(label);
    }
    fields.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use resolibre_core::track_info::AudioCodec;

    /// Partial and multichannel observations remain honest without guessed layouts or precision.
    #[test]
    fn describes_only_observed_facts() {
        assert_eq!(describe(StreamInfo::default()), "Unknown codec");
        assert_eq!(
            describe(StreamInfo {
                bits_per_sample: None,
                codec: AudioCodec::Flac,
                sample_rate_hz: Some(44100),
                channels: Some(2),
            }),
            "FLAC · 44.1 kHz · Stereo"
        );
        assert_eq!(
            describe(StreamInfo {
                bits_per_sample: None,
                codec: AudioCodec::Pcm,
                sample_rate_hz: Some(11025),
                channels: Some(6),
            }),
            "PCM · 11.025 kHz · 6 channels"
        );
        assert_eq!(
            describe(StreamInfo {
                bits_per_sample: None,
                codec: AudioCodec::Mp3,
                sample_rate_hz: Some(0),
                channels: Some(1),
            }),
            "MP3 · Mono"
        );
    }

    /// Real snapshot wiring clears stale facts and leaves actionable state notices available.
    #[test]
    fn snapshots_control_information_visibility() {
        i_slint_backend_testing::init_no_event_loop();
        let window = crate::PlayerWindow::new().unwrap();
        let mut snapshot = Snapshot {
            state: SessionState::Playing,
            stream_info: Some(StreamInfo {
                bits_per_sample: None,
                codec: AudioCodec::Flac,
                sample_rate_hz: Some(48000),
                channels: Some(2),
            }),
            ..Default::default()
        };
        show(&window, &snapshot);
        assert_eq!(
            window.get_technical_info(),
            "Lossless · 48 kHz · Stereo · BPM analyzing…"
        );
        assert!(window.get_lossless());
        snapshot.stream_info.as_mut().unwrap().bits_per_sample = Some(24);
        snapshot.rhythm = resolibre_audio::session::RhythmStatus::Ready(std::sync::Arc::new(
            resolibre_core::analysis::tempo::TempoReport {
                bpm: Some(113.0),
                ..Default::default()
            },
        ));
        show(&window, &snapshot);
        assert!(window.get_technical_info().contains("24-bit"));
        assert!(window.get_technical_info().contains("113.0 BPM"));
        assert!(matches!(
            snapshot.analysis,
            resolibre_audio::session::AnalysisStatus::Idle
        ));
        snapshot.state = SessionState::Paused;
        show(&window, &snapshot);
        assert!(!window.get_technical_info().is_empty());
        snapshot.edge_status = EdgeStatus::Unavailable;
        show(&window, &snapshot);
        assert!(window.get_technical_info().is_empty());
        snapshot.edge_status = EdgeStatus::Off;
        snapshot.state = SessionState::Loading;
        show(&window, &snapshot);
        assert!(window.get_technical_info().is_empty());
        snapshot.state = SessionState::Playing;
        snapshot.stream_info = None;
        show(&window, &snapshot);
        assert!(window.get_technical_info().is_empty());
    }

    /// The compiled text binding gives errors priority over a known stream description.
    #[test]
    fn error_text_remains_visible_with_technical_information() {
        use i_slint_backend_testing::ElementHandle;
        use slint::ComponentHandle;
        i_slint_backend_testing::init_no_event_loop();
        let window = crate::PlayerWindow::new().unwrap();
        window.set_technical_info("FLAC · 44.1 kHz · Stereo".into());
        window.show().unwrap();
        assert!(
            ElementHandle::find_by_accessible_label(&window, "FLAC · 44.1 kHz · Stereo")
                .next()
                .is_some()
        );
        window.set_error_message("Playback failed; retry".into());
        assert!(
            ElementHandle::find_by_accessible_label(&window, "Playback failed; retry")
                .next()
                .is_some()
        );
        assert!(
            ElementHandle::find_by_accessible_label(&window, "FLAC · 44.1 kHz · Stereo")
                .next()
                .is_none()
        );
    }
    /// The strip shows measured tempo at the playhead and never turns an unsupported codec into a lossless badge.
    #[test]
    fn variable_bpm_tracks_position_and_lossless_badge_tracks_codec() {
        use resolibre_core::analysis::tempo::{BeatSegment, TempoReport};
        i_slint_backend_testing::init_no_event_loop();
        let window = crate::PlayerWindow::new().unwrap();
        let report = TempoReport {
            activity: Default::default(),
            bpm: None,
            variable: true,
            segments: vec![
                BeatSegment {
                    start_seconds: 0.0,
                    end_seconds: 32.0,
                    bpm: 120.0,
                    beats: vec![],
                    confidence: 0.8,
                    subdivision_ambiguous: false,
                    beat_confidence: None,
                },
                BeatSegment {
                    start_seconds: 64.0,
                    end_seconds: 96.0,
                    bpm: 140.0,
                    beats: vec![],
                    confidence: 0.8,
                    subdivision_ambiguous: false,
                    beat_confidence: None,
                },
            ],
        };
        let mut snapshot = Snapshot {
            state: SessionState::Playing,
            position: Some(10.0),
            stream_info: Some(StreamInfo {
                codec: AudioCodec::Flac,
                ..Default::default()
            }),
            rhythm: resolibre_audio::session::RhythmStatus::Ready(std::sync::Arc::new(report.clone())),
            ..Default::default()
        };
        show(&window, &snapshot);
        assert_eq!(window.get_technical_info(), "Lossless · ≈ 120.0 BPM");
        assert!(window.get_lossless());
        let mut analyzer = resolibre_core::analysis::TrackAnalyzer::new(8000, 1).unwrap();
        analyzer.push(&[0.0; 8000]).unwrap();
        let mut full = analyzer.finish().unwrap();
        full.tempo = report;
        snapshot.analysis =
            resolibre_audio::session::AnalysisStatus::Complete(std::sync::Arc::new(full));
        snapshot.rhythm = resolibre_audio::session::RhythmStatus::Detecting;
        show(&window, &snapshot);
        assert!(
            window.get_technical_info().contains("120.0 BPM"),
            "full Analyze keeps measured local BPM visible"
        );
        snapshot.position = Some(70.0);
        show(&window, &snapshot);
        assert!(window.get_technical_info().contains("140.0 BPM"));
        snapshot.position = Some(40.0);
        show(&window, &snapshot);
        assert!(window.get_technical_info().contains("BPM unavailable"));
        for codec in [
            AudioCodec::Mp3,
            AudioCodec::WavPack,
            AudioCodec::Dts,
            AudioCodec::Unknown,
        ] {
            snapshot.stream_info.as_mut().unwrap().codec = codec;
            show(&window, &snapshot);
            assert!(!window.get_lossless());
            assert!(window.get_technical_info().starts_with(codec.label()));
        }
        snapshot.state = SessionState::Stopped;
        show(&window, &snapshot);
        assert!(!window.get_lossless());
        assert!(window.get_technical_info().is_empty());
    }
}
