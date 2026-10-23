//! Minimal Linux local-file adapter. Native ownership stays on the calling thread.

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("The initial local-file prototype currently supports Linux x86_64 only.");

mod analysis_cache;
mod controls;
mod edge_analysis;
mod equalizer;
mod ffi;
mod formats;
mod live_spectrum;
pub mod media_formats;
#[cfg(test)]
mod media_tests;
pub mod metadata;
#[cfg(test)]
mod mix_capture_tests;
mod native_logging;
mod original_audio;
mod playback_edges;
mod rhythm_analysis;
pub mod session;
mod track_analysis;

#[cfg(test)]
mod output_adaptation_tests;

#[cfg(test)]
mod edges_capture_tests;
#[cfg(test)]
mod eq_capture_tests;
#[cfg(test)]
mod pcm_capture_tests;
#[cfg(test)]
mod scan_capture_tests;

#[cfg(test)]
mod output_failure_tests;

#[cfg(all(test, target_env = "gnu"))]
mod responsiveness_tests;

use resolibre_core::{PlaybackEvent, SeekPosition};
use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::ptr::NonNull;

// Linux x86_64 open flag; other platforms must supply their own native adapter.
const OPEN_NONBLOCK: i32 = 0o4000;

/// Explicitly choose real device output or silent decoding for automated tests.
#[derive(Clone, Copy, Debug)]
pub enum AudioOutput {
    Device,
    Silent,
    /// Test-only sink that accepts stereo Float32; absent from production builds.
    #[cfg(test)]
    RestrictedSilent,
    /// Private test worker output; the parent owns the working directory and limits.
    #[cfg(test)]
    CapturePcm {
        resample: bool,
    },
    /// Private Pulse endpoint and virtual sink; absent from production builds.
    #[cfg(test)]
    PrivatePulse,
    /// Fault injection: real private output with its optional audio timestamp unavailable.
    #[cfg(test)]
    PrivatePulseWithoutClock,
    /// Injects one invalid incoming audio-clock observation after resume, before the first audible MIX frame.
    #[cfg(test)]
    PrivatePulseInvalidClock,
    /// Private native PipeWire output; explicit backend forbids fallback to a real desktop device.
    #[cfg(test)]
    PrivatePipeWire,
}

/// Privacy-safe errors retain native codes without native messages or paths.
#[derive(Debug)]
pub enum PlaybackError {
    Input(std::io::ErrorKind),
    NotRegularFile,
    UnsupportedApi,
    Allocation,
    Native { operation: &'static str, code: i32 },
    UnexpectedEnd,
    EventOverflow,
    InvalidEvent,
    InvalidFormat,
}

impl std::fmt::Display for PlaybackError {
    /// Formats only allowlisted categories and numeric native codes.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(kind) => write!(formatter, "local input unavailable ({kind:?})"),
            Self::NotRegularFile => formatter.write_str("input must be a regular local file"),
            Self::UnsupportedApi => formatter.write_str("libmpv client API 2.x is required"),
            Self::Allocation => formatter.write_str("cannot allocate playback engine"),
            Self::Native { operation, code } => {
                write!(formatter, "{operation} failed (libmpv code {code})")
            }
            Self::UnexpectedEnd => formatter.write_str("playback ended unexpectedly"),
            Self::EventOverflow => formatter.write_str("playback event queue overflowed"),
            Self::InvalidEvent => formatter.write_str("invalid playback event"),
            Self::InvalidFormat => formatter.write_str("invalid audio format property"),
        }
    }
}

impl std::error::Error for PlaybackError {}

