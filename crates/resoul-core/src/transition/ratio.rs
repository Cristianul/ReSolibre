//! Small, explicit pulse ratios; source BPM measurements are never rewritten.

use super::is_supported_mix_speed;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BeatRatio {
    #[default]
    OneToOne,
    OneToTwo,
    TwoToOne,
    OneToThree,
    ThreeToOne,
    TwoToThree,
    ThreeToTwo,
}

impl BeatRatio {
    /// Returns outgoing and incoming beat counts within one shared cycle.
    pub fn counts(self) -> (usize, usize) {
        match self {
            Self::OneToOne => (1, 1),
            Self::OneToTwo => (1, 2),
            Self::TwoToOne => (2, 1),
            Self::OneToThree => (1, 3),
            Self::ThreeToOne => (3, 1),
            Self::TwoToThree => (2, 3),
            Self::ThreeToTwo => (3, 2),
        }
    }

    /// Expresses incoming tempo in outgoing-beat units, preserving the original report's BPM.
    pub(super) fn comparable_incoming_bpm(self, incoming_bpm: f64) -> f64 {
        let (outgoing, incoming) = self.counts();
        incoming_bpm * outgoing as f64 / incoming as f64
    }

    /// Prefers unity; otherwise minimizes the largest speed adjustment across both streams.
    /// Unknown tempos and ratios outside the shared speed budget remain unavailable.
    pub(super) fn choose(outgoing_bpm: f64, incoming_bpm: f64) -> Option<Self> {
        if !outgoing_bpm.is_finite()
            || !(40.0..=240.0).contains(&outgoing_bpm)
            || !incoming_bpm.is_finite()
            || !(40.0..=240.0).contains(&incoming_bpm)
        {
            return None;
        }

        let mut best = None;
        let mut smallest_adjustment = f64::INFINITY;
        for ratio in [
            Self::OneToOne,
            Self::OneToTwo,
            Self::TwoToOne,
            Self::OneToThree,
            Self::ThreeToOne,
            Self::TwoToThree,
            Self::ThreeToTwo,
        ] {
            let speed = outgoing_bpm / ratio.comparable_incoming_bpm(incoming_bpm);
            if !is_supported_mix_speed(speed) || !is_supported_mix_speed(1.0 / speed) {
                continue;
            }
            if ratio == Self::OneToOne {
                return Some(ratio);
            }
            let adjustment = speed.max(1.0 / speed) - 1.0;
            if adjustment < smallest_adjustment {
                best = Some(ratio);
                smallest_adjustment = adjustment;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integer ratios win where appropriate; a shared 2:3 cycle is distinct from equal BPM.
    #[test]
    fn chooses_simple_bounded_ratios_without_clamping_off_grid() {
        for (outgoing, incoming, expected) in [
            (120.0, 140.0, BeatRatio::OneToOne),
            (100.0, 190.0, BeatRatio::OneToTwo),
            (190.0, 100.0, BeatRatio::TwoToOne),
            (80.0, 224.0, BeatRatio::OneToThree),
            (224.0, 80.0, BeatRatio::ThreeToOne),
            (120.0, 174.0, BeatRatio::TwoToThree),
            (174.0, 120.0, BeatRatio::ThreeToTwo),
            (100.0, 180.0, BeatRatio::OneToTwo),
        ] {
            assert_eq!(BeatRatio::choose(outgoing, incoming), Some(expected));
        }
        for (outgoing, incoming) in [
            (100.0, 124.0),
            (40.0, 240.0),
            (0.0, 120.0),
            (f64::NAN, 120.0),
            (120.0, f64::INFINITY),
        ] {
            assert_eq!(BeatRatio::choose(outgoing, incoming), None);
        }
    }
}
