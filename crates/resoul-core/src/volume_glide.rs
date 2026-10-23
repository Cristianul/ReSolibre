//! Bounded, clock-independent application-volume transitions shared by all clients.

use crate::session::Volume;
use std::time::Duration;

/// Two-second smoothstep in dial percentage, with zero endpoint velocity.
pub struct VolumeGlide {
    start: Volume,
    target: Volume,
    elapsed: Duration,
}

impl VolumeGlide {
    /// Starts from the currently displayed volume; replacing a glide never jumps.
    pub fn new(start: Volume, target: Volume) -> Self {
        Self {
            start,
            target,
            elapsed: Duration::ZERO,
        }
    }

    /// Returns the accepted destination for persistence, independent of intermediate frames.
    pub fn target(&self) -> Volume {
        self.target
    }

    /// Advances by real elapsed time and returns the bounded value and completion flag.
    pub fn advance(&mut self, elapsed: Duration) -> (Volume, bool) {
        self.elapsed = self.elapsed.saturating_add(elapsed);
        let fraction = (self.elapsed.as_secs_f64() / 2.0).min(1.0);
        if fraction >= 1.0 {
            return (self.target, true);
        }
        let eased = fraction * fraction * (3.0 - 2.0 * fraction);
        let percent = self.start.percent() + (self.target.percent() - self.start.percent()) * eased;
        // Convex interpolation of validated endpoints stays in the volume domain.
        let volume = Volume::from_percent(percent.clamp(0.0, 100.0)).unwrap_or(self.target);
        (volume, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both directions ease at their endpoints, remain monotonic and finish at exactly two seconds.
    #[test]
    fn glide_is_bounded_eased_and_time_based() {
        for (start, end) in [(0.0, 100.0), (100.0, 0.0), (37.0, 37.0)] {
            let mut glide = VolumeGlide::new(
                Volume::from_percent(start).unwrap(),
                Volume::from_percent(end).unwrap(),
            );
            assert_eq!(
                glide.advance(Duration::ZERO),
                (Volume::from_percent(start).unwrap(), false)
            );
            let mut values = vec![start];
            for index in 1..=20 {
                let (value, complete) = glide.advance(Duration::from_millis(100));
                assert_eq!(complete, index == 20);
                values.push(value.percent());
            }
            assert_eq!(values[20], end);
            assert!((values[10] - (start + end) / 2.0).abs() < 1e-10);
            assert!((values[1] - values[0]).abs() <= (values[10] - values[9]).abs());
            assert!((values[20] - values[19]).abs() <= (values[10] - values[9]).abs());
            for pair in values.windows(2) {
                assert!((pair[1] - pair[0]) * (end - start) >= 0.0);
            }
            assert_eq!(glide.advance(Duration::MAX), (glide.target(), true));
        }
    }
}
