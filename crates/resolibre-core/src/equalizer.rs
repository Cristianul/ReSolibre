//! Shared ten-band EQ contract. Manual preamp only; no automatic gain or limiter.

pub mod preset;

pub const FREQUENCIES: [f64; 10] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
pub const BAND_MIN_DB: f64 = -12.0;
pub const BAND_MAX_DB: f64 = 12.0;
pub const PREAMP_MIN_DB: f64 = -2.0;
pub const PREAMP_MAX_DB: f64 = 2.0;
pub const BAND_Q: f64 = 1.4;

/// Validated portable settings; adapters never receive raw filter expressions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Equalizer {
    enabled: bool,
    bands: [f64; 10],
    preamp: f64,
}

impl Default for Equalizer {
    /// Starts bypassed and flat, without altering playback gain.
    fn default() -> Self {
        Self {
            enabled: false,
            bands: [0.0; 10],
            preamp: 0.0,
        }
    }
}

/// One vocabulary for panel, presets and future hotkeys/remotes.
#[derive(Clone, Copy, Debug)]
pub enum EqAction {
    Band { index: usize, db: f64 },
    Preamp(f64),
    Enabled(bool),
    Reset,
    Load(Equalizer),
}

impl EqAction {
    /// Rejects invalid user/stored input before it can consume native work.
    pub fn validate(self) -> Result<(), &'static str> {
        match self {
            Self::Band { index, db } if index >= 10 || !valid_db(db, BAND_MIN_DB, BAND_MAX_DB) => {
                Err("Invalid equalizer band")
            }
            Self::Preamp(db) if !valid_db(db, PREAMP_MIN_DB, PREAMP_MAX_DB) => {
                Err("Invalid equalizer preamp")
            }
            _ => Ok(()),
        }
    }

    /// Only adjacent updates to the same control may replace pending slider work.
    pub fn replaces(self, previous: Self) -> bool {
        match (self, previous) {
            (Self::Band { index: a, .. }, Self::Band { index: b, .. }) => a == b,
            (Self::Preamp(_), Self::Preamp(_)) => true,
            _ => false,
        }
    }
}

impl Equalizer {
    /// Validates an entire preset atomically, preserving no partially accepted values.
    pub fn new(enabled: bool, bands: [f64; 10], preamp: f64) -> Result<Self, &'static str> {
        for (index, db) in bands.iter().enumerate() {
            EqAction::Band { index, db: *db }.validate()?;
        }
        EqAction::Preamp(preamp).validate()?;
        Ok(Self {
            enabled,
            bands,
            preamp,
        })
    }

    /// Migrates valid version-one stored preamp values into the current range.
    /// Rejects corrupt legacy values before clamping; all other settings remain intact.
    pub fn from_saved_v1(
        enabled: bool,
        bands: [f64; 10],
        preamp: f64,
    ) -> Result<Self, &'static str> {
        if !valid_db(preamp, -24.0, 12.0) {
            return Err("Invalid saved equalizer preamp");
        }
        Self::new(enabled, bands, preamp.clamp(PREAMP_MIN_DB, PREAMP_MAX_DB))
    }

    /// Applies shared policy. Reset flattens gains and preserves bypass state.
    pub fn apply(&mut self, action: EqAction) -> Result<(), &'static str> {
        action.validate()?;
        match action {
            EqAction::Band { index, db } => self.bands[index] = db,
            EqAction::Preamp(db) => self.preamp = db,
            EqAction::Enabled(enabled) => self.enabled = enabled,
            EqAction::Reset => {
                self.bands = [0.0; 10];
                self.preamp = 0.0;
            }
            EqAction::Load(settings) => *self = settings,
        }
        Ok(())
    }

    /// Returns bypass state while retaining the user's stored curve.
    pub fn enabled(self) -> bool {
        self.enabled
    }
    /// Returns gains in dB in the shared frequency order.
    pub fn bands(self) -> [f64; 10] {
        self.bands
    }
    /// Returns the explicitly chosen preamp, never an automatically calculated correction.
    pub fn preamp(self) -> f64 {
        self.preamp
    }
    /// Returns effective gains: bypass targets unity without erasing the curve.
    pub fn effective(self) -> ([f64; 10], f64) {
        if self.enabled {
            (self.bands, self.preamp)
        } else {
            ([0.0; 10], 0.0)
        }
    }

    /// Builds one named starting curve. Loading a preset enables EQ with manual preamp at zero.
    pub fn preset(index: usize) -> Option<Self> {
        let bands = match index {
            0 => [0.0; 10],
            1 => [5.0, 4.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            2 => [-2.0, -2.0, -1.0, 0.0, 2.0, 3.0, 3.0, 1.0, 0.0, -1.0],
            3 => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0],
            _ => return None,
        };
        Some(Self {
            enabled: true,
            bands,
            preamp: 0.0,
        })
    }
}

