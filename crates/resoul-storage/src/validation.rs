//! Untrusted cached measurements must satisfy the same finite dimensions and timeline bounds as analysis.

use resolibre_core::analysis::{MAX_COLUMNS, MAX_SECONDS, ROWS, TrackAnalysis, tempo::TempoReport};

/// Validates a cached rhythm timeline before playback is allowed to use it for transitions.
pub(crate) fn tempo(report: &TempoReport) -> bool {
    if report.segments.len() > 512
        || !report.bpm.is_none_or(valid_bpm)
        || !report.activity.is_valid()
    {
        return false;
    }
    let mut previous_end = 0.0;
    for segment in &report.segments {
        if !segment.start_seconds.is_finite()
            || !segment.end_seconds.is_finite()
            || segment.start_seconds < previous_end
            || segment.end_seconds <= segment.start_seconds
            || segment.end_seconds > MAX_SECONDS as f64
            || !valid_bpm(segment.bpm)
            || !unit(segment.confidence)
            || !segment.beat_confidence.is_none_or(unit)
            || segment.beats.len() > 1024
            || (segment.beat_confidence.is_none() && !segment.beats.is_empty())
        {
            return false;
        }
        let mut previous_beat = None;
        for beat in &segment.beats {
            if !beat.is_finite()
                || *beat < segment.start_seconds
                || *beat > segment.end_seconds
                || previous_beat.is_some_and(|previous| *beat <= previous)
            {
                return false;
            }
            previous_beat = Some(*beat);
        }
        previous_end = segment.end_seconds;
    }
    true
}

/// Bounds spectrogram allocation, geometry and numeric values before the UI sees a cached report.
pub(crate) fn track(report: &TrackAnalysis) -> bool {
    (8000..=384000).contains(&report.sample_rate_hz)
        && (1..=32).contains(&report.channels)
        && report.duration_seconds.is_finite()
        && report.duration_seconds > 0.0
        && report.duration_seconds <= MAX_SECONDS as f64
        && (1..=MAX_COLUMNS).contains(&report.columns)
        && report.power_db.len() == report.columns * ROWS
        && report
            .power_db
            .iter()
            .all(|value| value.is_finite() && (-120.0..=100.0).contains(value))
        && report.seconds_per_column.is_finite()
        && report.seconds_per_column > 0.0
        && report.seconds_per_column <= MAX_SECONDS as f64
        && report.sustained_bandwidth_hz.is_none_or(|value| {
            value.is_finite() && (0.0..=f64::from(report.sample_rate_hz) / 2.0).contains(&value)
        })
        && tempo(&report.tempo)
        && report.tempo.activity.analyzed_seconds <= report.duration_seconds + 0.01
        && report
            .tempo
            .segments
            .last()
            .is_none_or(|segment| segment.end_seconds <= report.duration_seconds + 0.01)
}

/// Cached BPM must remain within the detector's supported search range.
fn valid_bpm(value: f64) -> bool {
    value.is_finite() && (30.0..=300.0).contains(&value)
}

/// Confidence values are bounded scores, never arbitrary floating-point inputs.
fn unit(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
