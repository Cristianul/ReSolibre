//! Bounded spectral onset evidence for tempo candidates, independent of precise beat timestamps.
//!
//! A complete FFT arrives about half a window after its effective centre, plus the resampler's
//! filter delay. Consumers may use these envelopes for periodicity and tempo support, but must
//! retain their original-source onset detector when reporting measured beat positions. The long bass
//! observation uses a separate slower clock and is held between measurements, never mixed into the
//! short-window envelopes or treated as a newly measured beat.

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

const FFT_SIZE: usize = 512;
const BIN_COUNT: usize = FFT_SIZE / 2 + 1;
const MAX_RATE: u32 = 11_025;
const TRANSFORMS_PER_SECOND: f64 = 100.0;
const BAND_COUNT: usize = 3;
const LONG_FFT_SIZE: usize = 2_048;
const LONG_BIN_COUNT: usize = LONG_FFT_SIZE / 2 + 1;
const LONG_TRANSFORMS_PER_SECOND: f64 = 50.0;
// The long window resolves low notes but also exposes weak modulation sidebands of held notes.
const LONG_PEAK_CHANGE_FLOOR: f64 = 0.03;
pub(super) const ENVELOPE_BANDS: usize = 4;

#[derive(Clone)]
struct LowPass {
    input: [f64; 3],
    feedback: [f64; 2],
}

impl LowPass {
    /// Builds one stable low-pass section; the two supplied qualities form a fourth-order filter.
    fn new(rate: f64, cutoff: f64, quality: f64) -> Self {
        let angle = std::f64::consts::TAU * cutoff / rate;
        let cosine = angle.cos();
        let alpha = angle.sin() / (2.0 * quality);
        let divisor = 1.0 + alpha;
        let first = (1.0 - cosine) / (2.0 * divisor);

        Self {
            input: [first, 2.0 * first, first],
            feedback: [-2.0 * cosine / divisor, (1.0 - alpha) / divisor],
        }
    }

    /// Advances a channel-owned transposed filter state without mixing it with other channels.
    fn process(&self, sample: f64, state: &mut [f64; 2]) -> f64 {
        let output = self.input[0] * sample + state[0];
        state[0] = self.input[1] * sample - self.feedback[0] * output + state[1];
        state[1] = self.input[2] * sample - self.feedback[1] * output;
        output
    }
}

#[derive(Clone)]
pub(super) struct SpectralOnsets {
    channels: usize,
    decimation: usize,
    until_sample: usize,
    filters: [LowPass; 2],
    filter_state: Vec<[[f64; 2]; 2]>,
    history: Vec<f64>,
    write_position: usize,
    filled: usize,
    since_transform: usize,
    hop: usize,
    window: [f64; FFT_SIZE],
    fft: Arc<dyn Fft<f64>>,
    transform: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    power: [f64; BIN_COUNT],
    normalization: f64,
    bands: [u8; BIN_COUNT],
    band_bins: [usize; BAND_COUNT],
    previous: [f64; BIN_COUNT],
    primed: bool,
    pending: [f64; ENVELOPE_BANDS],
    long_bass: LongBassOnsets,
    since_long_transform: usize,
    long_hop: usize,
    latest_long_bass: f64,
    advanced_since_bin: bool,
}