/// One engine and one opened file. NonNull intentionally keeps this !Send/!Sync.
/// A new owner is required for another track, so stale cross-track events cannot
/// override newer intent. Drop stops the engine before the file is closed.
pub struct LocalPlayer {
    handle: NonNull<ffi::Handle>,
    _native_lease: native_logging::NativeLease,
    input: File,
    encoded_info: Option<resolibre_core::track_info::StreamInfo>,
    loaded: bool,
    scanning: bool,
    mix_tempo_filter: bool,
    #[cfg(test)]
    suppress_audio_clock: bool,
    #[cfg(test)]
    invalidate_initial_audio_clock: bool,
    equalizer: equalizer::EqRuntime,
    edge_filter: playback_edges::EdgeFilter,
    live_spectrum: Option<live_spectrum::LiveSpectrum>,
    edges: Option<(
        resolibre_core::playback_edges::PlaybackRegion,
        resolibre_core::playback_edges::FadeMode,
    )>,
}

impl LocalPlayer {
    /// Opens one regular file read-only, configures libmpv and requests playback.
    /// Paths are never interpreted as URLs. Native work may block; use a worker
    /// when composing a UI. Success is not evidence of audible output.
    pub fn open(path: &Path, output: AudioOutput) -> Result<Self, PlaybackError> {
        Self::open_configured(path, output, None)
    }

    /// Opens with session pause/gain set before any audio can reach the device.
    pub fn open_for_session(
        path: &Path,
        output: AudioOutput,
        paused: bool,
        volume: resolibre_core::session::Volume,
    ) -> Result<Self, PlaybackError> {
        Self::open_with_equalizer(
            path,
            output,
            paused,
            volume,
            resolibre_core::equalizer::Equalizer::default(),
        )
    }

    /// Installs the session curve before decoding; no initial unprocessed audio can escape.
    pub fn open_with_equalizer(
        path: &Path,
        output: AudioOutput,
        paused: bool,
        volume: resolibre_core::session::Volume,
        equalizer: resolibre_core::equalizer::Equalizer,
    ) -> Result<Self, PlaybackError> {
        Self::open_configured(path, output, Some((paused, volume, equalizer)))
    }

    /// Owns descriptor/engine initialization for both CLI and interactive sessions.
    fn open_configured(
        path: &Path,
        output: AudioOutput,
        settings: Option<(
            bool,
            resolibre_core::session::Volume,
            resolibre_core::equalizer::Equalizer,
        )>,
    ) -> Result<Self, PlaybackError> {
        // Linux O_NONBLOCK prevents a FIFO from blocking before metadata validation.
        // It has no effect on regular files. Opening first also avoids path-replace
        // races: mpv receives this descriptor, not a second lookup of the user path.
        let input = OpenOptions::new()
            .read(true)
            .custom_flags(OPEN_NONBLOCK)
            .open(path)
            .map_err(|error| PlaybackError::Input(error.kind()))?;
        let metadata = input
            .metadata()
            .map_err(|error| PlaybackError::Input(error.kind()))?;

        if !metadata.is_file() {
            return Err(PlaybackError::NotRegularFile);
        }

        Self::open_owned(input, output, settings, None)
    }

