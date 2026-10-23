//! Own the bundled tap's Linux loading and C ABI. Other OS adapters supply their own loader.

use std::ffi::{c_char, c_void};
use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::sync::OnceLock;

pub const BLOCK_FRAMES: usize = 1024;
pub const MAX_CHANNELS: usize = 32;

#[repr(C)]
pub struct Block {
    pub first_frame: u64,
    pub instance: u32,
    pub rate: u32,
    pub channels: u32,
    pub frames: u32,
    pub samples: [f32; BLOCK_FRAMES * MAX_CHANNELS],
}

type Open = unsafe extern "C" fn() -> u32;
type Read = unsafe extern "C" fn(u32, *mut Block) -> u32;
type Close = unsafe extern "C" fn(u32);

struct Library {
    _library: libloading::Library,
    file: File,
    open: Open,
    read: Read,
    close: Close,
    barrier: Open,
}

static LIBRARY: OnceLock<Result<Library, &'static str>> = OnceLock::new();

/// One reader/context, retained until LocalPlayer has stopped all native callbacks.
pub struct Tap {
    library: &'static Library,
    token: u32,
    block: Box<Block>,
}

impl Tap {
    /// Allocates bounded transport independently of media input and probes native filter availability.
    pub fn new() -> Result<Self, &'static str> {
        let library = LIBRARY.get_or_init(load).as_ref().map_err(|error| *error)?;
        // SAFETY: initialization completed once and the library lives for the process lifetime.
        let token = unsafe { (library.open)() };
        if token == 0 {
            return Err("Live spectrum capacity unavailable");
        }
        Ok(Self {
            library,
            token,
            block: Box::new(Block {
                first_frame: 0,
                instance: 0,
                rate: 0,
                channels: 0,
                frames: 0,
                samples: [0.0; BLOCK_FRAMES * MAX_CHANNELS],
            }),
        })
    }

    /// Builds only owned numeric syntax and a sealed descriptor path, never user-provided expressions.
    pub fn graph(&self, channels: u32) -> String {
        format!(
            "@resolibre_live:lavfi=[asplit[out][tap];[tap]ladspa=file=/proc/self/fd/{}:plugin=resolibre_{channels}:controls=c0={},anullsink]",
            self.library.file.as_raw_fd(),
            self.token
        )
    }

    /// Copies one block through the sole-reader ABI; rejects invalid native dimensions before slicing.
    pub fn read(&mut self) -> Result<Option<&Block>, &'static str> {
        // SAFETY: this context has one reader; the boxed repr(C) destination has the exact C capacity.
        let available = unsafe { (self.library.read)(self.token, self.block.as_mut()) };
        if available == 0 {
            return Ok(None);
        }
        let block = &self.block;
        if available != 1
            || block.instance == 0
            || !(1..=MAX_CHANNELS as u32).contains(&block.channels)
            || !(1..=BLOCK_FRAMES as u32).contains(&block.frames)
            || !(1..=1_536_000).contains(&block.rate)
        {
            return Err("Invalid live spectrum data");
        }
        Ok(Some(block))
    }

    /// Captures the last allocated graph identity before a seek; prior instances cannot pass the barrier.
    pub fn barrier(&self) -> u32 {
        // SAFETY: the library exposes a read-only atomic counter with no pointer arguments.
        unsafe { (self.library.barrier)() }
    }
}

impl Drop for Tap {
    /// LocalPlayer destroys mpv first, so no native producer can still reference this context.
    fn drop(&mut self) {
        // SAFETY: the unique context owner stops its reader and all producers before close.
        unsafe { (self.library.close)(self.token) };
    }
}

