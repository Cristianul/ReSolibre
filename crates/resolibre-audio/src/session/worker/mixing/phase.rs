//! Bounded clock feedback for native startup latency and drift during an observed-beat overlap.

use resolibre_core::transition::{
    MAX_TEMPO_CHANGE, TransitionFrame, TransitionPlan, is_supported_mix_speed,
};
use std::time::{Duration, Instant};

/// Recheck reconstruction before spending another muted startup seek on measured latency.
const SEEK_SETTLE_TIME: Duration = Duration::from_millis(150);
const MAX_PHASE_SPEED_CHANGE: f64 = 0.02;
const PHASE_RECOVERY_SECONDS: f64 = 0.5;

#[cfg(test)]
mod native_tests;

#[derive(Default)]
pub(super) struct PhaseAlignment {
    seek_count: u8,
    seek_lead: f64,
    settle_until: Option<Instant>,
    observed_clock: bool,
}

#[derive(Debug, PartialEq)]
pub(super) enum PhaseAdjustment {
    Seek(f64),
    Follow { speed: f64, synchronized: bool },
}

/// Holds full outgoing gain during acquisition, then smoothly joins the planned complementary fade.
/// An expired acquisition returns no frame so optional MIX can stop before the outgoing handover.
pub(super) fn gate_gains(
    plan: TransitionPlan,
    position: f64,
    mut frame: TransitionFrame,
    released: Option<f64>,
) -> Option<TransitionFrame> {
    let deadline = (plan.outgoing_start + 1.0).min(plan.outgoing_end - 0.25);
    if let Some(released) = released {
        if released >= deadline {
            return None;
        }
        let blend = ((position - released) / (0.15 * frame.outgoing_speed)).clamp(0.0, 1.0);
        frame.incoming_gain *= blend;
    } else {
        if position >= deadline {
            return None;
        }
        frame.incoming_gain = 0.0;
    }
    frame.outgoing_gain = 1.0 - frame.incoming_gain;
    Some(frame)
}

impl PhaseAlignment {
    /// Rejects the prepared stream's old clock until its initial unpause has settled.
    pub(super) fn start(&mut self, now: Instant) {
        self.settle_until = Some(now + SEEK_SETTLE_TIME);
    }

    /// Allows clock-free TEMPO only after startup settles and before actual phase evidence exists.
    pub(super) fn permits_unverified_fallback(&self, now: Instant) -> bool {
        !self.observed_clock && self.settle_until.is_some_and(|deadline| now >= deadline)
    }