    /// Owns a validated descriptor before native initialization; an analysis sink can only be an owned pipe.
    fn open_owned(
        input: File,
        output: AudioOutput,
        settings: Option<(
            bool,
            resolibre_core::session::Volume,
            resolibre_core::equalizer::Equalizer,
        )>,
        analysis_pipe: Option<i32>,
    ) -> Result<Self, PlaybackError> {
        // SAFETY: this global query has no pointer arguments or ownership effects.
        let version = unsafe { ffi::mpv_client_api_version() };
        if version >> 16 != 2 {
            return Err(PlaybackError::UnsupportedApi);
        }

        let native_lease = native_logging::NativeLease::acquire()?;

        // SAFETY: creation needs no inputs; a successful allocation has one owner.
        let handle = NonNull::new(unsafe { ffi::mpv_create() }).ok_or(PlaybackError::Allocation)?;
        use std::os::unix::fs::FileExt;
        let mut header = [0_u8; 42];
        let encoded_info = input
            .read_exact_at(&mut header, 0)
            .ok()
            .and_then(|()| resolibre_core::track_info::flac_stream_info(&header));
        let mut player = Self {
            encoded_info,
            handle,
            _native_lease: native_lease,
            input,
            loaded: false,
            scanning: false,
            mix_tempo_filter: false,
            #[cfg(test)]
            suppress_audio_clock: matches!(output, AudioOutput::PrivatePulseWithoutClock),
            #[cfg(test)]
            invalidate_initial_audio_clock: matches!(output, AudioOutput::PrivatePulseInvalidClock)
                && settings.is_some_and(|(_, volume, _)| volume.percent() == 0.0),
            equalizer: equalizer::EqRuntime::default(),
            edge_filter: playback_edges::EdgeFilter::default(),
            live_spectrum: None,
            edges: None,
        };

        // Construct the owner before fallible configuration so Drop always cleans up.
        player.configure(output)?;
        if let Some(descriptor) = analysis_pipe {
            player.configure_analysis_pipe(descriptor)?;
        }
        if let Some((paused, volume, equalizer)) = settings {
            player.configure_equalizer(equalizer)?;
            let pause = if paused { c"yes" } else { c"no" };
            player.set_option(c"pause", pause)?;
            let gain = CString::new(controls::software_volume(volume.amplitude()).to_string())
                .map_err(|_| PlaybackError::InvalidEvent)?;
            player.set_option(c"volume", &gain)?;
            player.set_option(c"volume-max", c"100")?;
        }

        // SAFETY: this is a live, configured, not-yet-initialized handle.
        let result = unsafe { ffi::mpv_initialize(player.handle.as_ptr()) };
        check_native("initialize", result)?;
        if analysis_pipe.is_some() {
            player.add_analysis_hook()?;
        }

        let descriptor_path = format!("/proc/self/fd/{}", player.input.as_raw_fd());
        player.command(&["loadfile", &descriptor_path, "replace"], "load")?;
        Ok(player)
    }

    /// Applies a closed configuration; unsupported safety options fail startup.
    fn configure(&mut self, output: AudioOutput) -> Result<(), PlaybackError> {
        let options = [
            (c"config", c"no"),
            (c"terminal", c"no"),
            (c"msg-level", c"all=no"),
            (c"load-scripts", c"no"),
            (c"ytdl", c"no"),
            (c"osc", c"no"),
            (c"input-default-bindings", c"no"),
            (c"input-vo-keyboard", c"no"),
            (c"vid", c"no"),
            (c"sid", c"no"),
            (c"autoload-files", c"no"),
            (c"access-references", c"no"),
            (c"save-position-on-quit", c"no"),
            (c"resume-playback", c"no"),
            (c"demuxer", c"lavf"),
            (c"demuxer-lavf-probesize", c"1048576"),
            (c"demuxer-lavf-analyzeduration", c"2"),
            (c"demuxer-max-bytes", c"8MiB"),
            (c"demuxer-max-back-bytes", c"0"),
            (c"cache", c"no"),
            (c"cache-on-disk", c"no"),
            (c"audio-pitch-correction", c"yes"),
            (c"audio-reversal-buffer", c"4MiB"),
            (c"demuxer-backward-playback-step", c"1"),
            (c"idle", c"yes"),
        ];

        for (name, value) in options {
            self.set_option(name, value)?;
        }

        self.set_option(c"demuxer-lavf-o", &media_formats::demuxer_options())?;

        match output {
            AudioOutput::Device => {}
            #[cfg(test)]
            AudioOutput::PrivatePulse
            | AudioOutput::PrivatePulseWithoutClock
            | AudioOutput::PrivatePulseInvalidClock => {
                output_failure_tests::configure(self)?;
            }
            #[cfg(test)]
            AudioOutput::PrivatePipeWire => {
                let directory =
                    std::env::current_dir().map_err(|error| PlaybackError::Input(error.kind()))?;
                if std::env::var_os("PIPEWIRE_RUNTIME_DIR").as_deref()
                    != Some(directory.as_os_str())
                {
                    return Err(PlaybackError::InvalidEvent);
                }
                self.set_option(c"ao", c"pipewire")?;
                self.set_option(c"pipewire-remote", c"resolibre-test")?;
                let client_name = CString::new(format!("resolibre-test-{}", std::process::id()))
                    .map_err(|_| PlaybackError::InvalidEvent)?;
                self.set_option(c"audio-client-name", &client_name)?;
            }
            AudioOutput::Silent => self.set_option(c"ao", c"null")?,
            #[cfg(test)]
            AudioOutput::RestrictedSilent => {
                self.set_option(c"ao", c"null")?;
                // Restrict the sink, not the global audio-format/channels policy.
                // libmpv must adapt decoded PCM during output negotiation.
                self.set_option(c"ao-null-channel-layouts", c"stereo")?;
                self.set_option(c"ao-null-format", c"float")?;
            }
            #[cfg(test)]
            AudioOutput::CapturePcm { resample } => {
                self.set_option(c"ao", c"pcm")?;
                // The capture harness measures a fixed PCM16 sink, independent
                // of the processing graph's negotiated working precision.
                self.set_option(c"audio-format", c"s16")?;
                self.set_option(c"ao-pcm-file", c"capture.wav")?;
                self.set_option(c"ao-pcm-waveheader", c"yes")?;
                self.set_option(c"ao-pcm-append", c"no")?;
                // The untimed writer could finish before format inspection.
                self.set_option(c"pause", c"yes")?;
                if resample {
                    self.set_option(c"audio-samplerate", c"48000")?;
                }
            }
        }

        Ok(())
    }

