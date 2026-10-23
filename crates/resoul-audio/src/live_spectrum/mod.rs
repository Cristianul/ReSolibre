//! Optional source-PCM observation. Native loading/clock access stays here; core owns analysis policy.

mod native;

use crate::LocalPlayer;
use resolibre_core::spectrum::{
    AnalysisError,
    live::{LiveConsumer, LiveLevels, PcmChunk},
};

pub(crate) struct LiveSpectrum {
    tap: native::Tap,
    channels: Option<u32>,
    consumer: LiveConsumer,
}

impl LiveSpectrum {
    /// Allocates bounded transport and shared analysis state before callbacks can run.
    fn new() -> Result<Self, &'static str> {
        Ok(Self {
            tap: native::Tap::new()?,
            channels: None,
            consumer: LiveConsumer::default(),
        })
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
        spectrum.poll(clock, timing.paused, self.scanning).map(Some)
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
            self.live_spectrum = Some(LiveSpectrum::new()?);
        }
        let spectrum = self
            .live_spectrum
            .as_ref()
            .ok_or("Live spectrum unavailable")?;
        if spectrum.channels != Some(source.channels) {
            let graph = spectrum.tap.graph(source.channels);
            self.command(&["af", "pre", &graph], "live spectrum setup")
                .map_err(|_| "Live spectrum filter unavailable")?;
            if let Some(spectrum) = self.live_spectrum.as_mut() {
                spectrum.channels = Some(source.channels);
            }
        }
        Ok(true)
    }
}
