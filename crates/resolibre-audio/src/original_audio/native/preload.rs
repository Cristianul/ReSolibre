//! A native preload hook selects and validates the stream before any PCM can fill the output pipe.

use crate::{LocalPlayer, PlaybackError, check_native, ffi};
use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

impl LocalPlayer {
    /// Registers one hook on this handle before loadfile; no hook IDs cross engine lifetimes.
    pub(crate) fn add_analysis_hook(&mut self) -> Result<(), PlaybackError> {
        // SAFETY: initialized handle and static hook name remain live; userdata identifies this owned hook.
        let code =
            unsafe { ffi::mpv_hook_add(self.handle.as_ptr(), 1, c"on_preloaded".as_ptr(), 0) };
        check_native("analysis preload", code)
    }

    /// Reads bounded adapter-generated numeric properties while the engine is held before decoding.
    fn analysis_integer(&mut self, name: &CStr) -> Result<i64, PlaybackError> {
        let mut value = 0_i64;
        // SAFETY: INT64 writes aligned live storage; the borrowed name and handle outlive the call.
        let code = unsafe {
            ffi::mpv_get_property(
                self.handle.as_ptr(),
                name.as_ptr(),
                ffi::FORMAT_INT64,
                (&mut value as *mut i64).cast(),
            )
        };
        check_native("analysis stream", code)?;
        Ok(value)
    }

    /// Finds the exact FFmpeg stream in at most 128 tracks and selects its mpv audio ID explicitly.
    fn select_analysis_stream(&mut self, stream: u32) -> Result<(u32, usize, u32), PlaybackError> {
        let stream = if stream == u32::MAX {
            self.default_analysis_stream()?
        } else {
            stream
        };
        let tracks = self.analysis_integer(c"track-list/count")?;
        if !(1..=128).contains(&tracks) {
            return Err(PlaybackError::InvalidFormat);
        }
        for index in 0..tracks {
            let prefix = format!("track-list/{index}");
            let property = CString::new(format!("{prefix}/ff-index"))
                .map_err(|_| PlaybackError::InvalidFormat)?;
            if self.analysis_integer(&property).ok() != Some(i64::from(stream)) {
                continue;
            }
            let rate = self.analysis_integer(
                &CString::new(format!("{prefix}/demux-samplerate"))
                    .map_err(|_| PlaybackError::InvalidFormat)?,
            )?;
            let channels = self.analysis_integer(
                &CString::new(format!("{prefix}/demux-channel-count"))
                    .map_err(|_| PlaybackError::InvalidFormat)?,
            )?;
            if !(8000..=384000).contains(&rate) || !(1..=32).contains(&channels) {
                return Err(PlaybackError::InvalidFormat);
            }
            let id = self.analysis_integer(
                &CString::new(format!("{prefix}/id")).map_err(|_| PlaybackError::InvalidFormat)?,
            )?;
            if id <= 0 {
                return Err(PlaybackError::InvalidFormat);
            }
            self.command(&["set", "aid", &id.to_string()], "analysis audio selection")?;
            return Ok((rate as u32, channels as usize, stream));
        }
        Err(PlaybackError::InvalidFormat)
    }

    /// Prepares the default audio track before mpv has current-tracks; playback later revalidates its exact stream ID.
    fn default_analysis_stream(&mut self) -> Result<u32, PlaybackError> {
        let count = self.analysis_integer(c"track-list/count")?;
        if !(1..=128).contains(&count) {
            return Err(PlaybackError::InvalidFormat);
        }
        let mut first = None;
        for index in 0..count {
            let prefix = format!("track-list/{index}");
            let rate = CString::new(format!("{prefix}/demux-samplerate"))
                .map_err(|_| PlaybackError::InvalidFormat)?;
            if !self.analysis_integer(&rate).is_ok_and(|rate| rate > 0) {
                continue;
            }
            let stream = CString::new(format!("{prefix}/ff-index"))
                .map_err(|_| PlaybackError::InvalidFormat)?;
            let stream = u32::try_from(self.analysis_integer(&stream)?)
                .map_err(|_| PlaybackError::InvalidFormat)?;
            first.get_or_insert(stream);
            let property = CString::new(format!("{prefix}/default"))
                .map_err(|_| PlaybackError::InvalidFormat)?;
            let mut selected = 0_i32;
            // SAFETY: FLAG writes one aligned integer; the generated property and exclusive handle stay live.
            let code = unsafe {
                ffi::mpv_get_property(
                    self.handle.as_ptr(),
                    property.as_ptr(),
                    ffi::FORMAT_FLAG,
                    (&mut selected as *mut i32).cast(),
                )
            };
            check_native("analysis default stream", code)?;
            if selected != 0 {
                return Ok(stream);
            }
        }
        first.ok_or(PlaybackError::InvalidFormat)
    }
}

/// Waits for the owned hook, selects the stream, then continues that hook exactly once, even on validation failure.
pub(super) fn prepare(
    player: &mut LocalPlayer,
    stream: u32,
    cancel: &AtomicBool,
    started: Instant,
    deadline: Duration,
) -> Result<(u32, usize, u32), PlaybackError> {
    loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() >= deadline {
            return Err(PlaybackError::UnexpectedEnd);
        }
        // SAFETY: exclusive owner borrows the event only until the next wait; copied hook ID has no native borrow.
        let event = unsafe { ffi::mpv_wait_event(player.handle.as_ptr(), 0.02).as_ref() }
            .ok_or(PlaybackError::InvalidEvent)?;
        if event.id == ffi::EVENT_HOOK && event.reply_userdata == 1 {
            // SAFETY: HOOK defines this payload layout; copy its ID before further client calls.
            let id = unsafe { event.data.cast::<ffi::EventHook>().as_ref() }
                .ok_or(PlaybackError::InvalidEvent)?
                .id;
            let result = player.select_analysis_stream(stream);
            // SAFETY: this ID came from this handle's sole hook and is continued exactly once.
            let code = unsafe { ffi::mpv_hook_continue(player.handle.as_ptr(), id) };
            check_native("analysis preload continuation", code)?;
            return result;
        }
        if matches!(
            event.id,
            ffi::EVENT_END_FILE | ffi::EVENT_SHUTDOWN | ffi::EVENT_QUEUE_OVERFLOW
        ) {
            return Err(PlaybackError::UnexpectedEnd);
        }
    }
}