impl SpectralOnsets {
    /// Allocates fixed transform/history storage for dimensions already validated by the owner.
    /// Supports the owner's full rate/channel range; storage never depends on recording length.
    pub(super) fn new(rate: u32, channels: usize) -> Self {
        debug_assert!(rate > 0 && rate <= 1_536_000);
        debug_assert!(channels > 0 && channels <= crate::spectrum::MAX_CHANNELS);

        let decimation = rate.div_ceil(MAX_RATE).max(1) as usize;
        let effective_rate = f64::from(rate) / decimation as f64;
        let cutoff = effective_rate * 0.4;
        let filters = [
            LowPass::new(f64::from(rate), cutoff, 0.541_196_100_146_197),
            LowPass::new(f64::from(rate), cutoff, 1.306_562_964_876_377),
        ];
        let hop = (effective_rate / TRANSFORMS_PER_SECOND).round().max(1.0) as usize;

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        let window = std::array::from_fn(|index| {
            let angle = std::f64::consts::TAU * index as f64 / FFT_SIZE as f64;
            0.5 - 0.5 * angle.cos()
        });

        let mut bands = [BAND_COUNT as u8; BIN_COUNT];
        let mut band_bins = [0; BAND_COUNT];
        for (bin, band) in bands.iter_mut().enumerate().skip(1) {
            let frequency = bin as f64 * effective_rate / FFT_SIZE as f64;
            if frequency < 30.0 {
                continue;
            }
            let selected = if frequency < 200.0 {
                0
            } else if frequency < 2_000.0 {
                1
            } else {
                2
            };
            *band = selected as u8;
            band_bins[selected] += 1;
        }

        Self {
            channels,
            decimation,
            until_sample: decimation,
            filters,
            filter_state: vec![[[0.0; 2]; 2]; channels],
            history: vec![0.0; LONG_FFT_SIZE * channels],
            write_position: 0,
            filled: 0,
            since_transform: 0,
            hop,
            window,
            fft,
            transform: vec![Complex::default(); FFT_SIZE],
            scratch,
            power: [0.0; BIN_COUNT],
            normalization: 1.0 / (FFT_SIZE * FFT_SIZE * channels) as f64,
            bands,
            band_bins,
            previous: [0.0; BIN_COUNT],
            primed: false,
            pending: [0.0; ENVELOPE_BANDS],
            long_bass: LongBassOnsets::new(effective_rate, channels),
            since_long_transform: 0,
            latest_long_bass: 0.0,
            advanced_since_bin: false,
            long_hop: (effective_rate / LONG_TRANSFORMS_PER_SECOND)
                .round()
                .max(1.0) as usize,
        }
    }

    /// Accepts one already-validated interleaved frame; no allocation or source mutation occurs.
    /// Anti-alias filtering precedes integer decimation, and each channel keeps independent state.
    pub(super) fn push_frame(&mut self, frame: &[f32]) {
        debug_assert_eq!(frame.len(), self.channels);
        self.advanced_since_bin = true;

        self.until_sample -= 1;
        let retain = self.until_sample == 0;
        for (channel, sample) in frame.iter().enumerate() {
            let mut sample = f64::from(*sample);
            if self.decimation > 1 {
                for (filter, state) in self.filters.iter().zip(&mut self.filter_state[channel]) {
                    sample = filter.process(sample, state);
                }
            }
            if retain {
                let destination = self.write_position * self.channels + channel;
                self.history[destination] = sample;
            }
        }
        if !retain {
            return;
        }

        self.until_sample = self.decimation;
        self.write_position = (self.write_position + 1) % LONG_FFT_SIZE;
        self.filled = (self.filled + 1).min(LONG_FFT_SIZE);
        self.since_transform += 1;
        if self.filled >= FFT_SIZE && self.since_transform >= self.hop {
            self.analyze();
            self.since_transform = 0;
        }

        self.since_long_transform += 1;
        if self.filled == LONG_FFT_SIZE && self.since_long_transform >= self.long_hop {
            self.latest_long_bass =
                self.long_bass
                    .analyze(&self.history, self.write_position, self.channels);
            self.since_long_transform = 0;
        }
    }

    /// Drains short bass/middle/upper and independent long-window bass evidence into the next bin.
    /// The slower bass observation is held only while source frames advance; empty repeated reads are zero.
    pub(super) fn take_bin(&mut self) -> [f64; ENVELOPE_BANDS] {
        let mut result = std::mem::take(&mut self.pending);
        if std::mem::take(&mut self.advanced_since_bin) {
            // Sparse 50 Hz impulses would create a four-bin comb on the owner's 200 Hz clock.
            // Hold the measured novelty until its next observation instead of inserting false zeros.
            result[BAND_COUNT] = self.latest_long_bass;
        }
        result
    }

