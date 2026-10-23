//! Typed live properties used by the interactive session.

use crate::{LocalPlayer, PlaybackError, check_native, ffi};
use std::ffi::CStr;

/// Keep stereo channels isolated and avoid Rubber Band 4's phase reset near unity.
/// Its 1e-7 ratio threshold needs a persistent 1 ppm guard (less than 0.002 cents).
/// Store the guard in the graph so seeks/reconfiguration cannot silently remove it.
/// At 48 kHz R3 needs 2048 input padding samples and 2048 output delay samples removed.
const MIX_TEMPO_FILTER: &str = concat!(
    "@resolibre_mix_input:lavfi=[aresample=48000,adelay=2048S:all=1],",
    "@resolibre_mix_tempo:rubberband=engine=finer:window=standard:",
    "pitch=consistency:channels=apart:pitch-scale=1.000001,",
    "@resolibre_mix_output:lavfi=[atrim=start_sample=2048,asetpts=PTS-2048/SR/TB]"
);

/// A non-atomic engine observation; unavailable properties remain explicit.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub seekable: bool,
}

impl LocalPlayer {
    /// Applies a bounded mix tempo through the existing pitch-corrected engine, never the device clock.
    /// Retains the filter at unity until this stream ends or scanning reconstructs its audio graph.
    pub(crate) fn set_mix_speed(&mut self, speed: f64) -> Result<(), PlaybackError> {
        if !resolibre_core::transition::is_supported_mix_speed(speed) {
            return Err(PlaybackError::InvalidEvent);
        }

        if speed != 1.0 {
            self.prepare_mix_tempo()?;
        }

        self.playback_command(&["set", "speed", &speed.to_string()], "mix speed")?;

        // Removing a live filter discards buffered audio and shortens one beat at handover.
        // Keep the same graph while returning to unity so its queued samples can drain.
        Ok(())
    }

    /// Prepares a persistent stretcher with the native real-time startup padding removed.
    pub(crate) fn prepare_mix_tempo(&mut self) -> Result<(), PlaybackError> {
        if !self.mix_tempo_filter {
            self.playback_command(&["af", "add", MIX_TEMPO_FILTER], "mix tempo setup")?;
            self.mix_tempo_filter = true;
        }

        Ok(())
    }

    /// Reads the optional audio clock including estimated output delay; invalid samples mean unavailable, not playback failure.
    pub(crate) fn audio_position(&mut self) -> Result<Option<f64>, PlaybackError> {
        #[cfg(test)]
        if self.suppress_audio_clock {
            return Ok(None);
        }
        let observation = self.optional_seconds(c"audio-pts");
        #[cfg(test)]
        let observation = if self.invalidate_initial_audio_clock && !self.flag(c"pause")? {
            self.invalidate_initial_audio_clock = false;
            println!("mix-clock-invalid");
            Err(PlaybackError::InvalidEvent)
        } else {
            observation
        };

        match observation {
            // The optional output clock may not yet describe audible samples after resume.
            // Preserve strict validation for transport position/duration; only this hint may disappear.
            Err(PlaybackError::InvalidEvent) => Ok(None),
            result => result,
        }
    }

    /// Reads safe timing properties; loading/draining may temporarily hide them.
    pub fn timing(&mut self) -> Result<Timing, PlaybackError> {
        Ok(Timing {
            position: self.optional_seconds(c"time-pos")?,
            duration: self.optional_seconds(c"duration")?,
            paused: self.flag(c"pause")?,
            seekable: self.flag(c"seekable")?,
        })
    }

    /// Reports native seek reconstruction, during which filter commands are not yet usable.
    pub(crate) fn is_seeking(&mut self) -> Result<bool, PlaybackError> {
        self.flag(c"seeking")
    }

