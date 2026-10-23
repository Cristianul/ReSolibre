//! Read-only libmpv streams retain a file handle instead of resolving media paths again.

use crate::{LocalPlayer, PlaybackError, check_native, ffi};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

#[repr(C)]
struct StreamInfo {
    cookie: *mut c_void,
    read: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, u64) -> i64>,
    seek: Option<unsafe extern "C" fn(*mut c_void, i64) -> i64>,
    size: Option<unsafe extern "C" fn(*mut c_void) -> i64>,
    close: Option<unsafe extern "C" fn(*mut c_void)>,
    cancel: Option<unsafe extern "C" fn(*mut c_void)>,
}

#[link(name = "libmpv-2", kind = "raw-dylib")]
unsafe extern "C" {
    /// Registers callbacks retained until mpv_terminate_destroy returns.
    fn mpv_stream_cb_add_ro(
        handle: *mut ffi::Handle,
        protocol: *const c_char,
        data: *mut c_void,
        open: unsafe extern "C" fn(*mut c_void, *mut c_char, *mut StreamInfo) -> c_int,
    ) -> c_int;
}

impl LocalPlayer {
    /// Registers the boxed retained input, whose address stays stable until engine teardown.
    pub(crate) fn load_retained_input(&mut self) -> Result<(), PlaybackError> {
        let data = (&*self.input as *const File).cast_mut().cast();
        // SAFETY: input is boxed and outlives native teardown; callbacks own independent cursors.
        let code = unsafe {
            mpv_stream_cb_add_ro(self.handle.as_ptr(), c"resolibre".as_ptr(), data, open)
        };
        check_native("register local input", code)?;
        self.command(&["loadfile", "resolibre://input", "replace"], "load")
    }
}

/// Accepts only the fixed adapter URI and allocates one independent, read-only cursor.
unsafe extern "C" fn open(data: *mut c_void, uri: *mut c_char, info: *mut StreamInfo) -> c_int {
    if data.is_null() || uri.is_null() || info.is_null() {
        return -13;
    }
    // SAFETY: mpv supplies a terminated URI and registration retains a live File.
    let (uri, input) = unsafe { (CStr::from_ptr(uri), &*data.cast::<File>()) };
    if uri != c"resolibre://input" {
        return -13;
    }
    let Ok(file) = super::independent_file(input) else {
        return -13;
    };
    let stream = StreamInfo {
        cookie: Box::into_raw(Box::new(file)).cast(),
        read: Some(read),
        seek: Some(seek),
        size: Some(size),
        close: Some(close),
        cancel: None,
    };
    // SAFETY: mpv lends the writable ABI struct during this call and closes the cookie once.
    unsafe {
        info.write(stream);
    }
    0
}

/// Copies a bounded chunk into mpv's borrowed buffer; short reads and errors follow stream_cb.h.
unsafe extern "C" fn read(cookie: *mut c_void, buffer: *mut c_char, length: u64) -> i64 {
    let count = length.min(1024 * 1024) as usize;
    if count == 0 {
        return 0;
    }
    if cookie.is_null() || buffer.is_null() {
        return -1;
    }
    // SAFETY: mpv serializes stream callbacks and supplies a writable buffer of at least count bytes.
    let (file, bytes) = unsafe {
        (
            &mut *cookie.cast::<File>(),
            std::slice::from_raw_parts_mut(buffer.cast(), count),
        )
    };
    file.read(bytes).map_or(-1, |count| count as i64)
}

/// Seeks only within nonnegative, representable local offsets.
unsafe extern "C" fn seek(cookie: *mut c_void, offset: i64) -> i64 {
    if cookie.is_null() || offset < 0 {
        return -1;
    }
    // SAFETY: the cookie owns a File and mpv serializes read/seek operations.
    let file = unsafe { &mut *cookie.cast::<File>() };
    file.seek(SeekFrom::Start(offset as u64))
        .ok()
        .and_then(|value| i64::try_from(value).ok())
        .unwrap_or(-1)
}

/// Returns the retained file's size, never pathname metadata.
unsafe extern "C" fn size(cookie: *mut c_void) -> i64 {
    if cookie.is_null() {
        return -1;
    }
    // SAFETY: the cookie remains live until close and metadata does not change the cursor.
    let file = unsafe { &*cookie.cast::<File>() };
    file.metadata()
        .ok()
        .and_then(|value| i64::try_from(value.len()).ok())
        .unwrap_or(-1)
}

/// Releases the one cookie created by open after mpv has stopped its callbacks.
unsafe extern "C" fn close(cookie: *mut c_void) {
    if !cookie.is_null() {
        // SAFETY: mpv invokes close exactly once for a successfully opened stream.
        drop(unsafe { Box::from_raw(cookie.cast::<File>()) });
    }
}
