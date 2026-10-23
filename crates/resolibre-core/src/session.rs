//! Portable session lifecycle and application gain; no native or UI dependency.

/// User-facing volume percentage; the squared curve gives finer quiet control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume(f64);

impl Volume {
    /// Rejects non-finite values and amplification beyond unity.
    pub fn from_percent(percent: f64) -> Option<Self> {
        if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
            return None;
        }
        Some(Self(percent))
    }

    /// Returns the slider percentage, independent of the OS mixer.
    pub fn percent(self) -> f64 {
        self.0
    }

    /// Converts the perceptual slider to linear amplitude in [0, 1].
    pub fn amplitude(self) -> f64 {
        (self.0 / 100.0).powi(2)
    }
}

impl Default for Volume {
    /// Starts quietly at 50% slider position (25% amplitude), without persistence.
    fn default() -> Self {
        Self(50.0)
    }
}

/// Temporary, pitch-preserving transport scanning; never stored with session preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanDirection {
    Forward,
    Backward,
}

/// Back restarts after one playable second; earlier or unavailable timing uses queue history.
/// The start is the retained TRIM boundary, not necessarily the beginning of the source file.
pub fn previous_restarts(position: Option<f64>, playback_start: f64) -> bool {
    if !playback_start.is_finite() || playback_start < 0.0 {
        return false;
    }
    position.is_some_and(|position| position.is_finite() && position - playback_start >= 1.0)
}

/// Session lifecycle; an acknowledged command is distinct from loaded playback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionState {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    Stopped,
    Ended,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact one-second boundary is relative to playable audio, and invalid timing never seeks.
    #[test]
    fn previous_uses_playable_elapsed_time() {
        for start in [0.0, 10.0] {
            for elapsed in [0.0, 0.999] {
                assert!(!previous_restarts(Some(start + elapsed), start));
            }
            for elapsed in [1.0, 20.0] {
                assert!(previous_restarts(Some(start + elapsed), start));
            }
        }
        for position in [None, Some(f64::NAN), Some(f64::INFINITY), Some(-1.0)] {
            assert!(!previous_restarts(position, 0.0));
        }
        for start in [f64::NAN, f64::INFINITY, -1.0, 5.0] {
            assert!(!previous_restarts(Some(2.0), start));
        }
    }

    /// Invalid gain inputs cannot reach the engine or affect other applications.
    #[test]
    fn volume_is_bounded_and_perceptual() {
        for value in [f64::NAN, f64::INFINITY, -1.0, 101.0] {
            assert_eq!(Volume::from_percent(value), None);
        }
        assert_eq!(Volume::default().amplitude(), 0.25);
        assert_eq!(Volume::from_percent(0.0).unwrap().amplitude(), 0.0);
        assert_eq!(Volume::from_percent(100.0).unwrap().amplitude(), 1.0);
    }
}
