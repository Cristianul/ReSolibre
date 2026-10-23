//! Portable logarithmic band reduction and time-based display motion.

mod consumer;
pub use consumer::{LiveConsumer, PcmChunk};

use super::{BIN_COUNT, FFT_SIZE, SpectrumFrame};

pub const BAND_COUNT: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveLevels {
    pub epoch: u64,
    pub levels: [f32; BAND_COUNT],
}

/// Integrates bin energy into fixed 40 Hz–16 kHz logarithmic bands, then maps -70..-10 dB to height.
/// Fractional-bin overlap avoids empty low-frequency bands; frequencies above Nyquist stay empty.
pub fn reduce(frame: &SpectrumFrame) -> [f32; BAND_COUNT] {
    if frame.sample_rate_hz == 0 {
        return [0.0; BAND_COUNT];
    }
    let mut power = [0.0_f64; BAND_COUNT];
    let edges: [f64; BAND_COUNT + 1] =
        std::array::from_fn(|band| 40.0 * 400_f64.powf(band as f64 / BAND_COUNT as f64));
    let spacing = f64::from(frame.sample_rate_hz) / FFT_SIZE as f64;
    let mut first_band = 0;
    for bin in 1..BIN_COUNT {
        let low = (bin as f64 - 0.5) * spacing;
        let high = ((bin as f64 + 0.5) * spacing).min(f64::from(frame.sample_rate_hz) / 2.0);
        while first_band < BAND_COUNT && edges[first_band + 1] <= low {
            first_band += 1;
        }
        if first_band == BAND_COUNT {
            break;
        }
        let energy = 10_f64.powf(f64::from(frame.power_db[bin]) / 10.0);
        if !energy.is_finite() {
            continue;
        }
        for (band, sum) in power.iter_mut().enumerate().skip(first_band) {
            let bottom = edges[band];
            if bottom >= high {
                break;
            }
            let top = edges[band + 1];
            let overlap = (high.min(top) - low.max(bottom)).max(0.0);
            *sum += energy * overlap / spacing;
        }
    }
    power.map(|energy| {
        if energy <= 1e-7 {
            return 0.0;
        }
        ((10.0 * energy.log10() + 70.0) / 60.0).clamp(0.0, 1.0) as f32
    })
}

/// Advances toward real measured targets with a 35 ms attack and 180 ms release, without overshoot.
pub fn smooth(current: &mut [f32; BAND_COUNT], target: &[f32; BAND_COUNT], seconds: f32) {
    if !seconds.is_finite() || seconds <= 0.0 {
        return;
    }
    for (value, target) in current.iter_mut().zip(target) {
        if !value.is_finite() {
            *value = 0.0;
        }
        *value = value.clamp(0.0, 1.0);
        let target = if target.is_finite() {
            target.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let tau = if target > *value { 0.035 } else { 0.180 };
        let blend = 1.0 - (-seconds.min(1.0) / tau).exp();
        *value += (target - *value) * blend;
        if (*value - target).abs() < 0.0001 {
            *value = target;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Band positions reflect measured frequency, while silence and unavailable high frequencies remain dark.
    #[test]
    fn bands_follow_frequency_without_inventing_high_rate_content() {
        let mut frame = SpectrumFrame {
            sample_rate_hz: 8000,
            start_frame: 0,
            end_frame: 2048,
            valid_frames: 2048,
            power_db: [-160.0; BIN_COUNT],
        };
        assert_eq!(reduce(&frame), [0.0; BAND_COUNT]);
        frame.power_db[64] = -20.0; // 250 Hz
        let low = reduce(&frame);
        frame.power_db[64] = -160.0;
        frame.power_db[384] = -20.0; // 1,500 Hz
        let high = reduce(&frame);
        let low_peak = low.iter().position(|level| *level > 0.5).unwrap();
        let high_peak = high.iter().position(|level| *level > 0.5).unwrap();
        assert!(high_peak > low_peak + 4);
        assert!(high[19..].iter().all(|level| *level == 0.0));
        frame.power_db.fill(f32::NAN);
        assert_eq!(reduce(&frame), [0.0; BAND_COUNT]);
        frame.sample_rate_hz = 0;
        assert_eq!(reduce(&frame), [0.0; BAND_COUNT]);
    }

    /// Equal elapsed time gives equal motion at different refresh rates; falling energy decays more slowly.
    #[test]
    fn motion_is_time_based_and_bounded() {
        let mut fast = [0.0; BAND_COUNT];
        let mut slow = fast;
        for _ in 0..60 {
            smooth(&mut fast, &[1.0; BAND_COUNT], 1.0 / 60.0);
        }
        for _ in 0..30 {
            smooth(&mut slow, &[1.0; BAND_COUNT], 1.0 / 30.0);
        }
        assert!((fast[0] - slow[0]).abs() < 0.0001);
        let mut attack = [0.0; BAND_COUNT];
        let mut release = [1.0; BAND_COUNT];
        smooth(&mut attack, &[1.0; BAND_COUNT], 0.035);
        smooth(&mut release, &[0.0; BAND_COUNT], 0.035);
        assert!(attack[0] > 0.6 && release[0] > 0.8);
        smooth(&mut release, &[f32::NAN; BAND_COUNT], f32::INFINITY);
        assert!(
            release
                .iter()
                .all(|level| level.is_finite() && (0.0..=1.0).contains(level))
        );
    }
}
