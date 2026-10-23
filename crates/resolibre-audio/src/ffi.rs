//! Private subset of the libmpv client API 2.x ABI, checked against mpv 0.41.0.
//! Handles and event payloads never escape the owning adapter.

use std::ffi::{c_char, c_int, c_ulong, c_void};

/// Opaque native allocation; Rust never constructs or dereferences its contents.
#[repr(C)]
pub struct Handle {
    _private: [u8; 0],
}

/// Borrowed event valid until the next wait call or handle destruction.
#[repr(C)]
pub struct Event {
    pub id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

/// End-file payload layout from client.h, including fields unused by this slice.
#[repr(C)]
pub struct EndFile {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

/// Hook payload is borrowed until the next event wait, and its ID must be continued exactly once.
#[repr(C)]
pub struct EventHook {
    pub name: *const c_char,
    pub id: u64,
}
pub const EVENT_HOOK: c_int = 25;

pub const EVENT_SHUTDOWN: c_int = 1;
pub const EVENT_END_FILE: c_int = 7;
pub const EVENT_FILE_LOADED: c_int = 8;
pub const EVENT_QUEUE_OVERFLOW: c_int = 24;
pub const END_REASON_EOF: c_int = 0;
pub const END_REASON_ERROR: c_int = 4;
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_DOUBLE: c_int = 5;
pub const FORMAT_STRING: c_int = 1;
pub const FORMAT_INT64: c_int = 4;
pub const ERROR_PROPERTY_UNAVAILABLE: c_int = -10;
pub const ERROR_COMMAND: c_int = -12;

#[link(name = "mpv")]
unsafe extern "C" {
    /// Releases an allocation returned by mpv, never a Rust-owned pointer.
    pub fn mpv_free(data: *mut c_void);
    /// Reads the runtime ABI version without creating an engine.
    pub fn mpv_client_api_version() -> c_ulong;
    /// Allocates an uninitialized client, or returns null on failure.
    pub fn mpv_create() -> *mut Handle;
    /// Initializes a configured client; a negative result is an engine error.
    pub fn mpv_initialize(handle: *mut Handle) -> c_int;
    /// Stops playback and frees the sole client, including initialization failures.
    pub fn mpv_terminate_destroy(handle: *mut Handle);
    /// Copies a null-terminated option name/value while the handle is alive.
    pub fn mpv_set_option_string(
        handle: *mut Handle,
        name: *const c_char,
        value: *const c_char,
    ) -> c_int;
    /// Executes a null-terminated vector of null-terminated command arguments.
    pub fn mpv_command(handle: *mut Handle, args: *const *const c_char) -> c_int;
    /// Copies a property into storage matching the requested mpv_format.
    pub fn mpv_get_property(
        handle: *mut Handle,
        name: *const c_char,
        format: c_int,
        data: *mut c_void,
    ) -> c_int;
    /// Registers an owned preload barrier before the file can start decoding.
    pub fn mpv_hook_add(
        handle: *mut Handle,
        userdata: u64,
        name: *const c_char,
        priority: c_int,
    ) -> c_int;
    /// Continues exactly the hook ID returned to this handle.
    pub fn mpv_hook_continue(handle: *mut Handle, id: u64) -> c_int;
    /// Borrows the next event for at most the requested timeout in seconds.
    pub fn mpv_wait_event(handle: *mut Handle, timeout: f64) -> *const Event;
}

// Audio graph rebuilt; adapter must refresh source-rate and runtime filter observations.
pub const EVENT_AUDIO_RECONFIG: c_int = 18;
