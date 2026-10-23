//! Bounded presentation of measured source-time intervals; gaps never inherit a neighbouring tempo.

use crate::TempoZone;
use resolibre_core::analysis::{MAX_SECONDS, TrackAnalysis};
use slint::{ModelRc, VecModel};

/// Builds at most 512 measured regions plus their gaps, normalized to the spectrogram's original duration.
pub(super) fn zones(report: &TrackAnalysis) -> ModelRc<TempoZone> {
    let duration = report.duration_seconds;
    if !duration.is_finite() || duration <= 0.0 || duration > MAX_SECONDS as f64 {
        return ModelRc::default();
    }
    let mut zones = Vec::new();
    let mut previous_end = 0.0;
    for segment in report.tempo.segments.iter().take(512) {
        let start = segment.start_seconds;
        let end = segment.end_seconds;
        if !start.is_finite()
            || !end.is_finite()
            || start < previous_end
            || start >= duration
            || end <= start
            || end > duration + 0.01
            || !segment.bpm.is_finite()
            || !(30.0..=300.0).contains(&segment.bpm)
        {
            continue;
        }
        if start > previous_end {
            zones.push(zone(previous_end, start, duration, None));
        }
        let end = end.min(duration);
        zones.push(zone(start, end, duration, Some(segment.bpm)));
        previous_end = end;
    }
    if previous_end < duration {
        zones.push(zone(previous_end, duration, duration, None));
    }
    ModelRc::new(VecModel::from(zones))
}

/// Formats one selectable region without rounding its geometry or treating missing beats as missing tempo.
fn zone(start: f64, end: f64, duration: f64, bpm: Option<f64>) -> TempoZone {
    let caption = bpm.map_or_else(|| "?".to_owned(), |bpm| format!("{bpm:.1}"));
    let tempo = bpm.map_or_else(
        || "BPM unavailable".to_owned(),
        |bpm| format!("≈ {bpm:.1} BPM · estimated"),
    );
    TempoZone {
        start: (start / duration) as f32,
        end: (end / duration) as f32,
        caption: caption.into(),
        details: format!("{}–{} · {tempo}", timestamp(start), timestamp(end)).into(),
        measured: bpm.is_some(),
    }
}

/// Shows bounded source times to tenths of a second, including hour-long recordings without truncation.
fn timestamp(seconds: f64) -> String {
    let tenths = (seconds * 10.0).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}
