//! Optional source-PCM observation. Native loading/clock access stays here; core owns analysis policy.

#[cfg(unix)]
mod native;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::LiveSpectrum;

use crate::LocalPlayer;
use resolibre_core::spectrum::live::LiveLevels;
#[cfg(unix)]
use resolibre_core::spectrum::{
    AnalysisError,
    live::{LiveConsumer, PcmChunk},
};

#[cfg(unix)]
pub(crate) struct LiveSpectrum {
    tap: native::Tap,
    format: Option<(u32, u32)>,
    consumer: LiveConsumer,
}

#[cfg(unix)]
impl LiveSpectrum {
    /// Allocates bounded transport and shared analysis state before callbacks can run.
    fn new(_handle: std::ptr::NonNull<crate::ffi::Handle>) -> Result<Self, &'static str> {
        Ok(Self {
            tap: native::Tap::new()?,
            format: None,
            consumer: LiveConsumer::default(),
        })
    }

    /// Lists only the owned PCM tap when a negotiated source format changes.
    fn labels(&self) -> Vec<String> {
        if self.format.is_some() {
            vec!["resolibre_live".into()]
        } else {
            Vec::new()
        }
    }

    /// Keeps Linux's sealed PCM tap while sharing the platform-independent installation path.
    fn graph(&mut self, _rate: u32, channels: u32) -> Result<String, &'static str> {
        Ok(self.tap.graph(channels))
    }

    /// Rejects existing graph identities before a native seek reconstructs audio filters.
    pub(crate) fn seek(&mut self, seconds: f64) -> Result<(), AnalysisError> {
        self.consumer.seek(self.tap.barrier(), seconds)
    }

    /// Copies bounded native input into the portable consumer using the observed playback clock.
    fn poll(
        &mut self,
        clock: Option<f64>,
        paused: bool,
        scanning: bool,
    ) -> Result<LiveLevels, &'static str> {
        let clock = clock.unwrap_or(self.consumer.offset());
        for _ in 0..8 {
            let Some(block) = self.tap.read()? else {
                break;
            };
            let length = block.frames as usize * block.channels as usize;
            let chunk = PcmChunk {
                instance: block.instance,
                rate: block.rate,
                channels: block.channels as usize,
                first_frame: block.first_frame,
                samples: &block.samples[..length],
            };
            self.consumer
                .accept(chunk, clock)
                .map_err(|_| "Invalid live spectrum samples")?;
        }
        Ok(self.consumer.present(clock, paused, scanning))
    }
}

impl LocalPlayer {
    /// Observes optional analysis; the tap remains owned by the player even during errors or unwinding.
    pub fn live_spectrum(&mut self) -> Result<Option<LiveLevels>, &'static str> {
        if !self.prepare_live_spectrum()? {
            return Ok(None);
        }
        let timing = self
            .timing()
            .map_err(|_| "Live spectrum clock unavailable")?;
        // Untimed sinks/startup may lack an audio clock; use the engine timeline until it is available.
        let clock = self.audio_position().ok().flatten().or(timing.position);
        let spectrum = self
            .live_spectrum
            .as_mut()
            .ok_or("Live spectrum unavailable")?;
        let result = spectrum.poll(clock, timing.paused, self.scanning).map(Some);
        #[cfg(windows)]
        if result.is_err() {
            // Stop native measurement work after optional-analysis failure, retaining the audio path.
            for label in spectrum.labels() {
                let _ = self.command(
                    &["af", "remove", &format!("@{label}")],
                    "live spectrum retirement",
                );
            }
        }
        result
    }

    /// Installs the branch after channel negotiation, retaining its context until native destruction.
    fn prepare_live_spectrum(&mut self) -> Result<bool, &'static str> {
        let source = self
            .audio_formats()
            .map_err(|_| "Live spectrum format unavailable")?
            .source;
        let Some(source) = source else {
            return Ok(false);
        };
        if !(1..=32).contains(&source.channels) {
            return Err("Live spectrum channel count unsupported");
        }
        if self.live_spectrum.is_none() {
            self.live_spectrum = Some(LiveSpectrum::new(self.handle)?);
        }
        let spectrum = self
            .live_spectrum
            .as_mut()
            .ok_or("Live spectrum unavailable")?;
        let format = (source.sample_rate_hz, source.channels);
        if spectrum.format != Some(format) {
            let labels = spectrum.labels();
            let graph = spectrum.graph(source.sample_rate_hz, source.channels)?;
            for label in labels {
                self.command(
                    &["af", "remove", &format!("@{label}")],
                    "live spectrum replacement",
                )
                .map_err(|_| "Live spectrum filter unavailable")?;
            }
            if !graph.is_empty() {
                self.command(&["af", "pre", &graph], "live spectrum setup")
                    .map_err(|_| "Live spectrum filter unavailable")?;
            }
            if let Some(spectrum) = self.live_spectrum.as_mut() {
                spectrum.format = Some(format);
            }
        }
        Ok(true)
    }
}
