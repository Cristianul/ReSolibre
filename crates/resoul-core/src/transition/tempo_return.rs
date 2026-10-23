//! One source-clock curve survives handover; paused playback cannot advance it.

use super::is_supported_mix_speed;

/// Begin restoring tempo this many outgoing source seconds before handover.
pub const TEMPO_RETURN_LEAD_SECONDS: f64 = 1.0;
/// Total playing time for the return, including the portion after handover.
pub const TEMPO_RETURN_DURATION_SECONDS: f64 = 6.0;

#[derive(Clone, Copy, Debug)]
pub struct TempoReturn {
    start_position: f64,
    initial_speed: f64,
    duration: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct TempoReturnFrame {
    pub speed: f64,
    pub complete: bool,
}

impl TempoReturn {
    /// Anchors a bounded curve to the incoming source; rejects invalid clocks, rates or tuning.
    pub(super) fn new(start_position: f64, initial_speed: f64) -> Option<Self> {
        let duration = TEMPO_RETURN_DURATION_SECONDS;
        if !start_position.is_finite()
            || start_position < 0.0
            || !is_supported_mix_speed(initial_speed)
            || !duration.is_finite()
            || duration <= 0.0
        {
            return None;
        }
        let curve = Self {
            start_position,
            initial_speed,
            duration,
        };
        (curve.end_position().is_finite() && curve.end_position() > start_position).then_some(curve)
    }

    /// Returns the incoming source position reached after the complete playing-time duration.
    pub fn end_position(self) -> f64 {
        self.start_position + self.duration * (self.initial_speed + 1.0) / 2.0
    }

    /// Samples the same continuous curve before and after handover; invalid clocks yield no update.
    pub fn frame(self, position: f64) -> Option<TempoReturnFrame> {
        if !position.is_finite() || position < 0.0 {
            return None;
        }
        if position <= self.start_position {
            return Some(TempoReturnFrame {
                speed: self.initial_speed,
                complete: false,
            });
        }
        if position >= self.end_position() {
            return Some(TempoReturnFrame {
                speed: 1.0,
                complete: true,
            });
        }

        // Invert the integral of speed: source seconds differ from playing seconds.
        // Positive supported speeds make this monotonic; 32 steps bound work and timing error.
        let distance = position - self.start_position;
        let mut lower = 0.0;
        let mut upper = 1.0;
        for _ in 0..32 {
            let middle = (lower + upper) / 2.0;
            if self.distance_at(middle) < distance {
                lower = middle;
            } else {
                upper = middle;
            }
        }
        let progress = (lower + upper) / 2.0;
        let smooth = progress * progress * (3.0 - 2.0 * progress);
        let speed = self.initial_speed + (1.0 - self.initial_speed) * smooth;
        Some(TempoReturnFrame {
            speed,
            complete: false,
        })
    }

    /// Integrates the smoothstep speed curve over a normalized playing-time interval.
    fn distance_at(self, progress: f64) -> f64 {
        let integral = progress.powi(3) - progress.powi(4) / 2.0;
        self.duration * (self.initial_speed * progress + (1.0 - self.initial_speed) * integral)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both directions take the configured playing time, remain monotonic and have gentle endpoints.
    #[test]
    fn duration_and_continuity_follow_playing_time() {
        for speed in [0.8, 1.0, 1.2] {
            let curve = TempoReturn::new(10.0, speed).unwrap();
            let mut position = 10.0;
            let mut elapsed = 0.0;
            let mut previous = curve.frame(position).unwrap();
            while !previous.complete {
                position += previous.speed * 0.001;
                elapsed += 0.001;
                let frame = curve.frame(position).unwrap();
                assert!((frame.speed - previous.speed).abs() < 0.00008);
                assert!((frame.speed - 1.0).abs() <= (previous.speed - 1.0).abs());
                previous = frame;
                assert!(elapsed < TEMPO_RETURN_DURATION_SECONDS + 0.01);
            }
            assert!((elapsed - TEMPO_RETURN_DURATION_SECONDS).abs() < 0.002);
            assert_eq!(previous.speed, 1.0);
            for progress in [0.001, 0.25, 0.5, 0.75, 0.999] {
                let frame = curve.frame(10.0 + curve.distance_at(progress)).unwrap();
                let expected = speed + (1.0 - speed) * progress * progress * (3.0 - 2.0 * progress);
                assert!((frame.speed - expected).abs() < 1e-9);
            }
        }
    }

    /// Nonfinite clocks, invalid speeds and unrepresentable extents cannot produce a native speed update.
    #[test]
    fn invalid_inputs_are_rejected() {
        for position in [f64::NAN, f64::INFINITY, -1.0, f64::MAX] {
            assert!(TempoReturn::new(position, 1.1).is_none());
        }
        for speed in [f64::NAN, f64::INFINITY, 0.0, 0.79, 1.21] {
            assert!(TempoReturn::new(0.0, speed).is_none());
        }
        let curve = TempoReturn::new(10.0, 1.1).unwrap();
        for position in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(curve.frame(position).is_none());
        }
        assert_eq!(curve.frame(0.0).unwrap().speed, 1.1);
    }
}
