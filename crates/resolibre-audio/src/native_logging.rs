//! Process-wide libmpv logging ownership must outlive every playback/analysis handle.

use crate::{PlaybackError, check_native, ffi};
use std::ptr::NonNull;
use std::sync::Mutex;

static LOGGING_OWNER: Mutex<LoggingOwner> = Mutex::new(LoggingOwner {
    anchor: None,
    leases: 0,
});

struct LoggingOwner {
    anchor: Option<QuietAnchor>,
    leases: usize,
}

struct QuietAnchor(NonNull<ffi::Handle>);

// SAFETY: the anchor has one owner behind LOGGING_OWNER. Every native call and
// destruction is serialized by that mutex; libmpv permits calls across threads.
unsafe impl Send for QuietAnchor {}

impl QuietAnchor {
    /// Initializes the first, idle engine solely to retain libmpv's private FFmpeg log callback.
    fn create() -> Result<Self, PlaybackError> {
        // SAFETY: creation has no inputs; the RAII owner also cleans up failed initialization.
        let pointer =
            NonNull::new(unsafe { ffi::mpv_create() }).ok_or(PlaybackError::Allocation)?;
        let anchor = Self(pointer);
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
            (c"ao", c"null"),
            (c"idle", c"yes"),
        ];

        for (name, value) in options {
            // SAFETY: the sole handle and both static strings remain valid throughout the call.
            let result = unsafe {
                ffi::mpv_set_option_string(anchor.0.as_ptr(), name.as_ptr(), value.as_ptr())
            };
            check_native("native logging configuration", result)?;
        }

        // SAFETY: this configured handle has not been initialized or exposed elsewhere.
        let result = unsafe { ffi::mpv_initialize(anchor.0.as_ptr()) };
        check_native("native logging initialization", result)?;
        Ok(anchor)
    }
}

impl Drop for QuietAnchor {
    /// Releases the private log owner only after all actual engines have been destroyed.
    fn drop(&mut self) {
        // SAFETY: the sole owner is still protected by LOGGING_OWNER, including failed startup.
        unsafe { ffi::mpv_terminate_destroy(self.0.as_ptr()) };
    }
}

/// Must be acquired before engine creation and dropped after engine destruction.
pub(crate) struct NativeLease {
    _private: (),
}

impl NativeLease {
    /// Retains one idle anchor for all overlapping engines; no media, device or decoding is opened.
    pub(crate) fn acquire() -> Result<Self, PlaybackError> {
        // No invariant-changing operation below can panic. Recovering poisoning
        // preserves cleanup even if a caller unwinds while owning a lease.
        let mut owner = LOGGING_OWNER
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let leases = owner
            .leases
            .checked_add(1)
            .ok_or(PlaybackError::Allocation)?;
        if owner.anchor.is_none() {
            owner.anchor = Some(QuietAnchor::create()?);
        }

        owner.leases = leases;
        Ok(Self { _private: () })
    }
}

impl Drop for NativeLease {
    /// Serializes the last release with acquisition so a new engine cannot inherit a dying log owner.
    fn drop(&mut self) {
        let mut owner = LOGGING_OWNER
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        owner.leases -= 1;
        if owner.leases == 0 {
            // libmpv resets FFmpeg's global callback when its first engine dies,
            // even when other engines remain. Destroy the anchor last, under the
            // lock, to cover both overlapping jobs and immediate restarts.
            drop(owner.anchor.take());
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{AudioOutput, LocalPlayer};

    /// Runs the native global-state case in a private child so unrelated parallel tests cannot mask the lifetime bug.
    #[test]
    fn overlapping_handles_keep_native_diagnostics_private() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "native_logging::tests::logging_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "native logging child failed");
        assert!(
            output.stderr.is_empty(),
            "native diagnostics escaped the closed logging boundary"
        );
    }

    /// Drops the first initialized playback handle while another remains, then emits a test-only native diagnostic.
    #[test]
    #[ignore = "owned child of overlapping_handles_keep_native_diagnostics_private"]
    fn logging_child() {
        use std::io::Write;

        let path =
            std::env::temp_dir().join(format!("resolibre-log-owner-{}.wav", std::process::id()));
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(16036_u32.to_le_bytes());
        bytes.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
        bytes.extend(8000_u32.to_le_bytes());
        bytes.extend(16000_u32.to_le_bytes());
        bytes.extend(b"\x02\0\x10\0data");
        bytes.extend(16000_u32.to_le_bytes());
        bytes.resize(16044, 0);
        let mut fixture = std::fs::File::create_new(&path).unwrap();
        fixture.write_all(&bytes).unwrap();
        drop(fixture);
        let first = LocalPlayer::open(&path, AudioOutput::Silent).unwrap();
        let second = LocalPlayer::open(&path, AudioOutput::Silent).unwrap();
        drop(first);
        type AvLog = unsafe extern "C" fn(*mut std::ffi::c_void, i32, *const std::ffi::c_char, ...);
        // SAFETY: the live libmpv dependency graph retains libavutil; av_log has this documented stable variadic ABI.
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"av_log".as_ptr()) };
        assert!(!symbol.is_null());
        // SAFETY: dlsym resolved av_log from the already-loaded library; the pointer is used only while it stays loaded.
        let log: AvLog = unsafe { std::mem::transmute(symbol) };
        // SAFETY: the format has no substitutions; null context is supported and the literal outlives this call.
        unsafe {
            log(
                std::ptr::null_mut(),
                16,
                c"private-native-diagnostic\n".as_ptr(),
            );
        }
        drop(second);
        let owner = super::LOGGING_OWNER.lock().unwrap();
        assert_eq!(owner.leases, 0);
        assert!(owner.anchor.is_none(), "idle anchor survived its last user");
        drop(owner);

        // Reacquisition must initialize a fresh owner after the complete teardown.
        let restarted = LocalPlayer::open(&path, AudioOutput::Silent).unwrap();
        // SAFETY: av_log remains loaded and this static format has no substitutions.
        unsafe {
            log(
                std::ptr::null_mut(),
                16,
                c"private-native-diagnostic\n".as_ptr(),
            )
        };
        drop(restarted);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(path).unwrap();
    }
}
