//! Honest presentation of detector evidence, separate from codec compression and spectral bandwidth.

use resolibre_core::analysis::provenance::{ProvenanceReport, ProvenanceVerdict};

/// Explains model scores without presenting them as a probability of recording authenticity.
pub(super) fn describe(report: ProvenanceReport) -> String {
    match report {
        ProvenanceReport::Measured { lossy_score, windows, .. } if report.is_valid() => {
            let verdict = match report.verdict() {
                ProvenanceVerdict::LikelyLossy => "Likely lossy source",
                ProvenanceVerdict::NoLossyTraces => "No lossy traces detected",
                ProvenanceVerdict::Inconclusive => "Inconclusive",
            };
            format!("{verdict} · lossy-model score {:.3}/1 · {windows} excerpts. Experimental detection; absence of traces does not prove lossless origin.", lossy_score)
        }
        ProvenanceReport::Unsupported => "Lossy-source detection: unsupported format (requires mono/stereo, 44.1–192 kHz).".into(),
        ProvenanceReport::InsufficientAudio => "Lossy-source detection: insufficient audible material (at least 2 seconds required).".into(),
        _ => "Lossy-source detection: unavailable; Analyze again to retry. Spectrum and BPM remain usable.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Even a zero model score carries an explicit limitation; invalid data never reaches a positive label.
    #[test]
    fn labels_distinguish_no_traces_from_verified_origin() {
        let report = ProvenanceReport::Measured {
            lossy_score: 0.0,
            minimum_score: 0.0,
            maximum_score: 0.0,
            windows: 1,
        };
        let text = describe(report);
        assert!(text.starts_with("No lossy traces detected"));
        assert!(text.contains("does not prove"));
        let invalid = ProvenanceReport::Measured {
            lossy_score: f32::NAN,
            minimum_score: 0.0,
            maximum_score: 1.0,
            windows: 1,
        };
        assert!(describe(invalid).contains("unavailable"));
        assert!(describe(ProvenanceReport::Unsupported).contains("44.1–192"));
    }
}