pub const PRESET_NAMES: [&str; 4] = ["Flat", "Bass", "Voice", "Bright"];

/// Includes only centers strictly below Nyquist; unavailable bands retain their saved gain.
pub fn active_bands(sample_rate: u32) -> [bool; 10] {
    FREQUENCIES.map(|frequency| frequency < f64::from(sample_rate) / 2.0)
}

/// Accepts finite decibels in the closed range before arithmetic or serialization.
fn valid_db(value: f64, minimum: f64, maximum: f64) -> bool {
    value.is_finite() && (minimum..=maximum).contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Both endpoints are valid; legacy migration preserves curves and rejects corrupt values.
    #[test]
    fn symmetric_preamp_and_legacy_limits() {
        for db in [-2.0, 0.0, 2.0] {
            assert_eq!(Equalizer::new(true, [1.0; 10], db).unwrap().preamp(), db);
        }
        for (stored, expected) in [(-24.0, -2.0), (-1.5, -1.5), (0.0, 0.0), (12.0, 2.0)] {
            let migrated = Equalizer::from_saved_v1(false, [3.0; 10], stored).unwrap();
            assert_eq!(migrated.preamp(), expected);
            assert_eq!(migrated.bands(), [3.0; 10]);
            assert!(!migrated.enabled());
        }
        for invalid in [-24.01, 12.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(Equalizer::from_saved_v1(true, [0.0; 10], invalid).is_err());
        }
        assert!(Equalizer::from_saved_v1(true, [13.0; 10], -6.0).is_err());
    }

    /// Invalid whole updates and individual edits leave previous valid state intact.
    #[test]
    fn validation_and_bypass_preserve_manual_settings() {
        let mut eq = Equalizer::preset(1).unwrap();
        eq.apply(EqAction::Preamp(-2.0)).unwrap();
        let before = eq;
        for action in [
            EqAction::Band { index: 10, db: 0.0 },
            EqAction::Band {
                index: 0,
                db: f64::NAN,
            },
            EqAction::Preamp(f64::INFINITY),
            EqAction::Preamp(-2.01),
            EqAction::Preamp(2.01),
            EqAction::Preamp(f64::NAN),
        ] {
            assert!(eq.apply(action).is_err());
            assert_eq!(eq, before);
        }
        assert!(Equalizer::new(true, [13.0; 10], 0.0).is_err());
        eq.apply(EqAction::Enabled(false)).unwrap();
        assert_eq!(eq.effective(), ([0.0; 10], 0.0));
        assert_eq!(eq.bands(), before.bands());
        assert_eq!(eq.preamp(), -2.0);
        eq.apply(EqAction::Reset).unwrap();
        assert!(!eq.enabled());
        assert_eq!(eq.bands(), [0.0; 10]);
    }
    /// Low-rate playback omits invalid bands; preset values obey the same portable limits.
    #[test]
    fn rate_boundaries_and_presets() {
        assert_eq!(active_bands(8000).iter().filter(|v| **v).count(), 7);
        assert!(!active_bands(32000)[9]);
        assert!(active_bands(44100).iter().all(|v| *v));
        for index in 0..PRESET_NAMES.len() {
            let eq = Equalizer::preset(index).unwrap();
            assert!(Equalizer::new(eq.enabled(), eq.bands(), eq.preamp()).is_ok());
        }
        assert!(Equalizer::preset(99).is_none());
    }
}