    /// Copies a static option into the live engine without exposing native strings.
    fn set_option(&mut self, name: &'static CStr, value: &CStr) -> Result<(), PlaybackError> {
        // SAFETY: handle and both NUL-terminated strings remain alive for this call.
        let result = unsafe {
            ffi::mpv_set_option_string(self.handle.as_ptr(), name.as_ptr(), value.as_ptr())
        };
        // Only static adapter-owned option names enter diagnostics, never values.
        let operation = name.to_str().unwrap_or("configure");
        check_native(operation, result)
    }

    /// Pauses or resumes decoding. Acceptance does not measure output cessation.
    pub fn set_paused(&mut self, paused: bool) -> Result<(), PlaybackError> {
        // A paused scan may rest at EOF. Ordinary Play restores normal queue completion.
        if !paused && !self.scanning {
            self.playback_command(&["set", "keep-open", "no"], "scan boundary")?;
        }
        let value = if paused { "yes" } else { "no" };
        self.command(&["set", "pause", value], "pause")
    }

    /// Requests an exact absolute seek; completion still requires output evidence.
    /// A control racing with teardown yields to the pending terminal event.
    pub fn seek(&mut self, position: SeekPosition) -> Result<(), PlaybackError> {
        self.commit_edge_envelope()?;
        let target_seconds = self.edges.map_or(position.seconds(), |(region, _)| {
            position.seconds().clamp(region.start, region.end)
        });
        if let Some(spectrum) = self.live_spectrum.as_mut() {
            spectrum
                .seek(target_seconds)
                .map_err(|_| PlaybackError::InvalidEvent)?;
        }
        let seconds = target_seconds.to_string();
        self.playback_command(&["seek", &seconds, "absolute+exact"], "seek")?;
        // Commands can arrive before AUDIO_RECONFIG. Re-negotiate before sending live DSP updates.
        self.equalizer_reconfigured();
        Ok(())
    }

    /// Reads the engine's current position; this is not a measurement at the DAC.
    /// Before loading or after EOF the property may be unavailable (native error).
    pub fn position(&mut self) -> Result<SeekPosition, PlaybackError> {
        let mut seconds = 0.0_f64;

        // SAFETY: MPV_FORMAT_DOUBLE (5) writes one f64 into aligned live storage.
        // The property name is static and the handle remains alive throughout.
        let result = unsafe {
            ffi::mpv_get_property(
                self.handle.as_ptr(),
                c"time-pos".as_ptr(),
                ffi::FORMAT_DOUBLE,
                (&mut seconds as *mut f64).cast(),
            )
        };
        check_native("position", result)?;

        SeekPosition::from_seconds(seconds).ok_or(PlaybackError::InvalidEvent)
    }

