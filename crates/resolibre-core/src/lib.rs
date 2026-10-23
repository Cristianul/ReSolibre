//! Portable playback values. This crate has no native, filesystem or GUI dependency.

#![forbid(unsafe_code)]

pub mod analysis;
pub mod equalizer;
pub mod queue;
pub mod session;
pub mod spectrum;
pub mod track_info;
pub mod transition;
pub mod volume_glide;

/// An absolute playback position in seconds, validated before reaching an engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeekPosition(f64);

impl SeekPosition {
    /// Accepts finite, nonnegative seconds; rejects invalid floating-point input.
    pub fn from_seconds(seconds: f64) -> Option<Self> {
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }

        Some(Self(seconds))
    }

    /// Returns the validated absolute position without changing it.
    pub fn seconds(self) -> f64 {
        self.0
    }
}

/// Engine lifecycle evidence; loading alone does not prove audible output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackEvent {
    Loaded,
    Ended,
}

/// PCM representation reported at a decoder or audio API boundary, not file bit depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    Unsigned8,
    Signed16,
    Signed32,
    Signed64,
    Float32,
    Float64,
    Unsigned8Planar,
    Signed16Planar,
    Signed32Planar,
    Signed64Planar,
    Float32Planar,
    Float64Planar,
    Unknown,
}

/// Engine-reported PCM format; channel count does not describe speaker placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u32,
    pub sample_format: SampleFormat,
}

/// Best-effort readings, not an atomic snapshot or a physical-device measurement.
/// Missing formats are normal before loading, during reconfiguration and after EOF.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioFormats {
    /// Source after decoding; this does not establish encoded precision or codec.
    pub source: Option<AudioFormat>,
    /// Data submitted to the audio API; the OS/device may convert it further.
    pub output: Option<AudioFormat>,
}

#[cfg(test)]
mod tests {
    use super::SeekPosition;

    /// Invalid positions must never cross the native engine boundary.
    #[test]
    fn rejects_invalid_seek_positions() {
        for seconds in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(SeekPosition::from_seconds(seconds), None);
        }
    }

    /// Absolute seeking preserves the requested value, including the start.
    #[test]
    fn preserves_valid_seek_positions() {
        for seconds in [0.0, 103.0, 0.125] {
            let position = SeekPosition::from_seconds(seconds).expect("valid position");
            assert_eq!(position.seconds(), seconds);
        }
    }
}

pub mod playback_edges;
