//! Bounded PCM output endpoints; Windows uses a local-only named pipe, Linux retained descriptors.

use std::fs::File;
use std::io::{self, Read};

#[cfg(unix)]
pub(super) type Pipe = File;
#[cfg(windows)]
pub(super) type Pipe = crate::platform::PipeReader;

/// Creates an owned output destination; only adapter-generated names reach the decoder.
#[cfg(unix)]
pub(super) fn create() -> io::Result<(Pipe, Option<File>, String)> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let mut descriptors = [-1; 2];
    // SAFETY: pipe2 writes two initialized descriptor outputs and transfers nothing on failure.
    if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful pipe2 returned two unique owned descriptors.
    let (read, write) = unsafe {
        (
            File::from_raw_fd(descriptors[0]),
            File::from_raw_fd(descriptors[1]),
        )
    };
    let path = format!("/proc/self/fd/{}", write.as_raw_fd());
    Ok((read, Some(write), path))
}

/// Creates a single local-only pipe instance; collisions fail instead of joining another server.
#[cfg(windows)]
pub(super) fn create() -> io::Result<(Pipe, Option<File>, String)> {
    let (reader, path) = crate::platform::windows::named_pipe()?;
    Ok((reader, None, path))
}

/// Waits at most 20 ms before a read, leaving cancellation and event polling to the owner.
#[cfg(unix)]
pub(super) fn read(pipe: &mut Pipe, bytes: &mut [u8]) -> io::Result<usize> {
    use std::os::fd::AsRawFd;
    let mut descriptor = libc::pollfd {
        fd: pipe.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one initialized descriptor remains owned during the bounded poll call.
    let ready = unsafe { libc::poll(&mut descriptor, 1, 20) };
    if ready < 0 {
        return Err(io::Error::last_os_error());
    }
    if ready == 0 {
        return Err(io::ErrorKind::WouldBlock.into());
    }
    pipe.read(bytes)
}

/// Waits on native completion with bounded cancellation checkpoints.
#[cfg(windows)]
pub(super) fn read(pipe: &mut Pipe, bytes: &mut [u8]) -> io::Result<usize> {
    pipe.read(bytes)
}