    /// Submits only adapter-owned commands; mpv copies arguments during the call.
    fn command(
        &mut self,
        arguments: &[&str],
        operation: &'static str,
    ) -> Result<(), PlaybackError> {
        let mut strings = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let string = CString::new(*argument).map_err(|_| PlaybackError::InvalidEvent)?;
            strings.push(string);
        }

        let mut pointers: Vec<_> = strings.iter().map(|value| value.as_ptr()).collect();
        pointers.push(std::ptr::null());

        // SAFETY: vector is null-terminated; every string and the handle outlive
        // this synchronous call. No argument points into mutable native memory.
        let result = unsafe { ffi::mpv_command(self.handle.as_ptr(), pointers.as_ptr()) };
        check_native(operation, result)
    }

    /// Waits at most 100 ms for one event. Unknown events are ignored for ABI
    /// compatibility; callers must keep draining events even during playback.
    pub fn poll_event(&mut self) -> Result<Option<PlaybackEvent>, PlaybackError> {
        self.poll_event_timeout(0.1)
    }

    /// Drains an event without blocking the session command worker.
    pub fn try_event(&mut self) -> Result<Option<PlaybackEvent>, PlaybackError> {
        self.poll_event_timeout(0.0)
    }

    /// Borrows one event with a caller-selected, adapter-owned wait bound.
    fn poll_event_timeout(&mut self, timeout: f64) -> Result<Option<PlaybackEvent>, PlaybackError> {
        // SAFETY: unique mutable access prevents another wait or destruction while
        // the borrowed event and its payload are inspected below. Nothing escapes.
        let event = unsafe { ffi::mpv_wait_event(self.handle.as_ptr(), timeout).as_ref() }
            .ok_or(PlaybackError::InvalidEvent)?;

        match event.id {
            ffi::EVENT_AUDIO_RECONFIG => {
                self.equalizer_reconfigured();
                Ok(None)
            }
            ffi::EVENT_FILE_LOADED => {
                self.loaded = true;
                Ok(Some(PlaybackEvent::Loaded))
            }
            ffi::EVENT_END_FILE => {
                self.loaded = false;
                // SAFETY: this event id defines the EndFile layout in API 2.x.
                let end = unsafe { event.data.cast::<ffi::EndFile>().as_ref() }
                    .ok_or(PlaybackError::InvalidEvent)?;

                if end.reason == ffi::END_REASON_ERROR {
                    return Err(PlaybackError::Native {
                        operation: "playback",
                        code: end.error,
                    });
                }
                if end.reason != ffi::END_REASON_EOF {
                    return Err(PlaybackError::UnexpectedEnd);
                }

                Ok(Some(PlaybackEvent::Ended))
            }
            ffi::EVENT_QUEUE_OVERFLOW => Err(PlaybackError::EventOverflow),
            ffi::EVENT_SHUTDOWN => Err(PlaybackError::UnexpectedEnd),
            _ => Ok(None),
        }
    }
}

impl Drop for LocalPlayer {
    /// Stops native work before Rust closes the retained read-only input file.
    fn drop(&mut self) {
        // SAFETY: this is the sole owner; no event borrow survives a method call.
        // libmpv permits destruction of a client whose initialization failed.
        unsafe { ffi::mpv_terminate_destroy(self.handle.as_ptr()) };
    }
}

/// Preserves the native failure code while discarding potentially private text.
fn check_native(operation: &'static str, code: i32) -> Result<(), PlaybackError> {
    if code < 0 {
        return Err(PlaybackError::Native { operation, code });
    }

    Ok(())
}
