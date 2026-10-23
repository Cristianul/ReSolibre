//! Small original-audio activity measurements shared by rhythm and full-track analysis.

/// Stronger than ordinary TRIM: MIX can omit quiet edges below -40 dBFS peak.
const ACTIVE_AMPLITUDE: f64 = 0.01;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioActivity {
    pub analyzed_seconds: f64,
    pub first_sound: Option<f64>,
    pub last_sound: Option<f64>,
}

impl AudioActivity {
    /// Validates cached coverage and paired sound bounds before they can authorize a transition cut.
    pub fn is_valid(&self) -> bool {
        if !self.analyzed_seconds.is_finite()
            || !(0.0..=super::MAX_SECONDS as f64).contains(&self.analyzed_seconds)
        {
            return false;
        }
        match (self.first_sound, self.last_sound) {
            (None, None) => true,
            (Some(first), Some(last)) => {
                first.is_finite()
                    && last.is_finite()
                    && first >= 0.0
                    && last > first
                    && last <= self.analyzed_seconds
            }
            _ => false,
        }
    }

    /// Returns supported activity only; silence, isolated transients and malformed reports supply no cuts.
    pub fn bounds(&self) -> Option<crate::playback_edges::PlaybackRegion> {
        if !self.is_valid() {
            return None;
        }
        Some(crate::playback_edges::PlaybackRegion {
            start: self.first_sound?,
            end: self.last_sound?,
        })
    }
}

#[derive(Clone, Default)]
pub(super) struct ActivityAnalyzer {
    first: Option<f64>,
    last: Option<f64>,
    active_bins: usize,
}

impl ActivityAnalyzer {
    /// Records five-millisecond peak bins across all channels; opposite phases cannot cancel.
    pub(super) fn observe(&mut self, peak: f64, start: f64, end: f64) {
        if peak >= ACTIVE_AMPLITUDE {
            self.first.get_or_insert(start);
            self.last = Some(end);
            self.active_bins += 1;
        }
    }

    /// Isolated clicks and silent files supply no cuts; incomplete excerpts retain their actual coverage.
    pub(super) fn finish(self, duration: f64) -> AudioActivity {
        let supported = self.active_bins >= 20;
        AudioActivity {
            analyzed_seconds: duration,
            first_sound: if supported { self.first } else { None },
            last_sound: if supported {
                self.last.map(|end| end.min(duration))
            } else {
                None
            },
        }
    }
}