    /// Averages channel powers before measuring positive spectral changes, preserving inverse polarity.
    fn analyze(&mut self) {
        self.power.fill(0.0);
        for channel in 0..self.channels {
            for (index, output) in self.transform.iter_mut().enumerate() {
                let frame =
                    (self.write_position + LONG_FFT_SIZE - FFT_SIZE + index) % LONG_FFT_SIZE;
                let sample = self.history[frame * self.channels + channel];
                *output = Complex::new(sample * self.window[index], 0.0);
            }
            self.fft
                .process_with_scratch(&mut self.transform, &mut self.scratch);
            for (power, transformed) in self.power.iter_mut().zip(&self.transform) {
                *power += transformed.norm_sqr() * self.normalization;
            }
        }

        let changes = spectral_changes(
            &self.power,
            &mut self.previous,
            self.primed,
            0.0,
            0..BIN_COUNT,
        );
        self.primed = true;
        let mut band_changes = [0.0; BAND_COUNT];
        for (bin, change) in changes.into_iter().enumerate().skip(1) {
            let band = usize::from(self.bands[bin]);
            if band < BAND_COUNT {
                band_changes[band] += change;
            }
        }

        for (band, change) in band_changes.into_iter().enumerate() {
            if self.band_bins[band] > 0 {
                self.pending[band] += change / self.band_bins[band] as f64;
            }
        }
    }
}

/// Resolves slow low-note changes without replacing the short window's faster onset observations.
#[derive(Clone)]
struct LongBassOnsets {
    window: [f64; LONG_FFT_SIZE],
    fft: Arc<dyn Fft<f64>>,
    transform: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    power: [f64; LONG_BIN_COUNT],
    previous: [f64; LONG_BIN_COUNT],
    normalization: f64,
    first_bin: usize,
    end_bin: usize,
    primed: bool,
}

impl LongBassOnsets {
    /// Allocates one fixed long transform; the outer analyzer owns and shares the sample history.
    fn new(rate: f64, channels: usize) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(LONG_FFT_SIZE);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        let first_bin = (30.0 * LONG_FFT_SIZE as f64 / rate).ceil() as usize;
        let end_bin = (200.0 * LONG_FFT_SIZE as f64 / rate).ceil() as usize;
        Self {
            window: std::array::from_fn(|index| {
                let angle = std::f64::consts::TAU * index as f64 / LONG_FFT_SIZE as f64;
                0.5 - 0.5 * angle.cos()
            }),
            fft,
            transform: vec![Complex::default(); LONG_FFT_SIZE],
            scratch,
            power: [0.0; LONG_BIN_COUNT],
            previous: [0.0; LONG_BIN_COUNT],
            normalization: 1.0 / (LONG_FFT_SIZE * LONG_FFT_SIZE * channels) as f64,
            first_bin: first_bin.clamp(1, LONG_BIN_COUNT),
            end_bin: end_bin.min(LONG_BIN_COUNT),
            primed: false,
        }
    }

    /// Measures independent bass novelty; its longer window is never a source of beat timestamps.
    fn analyze(&mut self, history: &[f64], write_position: usize, channels: usize) -> f64 {
        self.power.fill(0.0);
        for channel in 0..channels {
            for (index, output) in self.transform.iter_mut().enumerate() {
                let frame = (write_position + index) % LONG_FFT_SIZE;
                let sample = history[frame * channels + channel];
                *output = Complex::new(sample * self.window[index], 0.0);
            }
            self.fft
                .process_with_scratch(&mut self.transform, &mut self.scratch);
            for (power, transformed) in self.power.iter_mut().zip(&self.transform) {
                *power += transformed.norm_sqr() * self.normalization;
            }
        }
        let changes = spectral_changes(
            &self.power,
            &mut self.previous,
            self.primed,
            LONG_PEAK_CHANGE_FLOOR,
            self.first_bin..self.end_bin,
        );
        self.primed = true;
        if self.end_bin <= self.first_bin {
            return 0.0;
        }
        changes[self.first_bin..self.end_bin].iter().sum::<f64>()
            / (self.end_bin - self.first_bin) as f64
    }
}

