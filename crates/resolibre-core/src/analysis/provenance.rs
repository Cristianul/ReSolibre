//! Evidence of earlier lossy encoding, never a certificate of recording provenance.

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ProvenanceReport {
    Unavailable,
    Unsupported,
    InsufficientAudio,
    Measured {
        lossy_score: f32,
        minimum_score: f32,
        maximum_score: f32,
        windows: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvenanceVerdict {
    LikelyLossy,
    NoLossyTraces,
    Inconclusive,
}

/// Restricts claims to the mono/stereo sample-rate range evaluated by the model author.
pub fn supports_format(rate: u32, channels: usize) -> bool {
    (44_100..=192_000).contains(&rate) && (1..=2).contains(&channels)
}

impl ProvenanceReport {
    /// Measured evidence must also match a supported, sufficiently long source in a cached full report.
    pub fn is_valid_for(self, rate: u32, channels: usize, seconds: f64) -> bool {
        self.is_valid()
            && (!matches!(self, Self::Measured { .. })
                || (supports_format(rate, channels) && seconds.is_finite() && seconds >= 2.0))
    }

    /// Applies conservative decision bands; scores are model outputs, not calibrated guarantees.
    pub fn verdict(self) -> ProvenanceVerdict {
        if !self.is_valid() {
            return ProvenanceVerdict::Inconclusive;
        }
        let Self::Measured {
            lossy_score,
            minimum_score,
            maximum_score,
            ..
        } = self
        else {
            return ProvenanceVerdict::Inconclusive;
        };
        if minimum_score <= 0.1 && maximum_score >= 0.9 {
            return ProvenanceVerdict::Inconclusive;
        }
        if lossy_score >= 0.9 {
            ProvenanceVerdict::LikelyLossy
        } else if lossy_score <= 0.1 {
            ProvenanceVerdict::NoLossyTraces
        } else {
            ProvenanceVerdict::Inconclusive
        }
    }

    /// Rejects malformed cached measurements before they can supply a provenance label.
    pub fn is_valid(self) -> bool {
        let Self::Measured {
            lossy_score,
            minimum_score,
            maximum_score,
            windows,
        } = self
        else {
            return true;
        };
        [lossy_score, minimum_score, maximum_score]
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
            && minimum_score <= lossy_score
            && lossy_score <= maximum_score
            && (1..=6).contains(&windows)
            && (windows != 1 || minimum_score == maximum_score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uncertainty, conflicting excerpts and malformed scores never certify losslessness.
    #[test]
    fn conservative_verdicts_reject_conflicts_and_invalid_values() {
        for (score, expected) in [
            (0.01, ProvenanceVerdict::NoLossyTraces),
            (0.6, ProvenanceVerdict::Inconclusive),
            (0.99, ProvenanceVerdict::LikelyLossy),
        ] {
            let report = ProvenanceReport::Measured {
                lossy_score: score,
                minimum_score: score,
                maximum_score: score,
                windows: 1,
            };
            assert!(report.is_valid());
            assert_eq!(report.verdict(), expected);
        }
        let conflicting = ProvenanceReport::Measured {
            lossy_score: 0.9,
            minimum_score: 0.05,
            maximum_score: 1.0,
            windows: 6,
        };
        assert_eq!(conflicting.verdict(), ProvenanceVerdict::Inconclusive);
        for score in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            let invalid = ProvenanceReport::Measured {
                lossy_score: score,
                minimum_score: score,
                maximum_score: score,
                windows: 1,
            };
            assert!(!invalid.is_valid());
            assert_eq!(invalid.verdict(), ProvenanceVerdict::Inconclusive);
        }
        assert_eq!(
            ProvenanceReport::InsufficientAudio.verdict(),
            ProvenanceVerdict::Inconclusive
        );
    }
}
