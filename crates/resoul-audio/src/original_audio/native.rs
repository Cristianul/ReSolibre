//! On-demand libmpv decoding with an owned bounded pipe; no device, playback processing or disk output.

mod preload;

use super::SourceKey;
use crate::{AudioOutput, LocalPlayer, PlaybackError};
use resolibre_core::PlaybackEvent;
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const UNAVAILABLE: &str = "Original audio analysis unavailable";

impl LocalPlayer {
    /// Configures only an adapter-owned pipe; no external destination or user DSP enters original analysis.
    pub(crate) fn configure_analysis_pipe(&mut self, descriptor: i32) -> Result<(), PlaybackError> {
        let destination = CString::new(format!("/proc/self/fd/{descriptor}"))
            .map_err(|_| PlaybackError::InvalidEvent)?;
        self.set_option(c"ao", c"pcm")?;
        self.set_option(c"ao-pcm-file", &destination)?;
        self.set_option(c"ao-pcm-waveheader", c"yes")?;
        self.set_option(c"ao-pcm-append", c"no")?;
        self.set_option(c"audio-format", c"float")?;
        self.set_option(c"audio-channels", c"auto")?;
        self.set_option(c"volume", c"100")?;
        self.set_option(c"replaygain", c"no")?;
        self.set_option(c"pause", c"no")?;
        self.set_option(c"ad-lavc-threads", c"1")?;
        self.set_option(c"demuxer-max-bytes", c"1MiB")?;
        Ok(())
    }
}

/// Selects the retained file's exact stream before decoding and revalidates identity before publishing.
pub(crate) fn decode<T>(
    file: File,
    key: SourceKey,
    cancel: &AtomicBool,
    deadline: Duration,
    consume: impl FnOnce(&mut Reader<'_>, Instant) -> Result<T, ()>,
) -> Result<T, &'static str> {
    decode_inner(file, key, cancel, deadline, true, consume)
}

/// Allows a deliberate bounded excerpt; identity and cancellation checks are identical to a full decode.
pub(crate) fn decode_prefix<T>(
    file: File,
    key: SourceKey,
    cancel: &AtomicBool,
    deadline: Duration,
    consume: impl FnOnce(&mut Reader<'_>, Instant) -> Result<T, ()>,
) -> Result<T, &'static str> {
    decode_inner(file, key, cancel, deadline, false, consume)
}

/// Owns every native/pipe lifetime; prefix consumers may finish before EOF without publishing decoder failures.
fn decode_inner<T>(
    file: File,
    key: SourceKey,
    cancel: &AtomicBool,
    deadline: Duration,
    require_eof: bool,
    consume: impl FnOnce(&mut Reader<'_>, Instant) -> Result<T, ()>,
) -> Result<T, &'static str> {
    let started = Instant::now();
    let (read, write) = pipe().map_err(|_| UNAVAILABLE)?;
    let retained = file.try_clone().map_err(|_| UNAVAILABLE)?;
    let player =
        LocalPlayer::open_owned(retained, AudioOutput::Silent, None, Some(write.as_raw_fd()))
            .map_err(|_| UNAVAILABLE)?;
    // Own the pipe before preload can continue; every error closes the reader before native teardown.
    let mut reader = Reader {
        pipe: Some(read),
        writer: Some(write),
        player: Some(player),
        cancel,
        started,
        deadline,
        ended: false,
        format: (0, 0, 0),
    };
    reader.format = preload::prepare(
        reader.player.as_mut().ok_or(UNAVAILABLE)?,
        key.stream,
        cancel,
        started,
        deadline,
    )
    .map_err(|_| UNAVAILABLE)?;
    let result = consume(&mut reader, started).map_err(|_| UNAVAILABLE)?;
    if (require_eof && !reader.ended)
        || cancel.load(Ordering::Relaxed)
        || SourceKey::read(&file, key.stream)? != key
    {
        return Err(UNAVAILABLE);
    }
    Ok(result)
}

/// Creates close-on-exec descriptors with exactly one Rust owner each; kernel pipe memory is bounded.
fn pipe() -> io::Result<(File, File)> {
    let mut descriptors = [-1; 2];
    // SAFETY: pipe2 writes two integers into live storage; failed calls transfer no descriptors.
    if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful pipe2 returned distinct owned descriptors; neither has another Rust owner.
    Ok(unsafe {
        (
            File::from_raw_fd(descriptors[0]),
            File::from_raw_fd(descriptors[1]),
        )
    })
}

pub(crate) struct Reader<'a> {
    pipe: Option<File>,
    writer: Option<File>,
    player: Option<LocalPlayer>,
    cancel: &'a AtomicBool,
    started: Instant,
    deadline: Duration,
    ended: bool,
    format: (u32, usize, u32),
}

impl Reader<'_> {
    /// Returns the actual selected stream for cache identity after default selection.
    pub(crate) fn selected_stream(&self) -> u32 {
        self.format.2
    }

    /// Returns selected-stream dimensions for comparison against the emitted WAV format; discrepancies fail closed.
    pub(crate) fn expected_format(&self) -> (u32, usize) {
        (self.format.0, self.format.1)
    }
}

impl Read for Reader<'_> {
    /// Drains PCM while polling only nonblocking events; synchronous properties could deadlock behind pipe writes.
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            if self.cancel.load(Ordering::Relaxed) || self.started.elapsed() >= self.deadline {
                return Err(unavailable());
            }
            if let Some(player) = self.player.as_mut()
                && player.try_event().map_err(|_| unavailable())? == Some(PlaybackEvent::Ended)
            {
                self.ended = true;
                self.writer = None;
            }
            let pipe = self.pipe.as_mut().ok_or_else(unavailable)?;
            let mut descriptor = libc::pollfd {
                fd: pipe.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: poll borrows one initialized descriptor entry owned throughout this bounded call.
            let ready = unsafe { libc::poll(&mut descriptor, 1, 20) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if ready > 0 {
                let count = pipe.read(bytes)?;
                if count > 0 {
                    return Ok(count);
                }
                if self.ended {
                    return Ok(0);
                }
            }
            if self.ended {
                if self.player.is_none() {
                    return Ok(0);
                }
                // Drain before teardown so any final stdio flush has room in the empty pipe.
                self.player = None;
            }
        }
    }
}

impl Drop for Reader<'_> {
    /// Close the read end first, unblocking any native writer before synchronous engine teardown.
    fn drop(&mut self) {
        self.pipe = None;
        self.writer = None;
        self.player = None;
    }
}

/// Converts native failures to a fixed private I/O category for the shared PCM consumer.
fn unavailable() -> io::Error {
    io::Error::other(UNAVAILABLE)
}