/// Rejects deep sidelobes, adjacent-bin pitch movement and tiny changes of sustained tones.
/// Both resolutions share these guards so improved low-note resolution does not bypass them.
fn spectral_changes<const BINS: usize>(
    power: &[f64; BINS],
    previous: &mut [f64; BINS],
    primed: bool,
    peak_change_floor: f64,
    peak_bins: std::ops::Range<usize>,
) -> [f64; BINS] {
    let strongest = power.iter().copied().fold(0.0_f64, f64::max);
    let floor = (strongest * 1e-6).max(1e-16);
    let levels: [f64; BINS] = std::array::from_fn(|bin| {
        let retained = (power[bin] - floor).max(0.0);
        (100.0 * retained.sqrt()).ln_1p()
    });
    // The leakage floor remains spectrum-wide, but a louder stationary instrument in another
    // band must not raise the minimum change required from this band's audible notes.
    let largest_level = levels[peak_bins].iter().copied().fold(0.0_f64, f64::max);
    let minimum_change = largest_level * peak_change_floor;
    let mut changes = [0.0; BINS];
    if primed {
        for bin in 1..BINS {
            let neighbor_max = previous[bin - 1]
                .max(previous[bin])
                .max(previous[(bin + 1).min(BINS - 1)]);
            changes[bin] =
                (levels[bin] - neighbor_max - levels[bin] * 0.01 - minimum_change).max(0.0);
        }
    }
    *previous = levels;
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds fixed input in arbitrary block sizes while draining the owner's 200 Hz source clock.
    fn envelopes(
        rate: u32,
        channels: usize,
        samples: &[f32],
        block_frames: usize,
    ) -> [Vec<f64>; ENVELOPE_BANDS] {
        let mut analyzer = SpectralOnsets::new(rate, channels);
        let mut output: [Vec<f64>; ENVELOPE_BANDS] = std::array::from_fn(|_| Vec::new());
        let mut position = 0_u64;
        let mut previous_bin = 0;
        for block in samples.chunks(block_frames * channels) {
            for frame in block.chunks_exact(channels) {
                let bin = position * 200 / u64::from(rate);
                if bin != previous_bin {
                    for (band, value) in output.iter_mut().zip(analyzer.take_bin()) {
                        band.push(value);
                    }
                    previous_bin = bin;
                }
                analyzer.push_frame(frame);
                position += 1;
            }
        }
        for (band, value) in output.iter_mut().zip(analyzer.take_bin()) {
            band.push(value);
        }
        output
    }

    /// Generates layered attacks with source-clock timing independent of transform/hop boundaries.
    fn percussion(rate: u32, seconds: u32) -> Vec<f32> {
        (0..rate * seconds)
            .map(|frame| {
                let time = f64::from(frame) / f64::from(rate);
                let beat = time % 0.5;
                let envelope = (-beat * 45.0).exp();
                let bass = (std::f64::consts::TAU * 80.0 * time).sin();
                let upper = (std::f64::consts::TAU * 2_800.0 * time).sin();
                ((bass + upper * 0.3) * envelope * 0.4) as f32
            })
            .collect()
    }

    /// Opposite-polarity stereo must carry the same onset evidence as one original channel.
    #[test]
    fn inverse_channels_and_block_boundaries_preserve_onsets() {
        for rate in [8_000, 44_100, 48_000] {
            let mono = percussion(rate, 2);
            let stereo: Vec<f32> = mono.iter().flat_map(|sample| [*sample, -*sample]).collect();
            let expected = envelopes(rate, 1, &mono, 997);
            let actual = envelopes(rate, 2, &stereo, 193);
            let fragmented = envelopes(rate, 2, &stereo, 1);
            assert_eq!(actual, fragmented);
            for (single, opposite) in expected.iter().flatten().zip(actual.iter().flatten()) {
                assert!((single - opposite).abs() < 1e-12);
            }
            assert!(actual[0].iter().sum::<f64>() > 0.1);
            assert!(actual[2].iter().sum::<f64>() > 0.01);
        }
    }

    /// Real repeated attacks retain their source tempo after filtering, decimation and leakage rejection.
    #[test]
    fn percussive_tempo_survives_spectral_processing() {
        for rate in [8_000, 48_000] {
            let source = percussion(rate, 16);
            let measured = envelopes(rate, 1, &source, 997);
            let candidates: Vec<_> = measured
                .iter()
                .filter_map(|band| super::super::estimate::estimate(band, 0.0))
                .collect();
            assert!(
                candidates
                    .iter()
                    .any(|candidate| (candidate.bpm - 120.0).abs() < 0.5),
                "spectral evidence lost the source pulse at {rate} Hz: {candidates:?}"
            );
        }
    }

    /// Silence, steady tones and seeded noise do not become supported periodic tempo evidence.
    #[test]
    fn stationary_inputs_do_not_create_a_beat_candidate() {
        for rate in [8_000, 44_100] {
            let mut state = 17_u64;
            let noise: Vec<f32> = (0..rate * 16)
                .map(|_| {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1);
                    ((state >> 32) as u32 as f64 / f64::from(u32::MAX) - 0.5) as f32 * 0.2
                })
                .collect();
            let mut signals = vec![vec![0.0; noise.len()], noise];
            for frequency in [55.0, 80.0, 110.0, 440.0, 1_000.0, 2_800.0] {
                signals.push(
                    (0..rate * 16)
                        .map(|frame| {
                            let phase = std::f64::consts::TAU * frequency * f64::from(frame)
                                / f64::from(rate);
                            phase.sin() as f32 * 0.2
                        })
                        .collect(),
                );
            }
            for (signal_index, signal) in signals.into_iter().enumerate() {
                let measured = envelopes(rate, 1, &signal, 4096);
                for (band_index, band) in measured.into_iter().enumerate() {
                    let mut periodicity = super::super::evidence::PeriodicityAnalyzer::new();
                    let evidence = periodicity.collect(0.0, &[&band], None);
                    let candidates = evidence.candidates();
                    assert!(
                        candidates.is_empty(),
                        "stationary signal {signal_index}, band {band_index}: {candidates:?}, maximum {}",
                        evidence.maximum()
                    );
                }
            }
        }
    }

    /// Higher bass resolution must not turn small pitch motion of a held note into new tempo evidence.
    #[test]
    fn sustained_vibrato_does_not_create_long_bass_candidates() {
        for rate in [8_000, 44_100] {
            for center in [55.0, 110.0, 180.0, 440.0] {
                for vibrato_rate in [3.1, 5.7] {
                    let mut phase = 0.0;
                    let signal: Vec<f32> = (0..rate * 16)
                        .map(|frame| {
                            let time = f64::from(frame) / f64::from(rate);
                            let modulation = (std::f64::consts::TAU * vibrato_rate * time).sin();
                            let frequency = center * 2.0_f64.powf(modulation * 20.0 / 1_200.0);
                            phase += std::f64::consts::TAU * frequency / f64::from(rate);
                            phase.sin() as f32 * 0.2
                        })
                        .collect();
                    let measured = envelopes(rate, 1, &signal, 997);
                    let mut periodicity = super::super::evidence::PeriodicityAnalyzer::new();
                    let evidence = periodicity.collect(0.0, &[&measured[BAND_COUNT]], None);
                    let candidates = evidence.candidates();
                    assert!(
                        candidates.is_empty(),
                        "held vibrato {center}/{vibrato_rate} at {rate} Hz: {candidates:?}, maximum {}",
                        evidence.maximum()
                    );
                }
            }
        }
    }

    /// Equal-volume low notes retain periodic changes even without a percussive amplitude envelope.
    #[test]
    fn long_bass_window_retains_legato_note_changes() {
        let rate = 8_000;
        let mut phase = 0.0;
        let signal: Vec<f32> = (0..rate * 24)
            .map(|frame| {
                let time = f64::from(frame) / f64::from(rate);
                let frequency = if time % 1.0 < 0.5 { 55.0 } else { 73.416 };
                phase += std::f64::consts::TAU * frequency / f64::from(rate);
                phase.sin() as f32 * 0.2
            })
            .collect();
        let measured = envelopes(rate, 1, &signal, 997);
        let mut periodicity = super::super::evidence::PeriodicityAnalyzer::new();
        let evidence = periodicity.collect(0.0, &[&measured[BAND_COUNT]], None);
        let candidates = evidence.candidates();
        assert!(
            candidates
                .iter()
                .any(|bpm| { (*bpm - 120.0).abs() < 0.5 || (*bpm - 60.0).abs() < 0.5 }),
            "periodic low-note changes must retain their family: {candidates:?}"
        );
    }

    /// An unrelated stationary upper tone must not erase quieter, independently repeated bass notes.
    #[test]
    fn louder_stationary_upper_tone_does_not_mask_low_note_changes() {
        let rate = 8_000;
        let mut phase = 0.0;
        let bass: Vec<f32> = (0..rate * 24)
            .map(|frame| {
                let time = f64::from(frame) / f64::from(rate);
                let frequency = if time % 1.0 < 0.5 { 55.0 } else { 73.416 };
                phase += std::f64::consts::TAU * frequency / f64::from(rate);
                phase.sin() as f32 * 0.001
            })
            .collect();
        let mixture: Vec<f32> = bass
            .iter()
            .enumerate()
            .map(|(frame, sample)| {
                let phase = std::f64::consts::TAU * 1_000.0 * frame as f64 / f64::from(rate);
                *sample + phase.sin() as f32 * 0.4
            })
            .collect();
        for source in [&bass, &mixture] {
            let measured = envelopes(rate, 1, source, 997);
            let mut periodicity = super::super::evidence::PeriodicityAnalyzer::new();
            let evidence = periodicity.collect(0.0, &[&measured[BAND_COUNT]], None);
            let candidates = evidence.candidates();
            assert!(
                candidates
                    .iter()
                    .any(|bpm| (*bpm - 60.0).abs() < 0.5 || (*bpm - 120.0).abs() < 0.5),
                "stationary upper tone masked a supported bass pulse: {candidates:?}"
            );
        }
    }

    /// High-rate input and extreme finite samples retain fixed storage and finite drained output.
    #[test]
    fn storage_and_pending_values_remain_bounded() {
        let mut analyzer = SpectralOnsets::new(1_536_000, 32);
        let capacity = analyzer.history.capacity();
        let frame = [f32::MAX; 32];
        for _ in 0..400_000 {
            analyzer.push_frame(&frame);
        }
        assert_eq!(analyzer.history.len(), LONG_FFT_SIZE * 32);
        assert!(analyzer.long_bass.primed);
        assert_eq!(analyzer.history.capacity(), capacity);
        assert!(analyzer.take_bin().into_iter().all(f64::is_finite));
        assert_eq!(analyzer.take_bin(), [0.0; ENVELOPE_BANDS]);

        let mut startup = SpectralOnsets::new(8_000, 1);
        for _ in 0..FFT_SIZE - 1 {
            startup.push_frame(&[1.0]);
        }
        assert_eq!(startup.take_bin(), [0.0; ENVELOPE_BANDS]);
        startup.push_frame(&[1.0]);
        assert_eq!(startup.take_bin(), [0.0; ENVELOPE_BANDS]);
    }
}