    /// Temporarily scans at 3x with native pitch correction, holding at either file edge.
    /// Direction changes seek internally; commit the latest EQ graph before reconstruction.
    pub fn set_scan(
        &mut self,
        direction: Option<resolibre_core::session::ScanDirection>,
        settings: resolibre_core::equalizer::Equalizer,
        paused: bool,
    ) -> Result<(), PlaybackError> {
        let position = self.timing()?.position;
        self.set_paused(true)?;
        if self.mix_tempo_filter {
            self.playback_command(
                &[
                    "af",
                    "remove",
                    "@resolibre_mix_tempo,@resolibre_mix_input,@resolibre_mix_output",
                ],
                "mix tempo restore",
            )?;
            self.mix_tempo_filter = false;
        }
        self.scanning = direction.is_some();
        self.commit_equalizer(settings)?;
        self.playback_command(&["set", "keep-open", "yes"], "scan boundary")?;
        let backward = direction == Some(resolibre_core::session::ScanDirection::Backward);
        self.set_edge_direction(backward)?;
        let native_direction = if backward { "backward" } else { "forward" };
        self.playback_command(
            &["set", "speed", if direction.is_some() { "3" } else { "1" }],
            "scan speed",
        )?;
        self.playback_command(
            &["set", "play-direction", native_direction],
            "scan direction",
        )?;
        // Flush queued audio at the old speed; also preserve the position across direction changes.
        if let Some(position) = position.and_then(resolibre_core::SeekPosition::from_seconds) {
            self.seek(position)?;
        }
        self.set_paused(direction.is_none() && paused)
    }

    /// Sets software gain only; the session ramps validated amplitude over time.
    pub fn set_amplitude(&mut self, amplitude: f64) -> Result<(), PlaybackError> {
        if !amplitude.is_finite() || !(0.0..=1.0).contains(&amplitude) {
            return Err(PlaybackError::InvalidEvent);
        }
        self.command(
            &["set", "volume", &software_volume(amplitude).to_string()],
            "volume",
        )
    }

    /// Lets an already-loaded engine's terminal event resolve controls racing with teardown.
    /// Other command failures remain errors; this never manufactures successful EOF.
    pub(crate) fn playback_command(
        &mut self,
        arguments: &[&str],
        operation: &'static str,
    ) -> Result<(), PlaybackError> {
        let result = self.command(arguments, operation);
        if matches!(
            result,
            Err(PlaybackError::Native {
                code: ffi::ERROR_COMMAND,
                ..
            })
        ) && self.loaded
            && self.flag(c"playback-abort")?
        {
            // Each engine owns one file. No replacement can load here; the queued
            // terminal event still determines normal EOF versus actual playback failure.
            return Ok(());
        }
        result
    }

    /// Copies a bounded boolean property, treating temporary absence as false.
    fn flag(&mut self, name: &'static CStr) -> Result<bool, PlaybackError> {
        let mut value = 0_i32;
        // SAFETY: FLAG writes an i32 to aligned live storage; the handle is owned.
        let code = unsafe {
            ffi::mpv_get_property(
                self.handle.as_ptr(),
                name.as_ptr(),
                ffi::FORMAT_FLAG,
                (&mut value as *mut i32).cast(),
            )
        };
        if code == ffi::ERROR_PROPERTY_UNAVAILABLE {
            return Ok(false);
        }
        check_native("state", code)?;
        if value != 0 && value != 1 {
            return Err(PlaybackError::InvalidEvent);
        }
        Ok(value == 1)
    }

    /// Rejects non-finite/negative native times; unknown duration is not zero.
    fn optional_seconds(&mut self, name: &'static CStr) -> Result<Option<f64>, PlaybackError> {
        let mut value = 0.0_f64;
        // SAFETY: DOUBLE writes an f64 to aligned live storage; nothing escapes.
        let code = unsafe {
            ffi::mpv_get_property(
                self.handle.as_ptr(),
                name.as_ptr(),
                ffi::FORMAT_DOUBLE,
                (&mut value as *mut f64).cast(),
            )
        };
        if code == ffi::ERROR_PROPERTY_UNAVAILABLE {
            return Ok(None);
        }
        check_native("timing", code)?;
        if !value.is_finite() || value < 0.0 {
            return Err(PlaybackError::InvalidEvent);
        }
        Ok(Some(value))
    }
}

/// Inverts mpv 0.41's cubic percentage curve so core amplitude remains engine-independent.
/// Captured PCM verifies silence, unity and the perceptual midpoint, not just the property.
pub(crate) fn software_volume(amplitude: f64) -> f64 {
    amplitude.cbrt() * 100.0
}
