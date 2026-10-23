//! Private application storage preparation; media locations are never supplied to this module.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

pub(crate) const MAX_DATABASE_BYTES: u64 = 256 * 1024 * 1024;

/// Creates a private dedicated directory and rejects links, special files and oversized databases.
/// The platform supplies a trusted, absolute profile location; this function does not discover one.
pub(crate) fn database_path(directory: &Path) -> Result<PathBuf, ()> {
    if !directory.is_absolute() {
        return Err(());
    }
    ensure_directory(directory)?;
    for name in [
        "analysis.sqlite3",
        "analysis.sqlite3-journal",
        "analysis.sqlite3-wal",
        "analysis.sqlite3-shm",
    ] {
        check_file(&directory.join(name))?;
    }
    let path = directory.join("analysis.sqlite3");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(()),
    }
    check_file(&path)?;
    Ok(path)
}

/// Creates only absent ancestors; existing directories must be real directories, not links.
fn ensure_directory(directory: &Path) -> Result<(), ()> {
    match directory.symlink_metadata() {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(());
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = directory.parent().ok_or(())?;
            if !parent.exists() {
                ensure_directory(parent)?;
            } else if !parent.symlink_metadata().map_err(|_| ())?.is_dir() {
                return Err(());
            }
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(directory).map_err(|_| ())
        }
        Err(_) => Err(()),
    }
}

/// Checks existing SQLite files before native open; private parent permissions protect journal creation.
fn check_file(path: &Path) -> Result<(), ()> {
    match path.symlink_metadata() {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.len() > MAX_DATABASE_BYTES {
                return Err(());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
                    return Err(());
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(()),
    }
}
