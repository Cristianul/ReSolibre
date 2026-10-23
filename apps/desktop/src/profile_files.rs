//! Native profile file options; Windows inherits the current user's profile ACL.

use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::path::Path;

/// Applies private creation permissions and rejects following leaf links when opening settings.
pub fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(0x20000 | 0x800);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    options
}

/// Rejects Windows junctions and other reparse objects as well as ordinary symbolic links.
pub fn linked(metadata: &Metadata) -> bool {
    #[cfg(unix)]
    {
        metadata.file_type().is_symlink()
    }
    #[cfg(windows)]
    {
        resolibre_audio::platform::windows::is_reparse(metadata)
    }
}

/// Requires a regular, non-linked object after opening, before reading or locking it.
pub fn regular(file: &File) -> io::Result<bool> {
    let metadata = file.metadata()?;
    Ok(metadata.is_file() && !linked(&metadata))
}

/// Creates application directories, inheriting the user's Windows ACL or applying Unix 0700.
pub fn create_directory(directory: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)
}

/// Flushes directory entries where supported; Windows replacements use write-through instead.
pub fn sync_directory(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(directory)?.sync_all()
    }
    #[cfg(windows)]
    {
        let _ = directory;
        Ok(())
    }
}

/// Atomically replaces a settings file; Windows requests durable completion of the move.
pub fn replace(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::rename(source, destination)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<_> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: both terminated paths are borrowed for this synchronous operation only.
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
