//! Native EQ translation. Only validated numbers and adapter-owned filter names enter libavfilter.

use crate::{LocalPlayer, PlaybackError};
use resolibre_core::equalizer::{BAND_Q, Equalizer, FREQUENCIES, active_bands};
use std::ffi::CString;

#[derive(Default)]
pub(crate) struct EqRuntime {
    rate: u32,
    configured: Option<Equalizer>,
    bands: [f64; 10],
    preamp: f64,
}

impl LocalPlayer {
    /// Configures a fixed graph before initialization, avoiding uninitialized live-filter commands.
    pub(crate) fn configure_equalizer(&mut self, settings: Equalizer) -> Result<(), PlaybackError> {
        let graph = CString::new(format!(
            "{},{}",
            filter_graph(settings),
            crate::playback_edges::filter_graph(None)
        ))
        .map_err(|_| PlaybackError::InvalidEvent)?;
        self.set_option(c"af", &graph)?;
        let (bands, preamp) = settings.effective();
        self.equalizer = EqRuntime {
            rate: 0,
            configured: Some(settings),
            bands,
            preamp,
        };
        Ok(())
    }

    /// Makes the latest curve part of the graph configuration before seek can recreate it.
    /// libavfilter runtime commands alone do not survive graph reset/reconstruction.
    pub fn seek_with_equalizer(
        &mut self,
        position: resolibre_core::SeekPosition,
        settings: Equalizer,
    ) -> Result<(), PlaybackError> {
        self.commit_equalizer(settings)?;
        self.seek(position)
    }

    /// Commits live EQ intent before a seek or direction change rebuilds the filter graph.
    pub(crate) fn commit_equalizer(&mut self, settings: Equalizer) -> Result<(), PlaybackError> {
        if self.equalizer.configured != Some(settings) {
            self.playback_command(
                &["af", "add", &filter_graph(settings)],
                "equalizer seek setup",
            )?;
            self.equalizer.configured = Some(settings);
        }
        // Seek reconstructs the configured graph even if a live ramp was mid-step.
        let (bands, preamp) = settings.effective();
        self.equalizer.bands = bands;
        self.equalizer.preamp = preamp;
        Ok(())
    }

    /// Invalidates observations when mpv rebuilds audio filters, including source-format changes.
    pub(crate) fn equalizer_reconfigured(&mut self) {
        self.edge_filter.dirty = true;
        self.equalizer.rate = 0;
        if let Some(settings) = self.equalizer.configured {
            let (bands, preamp) = settings.effective();
            self.equalizer.bands = bands;
            self.equalizer.preamp = preamp;
        }
    }

    /// Applies the curve before resume; false means source/output negotiation is still pending.
    pub fn prepare_equalizer(&mut self, settings: Equalizer) -> Result<bool, PlaybackError> {
        if self.is_seeking()? {
            return Ok(false);
        }
        let formats = self.audio_formats()?;
        let (Some(source), Some(_output)) = (formats.source, formats.output) else {
            return Ok(false);
        };
        self.equalizer.rate = source.sample_rate_hz;
        self.advance_equalizer(settings, 36.0)?;
        self.restore_edge_envelope()?;
        Ok(true)
    }

    /// Moves live gains toward the latest intent without rebuilding the active filter graph.
    /// Native commands are bounded by eleven parameters; no filesystem or user expressions.
    pub fn ramp_equalizer(
        &mut self,
        settings: Equalizer,
        seconds: f64,
    ) -> Result<(), PlaybackError> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(PlaybackError::InvalidEvent);
        }
        self.advance_equalizer(settings, seconds.min(0.05) * 60.0)
    }

    /// Exposes the negotiated source rate for truthful unavailable-band presentation.
    pub fn equalizer_rate(&self) -> u32 {
        self.equalizer.rate
    }

    /// Updates an initialized graph. Out-of-range centers bypass natively and receive no commands.
    fn advance_equalizer(&mut self, settings: Equalizer, step: f64) -> Result<(), PlaybackError> {
        if self.equalizer.rate == 0 {
            let Some(source) = self.audio_formats()?.source else {
                return Err(PlaybackError::InvalidFormat);
            };
            self.equalizer.rate = source.sample_rate_hz;
        }
        let active = active_bands(self.equalizer.rate);
        let (target_bands, target_preamp) = settings.effective();
        // Attenuate first and raise preamp last. This is ordering, not automatic headroom.
        let next_preamp = approach(self.equalizer.preamp, target_preamp, step);
        if next_preamp < self.equalizer.preamp {
            self.set_eq_preamp(next_preamp)?;
        }
        for index in 0..10 {
            let next = approach(self.equalizer.bands[index], target_bands[index], step);
            if active[index] && next != self.equalizer.bands[index] {
                self.playback_command(
                    &[
                        "af-command",
                        "resolibre_eq",
                        "gain",
                        &next.to_string(),
                        &format!("equalizer@b{index}"),
                    ],
                    "equalizer band",
                )?;
            }
            self.equalizer.bands[index] = next;
        }
        if next_preamp > self.equalizer.preamp {
            self.set_eq_preamp(next_preamp)?;
        }
        Ok(())
    }

    /// Converts explicit preamp dB to a linear double-precision filter gain.
    fn set_eq_preamp(&mut self, db: f64) -> Result<(), PlaybackError> {
        let gain = 10.0_f64.powf(db / 20.0);
        self.playback_command(
            &[
                "af-command",
                "resolibre_eq",
                "volume",
                &gain.to_string(),
                "volume@pre",
            ],
            "equalizer preamp",
        )?;
        self.equalizer.preamp = db;
        Ok(())
    }
}

/// Creates one fixed double-precision graph. FFmpeg bypasses invalid centers at initial setup;
/// the adapter never sends gain commands to bands at/above source Nyquist.
fn filter_graph(settings: Equalizer) -> String {
    let (bands, preamp) = settings.effective();
    let gain = 10.0_f64.powf(preamp / 20.0);
    let mut filters = vec![format!("volume@pre=volume={gain}:precision=double")];
    for (index, frequency) in FREQUENCIES.iter().enumerate() {
        filters.push(format!(
            "equalizer@b{index}=f={frequency}:t=q:w={BAND_Q}:g={}:r=f64",
            bands[index]
        ));
    }
    format!("@resolibre_eq:lavfi=[{}]", filters.join(","))
}

/// Bounds a parameter step and lands exactly on the target to allow unity-graph removal.
fn approach(current: f64, target: f64, step: f64) -> f64 {
    current + (target - current).clamp(-step, step)
}