/// Loads only embedded application bytes from a sealed anonymous file; never searches disk for plugins.
fn load() -> Result<Library, &'static str> {
    type FindFilter = unsafe extern "C" fn(*const c_char) -> *const c_void;
    let process = libloading::os::unix::Library::this();
    // SAFETY: avfilter's documented lookup ABI returns a borrowed opaque pointer, never dereferenced.
    let find = unsafe { process.get::<FindFilter>(b"avfilter_get_by_name\0") }
        .map_err(|_| "Native live spectrum filter unavailable")?;
    for name in [c"ladspa", c"asplit", c"anullsink"] {
        // SAFETY: each argument is a static NUL-terminated filter name.
        if unsafe { find(name.as_ptr()) }.is_null() {
            return Err("Native live spectrum filter unavailable");
        }
    }

    // SAFETY: static name and validated flags; ownership transfers immediately into File.
    let descriptor = unsafe {
        libc::memfd_create(
            c"resolibre-tap".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if descriptor < 0 {
        return Err("Cannot allocate live spectrum filter");
    }
    // SAFETY: memfd returned a new owned descriptor, which is closed exactly once by File.
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    file.write_all(include_bytes!(env!("RESOUL_TAP_BINARY")))
        .map_err(|_| "Cannot prepare live spectrum filter")?;
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    // SAFETY: fcntl receives the owned descriptor and an integer seal mask.
    if unsafe { libc::fcntl(descriptor, libc::F_ADD_SEALS, seals) } != 0 {
        return Err("Cannot seal live spectrum filter");
    }
    // SAFETY: these are our compiled bytes, now immutable; no media/environment-provided library is loaded.
    let library = unsafe { libloading::Library::new(format!("/proc/self/fd/{descriptor}")) }
        .map_err(|_| "Cannot load live spectrum filter")?;
    // SAFETY: the bundled source defines these exact signatures; the Library owner outlives copied functions.
    let (open, read, close, barrier) = unsafe {
        let initialize = library
            .get::<Open>(b"resolibre_tap_initialize\0")
            .map_err(|_| "Invalid tap ABI")?;
        if initialize() != 1 {
            return Err("Invalid tap ABI");
        }
        (
            *library
                .get::<Open>(b"resolibre_tap_open\0")
                .map_err(|_| "Invalid tap ABI")?,
            *library
                .get::<Read>(b"resolibre_tap_read\0")
                .map_err(|_| "Invalid tap ABI")?,
            *library
                .get::<Close>(b"resolibre_tap_close\0")
                .map_err(|_| "Invalid tap ABI")?,
            *library
                .get::<Open>(b"resolibre_tap_barrier\0")
                .map_err(|_| "Invalid tap ABI")?,
        )
    };
    Ok(Library {
        _library: library,
        file,
        open,
        read,
        close,
        barrier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Isolated library storage enforces capacity, retires stale tokens and seals executable bytes.
    #[test]
    fn bounded_contexts_and_sealed_code() {
        let library = load().unwrap();
        let mut file = library.file.try_clone().unwrap();
        assert!(file.write_all(b"replacement").is_err());
        let mut block = Box::new(Block {
            first_frame: 0,
            instance: 0,
            rate: 0,
            channels: 0,
            frames: 0,
            samples: [0.0; BLOCK_FRAMES * MAX_CHANNELS],
        });
        // SAFETY: this independently loaded library has no native engines/producers. Each token is closed once.
        unsafe {
            let mut tokens = Vec::new();
            for _ in 0..16 {
                let token = (library.open)();
                assert_ne!(token, 0);
                assert!(!tokens.contains(&token));
                tokens.push(token);
            }
            assert_eq!((library.open)(), 0);
            let old = tokens.remove(0);
            (library.close)(old);
            let replacement = (library.open)();
            assert_ne!(replacement, 0);
            assert_ne!(replacement, old);
            assert_eq!((library.read)(old, block.as_mut()), 0);
            assert_eq!((library.read)(replacement, std::ptr::null_mut()), 0);
            (library.close)(old);
            assert_eq!((library.open)(), 0, "stale close released a newer context");
            tokens.push(replacement);
            for token in tokens {
                (library.close)(token);
            }
        }
    }
}
