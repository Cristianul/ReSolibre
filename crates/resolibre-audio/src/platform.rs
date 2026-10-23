//! Native file and pipe operations shared by playback and bounded helper jobs.

#[cfg(windows)]
mod stream;
#[cfg(windows)]
pub mod windows;
#[cfg(all(test, windows))]
mod windows_tests;

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Opens media read-only and validates the opened object; no pathname is reopened by a decoder.
pub(crate) fn open_media(path: &Path, follow_links: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let flags = if follow_links {
            libc::O_NONBLOCK
        } else {
            libc::O_NONBLOCK | libc::O_NOFOLLOW
        };
        options.custom_flags(flags);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        windows::validate_media_path(path)?;
        // Deny concurrent writers/deletion while the retained source is in use.
        options.share_mode(1);
        if !follow_links {
            options.custom_flags(0x0020_0000);
        }
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("Input must be a regular file"));
    }
    #[cfg(windows)]
    if !follow_links && windows::is_reparse(&metadata) {
        return Err(io::Error::other("Linked input is not allowed"));
    }
    Ok(file)
}

/// Reopens the retained file with an independent cursor without resolving its original pathname.
pub(crate) fn independent_file(file: &File) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        File::open(format!("/proc/self/fd/{}", file.as_raw_fd()))
    }
    #[cfg(windows)]
    {
        windows::independent_file(file)
    }
}

/// Reads the small format header without changing any retained playback cursor.
pub(crate) fn read_header(file: &File, header: &mut [u8]) -> io::Result<()> {
    use std::io::Read;
    independent_file(file)?.read_exact(header)
}

/// Resolves bundled helpers next to the application on Windows; Linux uses installed tools.
pub(crate) fn helper(program: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        let executable = std::env::current_exe().ok().and_then(|path| {
            path.parent()
                .map(|parent| parent.join(format!("{program}.exe")))
        });
        let path = executable.unwrap_or_default();
        let mut command = std::process::Command::new(path);
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
        command
    }
    #[cfg(unix)]
    {
        std::process::Command::new(program)
    }
}

#[cfg(unix)]
pub use std::os::unix::net::UnixStream as PipeReader;
#[cfg(windows)]
pub use windows::PipeReader;

/// Returns a private output channel and an owned child stdout endpoint.
pub fn output_pipe() -> io::Result<(PipeReader, std::process::Stdio)> {
    #[cfg(unix)]
    {
        use std::os::fd::OwnedFd;
        let (reader, writer) = std::os::unix::net::UnixStream::pair()?;
        Ok((reader, std::process::Stdio::from(OwnedFd::from(writer))))
    }
    #[cfg(windows)]
    {
        windows::output_pipe()
    }
}