    /// Rechecks actual clocks after native seeks, then gently corrects remaining drift without audible seeks.
    /// Call only with available clocks outside native reconstruction; absent observations never verify sync.
    pub(super) fn observe(
        &mut self,
        wanted: f64,
        actual: f64,
        nominal_speed: f64,
        can_seek: bool,
        now: Instant,
    ) -> Option<PhaseAdjustment> {
        if !wanted.is_finite()
            || wanted < 0.0
            || !actual.is_finite()
            || actual < 0.0
            || !is_supported_mix_speed(nominal_speed)
        {
            return None;
        }
        if self.settle_until.is_some_and(|deadline| now < deadline) {
            return None;
        }
        self.observed_clock = true;

        let error = actual - wanted;
        let seek_threshold = if self.seek_count == 0 {
            0.005
        } else {
            0.030 * nominal_speed
        };
        if can_seek && self.seek_count < 3 && error.abs() > seek_threshold {
            // Seeking to "now" still leaves the incoming stream behind by reconstruction time.
            // Retain corrections so a faster warm seek can remove an excessive cold-seek allowance.
            if self.seek_count > 0 {
                self.seek_lead = (self.seek_lead - error).clamp(-0.5, 0.5);
            }
            let target = (wanted + self.seek_lead).max(0.0);
            self.seek_count += 1;
            self.settle_until = Some(now + SEEK_SETTLE_TIME);
            return Some(PhaseAdjustment::Seek(target));
        }

        // A command acknowledgement is not alignment. Keep comparing actual clocks throughout the overlap.
        // Source-clock error divided by recovery time yields a small, pitch-corrected speed adjustment.
        let correction = if error.abs() <= 0.003 {
            0.0
        } else {
            (-error / PHASE_RECOVERY_SECONDS).clamp(-MAX_PHASE_SPEED_CHANGE, MAX_PHASE_SPEED_CHANGE)
        };
        let speed =
            (nominal_speed + correction).clamp(1.0 - MAX_TEMPO_CHANGE, 1.0 + MAX_TEMPO_CHANGE);
        Some(PhaseAdjustment::Follow {
            speed,
            synchronized: error.abs() / nominal_speed <= 0.030,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Short overlaps hold current audio, bound acquisition before handover and release gain without a step.
    #[test]
    fn acquisition_stays_muted_and_releases_complementary_gains() {
        let mut plan = TransitionPlan {
            outgoing_start: 10.0,
            outgoing_end: 12.0,
            incoming_start: 1.0,
            outgoing_bpm: Some(240.0),
            incoming_bpm: Some(240.0),
            phase_matched: true,
            beat_ratio: resolibre_core::transition::BeatRatio::OneToOne,
        };
        for position in [9.8, 10.0, 10.5, 10.99] {
            let frame = gate_gains(plan, position, plan.frame(position), None).unwrap();
            assert_eq!(frame.incoming_gain, 0.0);
            assert_eq!(frame.outgoing_gain, 1.0);
        }
        assert!(gate_gains(plan, 11.0, plan.frame(11.0), None).is_none());
        // A stalled worker must not admit an incoming stream for the first time at or after the deadline.
        for late in [11.0, 11.9, 12.0] {
            assert!(gate_gains(plan, late, plan.frame(late), Some(late)).is_none());
        }
        for (position, blend) in [(10.5, 0.0), (10.575, 0.5), (10.65, 1.0), (11.0, 1.0)] {
            let nominal = plan.frame(position);
            let frame = gate_gains(plan, position, nominal, Some(10.5)).unwrap();
            assert!((frame.incoming_gain - nominal.incoming_gain * blend).abs() < 1e-10);
            assert_eq!(frame.outgoing_gain + frame.incoming_gain, 1.0);
        }
        plan.outgoing_end = 10.5;
        assert!(gate_gains(plan, 10.25, plan.frame(10.25), None).is_none());
    }

    /// Native reconstruction latency must be measured after the first seek and included in the second target.
    #[test]
    fn delayed_seek_is_remeasured_before_sync() {
        for delay in [0.024, 0.080, 0.180, 0.300, 0.420] {
            let mut phase = PhaseAlignment::default();
            let beginning = Instant::now();
            assert_eq!(
                phase.observe(5.0, 4.7, 1.0, true, beginning),
                Some(PhaseAdjustment::Seek(5.0))
            );
            // The old clock can remain visible after command acknowledgement; never use it as proof.
            assert_eq!(
                phase.observe(5.01, 5.01, 1.0, true, beginning + Duration::from_millis(10)),
                None
            );
            let settled = beginning + Duration::from_secs(1);
            if delay <= 0.030 {
                assert_eq!(
                    phase.observe(6.0, 6.0 - delay, 1.0, true, settled),
                    Some(PhaseAdjustment::Follow {
                        speed: 1.02,
                        synchronized: true
                    })
                );
                continue;
            }
            assert_eq!(
                phase.observe(6.0, 6.0 - delay, 1.0, true, settled),
                Some(PhaseAdjustment::Seek(6.0 + delay))
            );
            let after_seek = settled + Duration::from_secs(1);
            assert_eq!(
                phase.observe(7.0, 7.0, 1.0, true, after_seek),
                Some(PhaseAdjustment::Follow {
                    speed: 1.0,
                    synchronized: true,
                })
            );
        }
    }

    /// A cold seek can take much longer than a warm retry; its allowance must not become a permanent offset.
    #[test]
    fn cold_and_warm_seek_latency_converge_before_unmuting() {
        let mut phase = PhaseAlignment::default();
        let now = Instant::now();
        phase.start(now);
        assert_eq!(phase.observe(1.0, 1.0, 1.0, true, now), None);
        let first = now + Duration::from_millis(150);
        assert_eq!(
            phase.observe(1.0, 0.6, 1.0, true, first),
            Some(PhaseAdjustment::Seek(1.0))
        );
        let second = first + Duration::from_millis(320);
        let Some(PhaseAdjustment::Seek(target)) = phase.observe(1.32, 1.0, 1.0, true, second)
        else {
            panic!("cold seek latency must be measured");
        };
        assert!((target - 1.64).abs() < 1e-10);
        let third = second + Duration::from_millis(150);
        let Some(PhaseAdjustment::Seek(target)) = phase.observe(1.47, 1.77, 1.0, true, third)
        else {
            panic!("cold seek allowance must be corrected while muted");
        };
        assert!((target - 1.49).abs() < 1e-10);
        let settled = third + Duration::from_millis(150);
        assert_eq!(
            phase.observe(1.62, 1.62, 1.0, true, settled),
            Some(PhaseAdjustment::Follow {
                speed: 1.0,
                synchronized: true
            })
        );
        assert!(matches!(
            phase.observe(2.0, 1.9, 1.0, true, settled),
            Some(PhaseAdjustment::Follow {
                synchronized: false,
                ..
            })
        ));
    }

    /// Losing a clock after measured lag cannot release the incoming stream during corrective reconstruction.
    #[test]
    fn missing_clock_cannot_bypass_observed_misalignment() {
        let mut phase = PhaseAlignment::default();
        let now = Instant::now();
        phase.start(now);
        assert!(!phase.permits_unverified_fallback(now));
        let settled = now + Duration::from_millis(150);
        assert!(phase.permits_unverified_fallback(settled));
        assert!(matches!(
            phase.observe(1.0, 0.7, 1.0, true, settled),
            Some(PhaseAdjustment::Seek(_))
        ));
        assert!(!phase.permits_unverified_fallback(settled));
        assert!(!phase.permits_unverified_fallback(settled + Duration::from_secs(1)));
    }

    /// Changed device latency and clock drift converge without further seeks after acquisition.
    #[test]
    fn continued_feedback_centers_residual_error_without_repeated_seeks() {
        for initial_error in [-0.080, -0.024, 0.024, 0.080] {
            let mut phase = PhaseAlignment::default();
            let mut outgoing = 10.0;
            let mut incoming = outgoing + initial_error;
            let beginning = Instant::now();
            for step in 0..800 {
                let now = beginning + Duration::from_millis(step * 10);
                let adjustment = phase.observe(outgoing, incoming, 1.0, false, now);
                let Some(PhaseAdjustment::Follow { speed, .. }) = adjustment else {
                    panic!("audible clock correction must not seek");
                };
                assert!((0.98..=1.02).contains(&speed));
                outgoing += 0.01;
                // The extra clock drift ensures the controller cannot rely on one corrected offset forever.
                incoming += (speed + 0.001) * 0.01;
            }
            assert!((incoming - outgoing).abs() < 0.004);
        }
    }

    /// Bad clocks never authorize SYNC or seeks; corrective rates retain the existing twenty-percent bound.
    #[test]
    fn invalid_clocks_and_extreme_errors_are_bounded() {
        let mut phase = PhaseAlignment::default();
        let now = Instant::now();
        for invalid in [f64::NAN, f64::INFINITY, -1.0] {
            assert_eq!(phase.observe(invalid, 1.0, 1.0, true, now), None);
            assert_eq!(phase.observe(1.0, invalid, 1.0, true, now), None);
            assert_eq!(phase.observe(1.0, 1.0, invalid, true, now), None);
        }
        for nominal in [0.8, 1.0, 1.2] {
            for actual in [0.0, 100.0] {
                let Some(PhaseAdjustment::Follow {
                    speed,
                    synchronized,
                }) = phase.observe(50.0, actual, nominal, false, now)
                else {
                    panic!("valid clock must retain bounded feedback");
                };
                assert!(is_supported_mix_speed(speed));
                assert!(!synchronized);
            }
        }
    }
}
