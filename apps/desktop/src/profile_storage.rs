//! Private bounded files shared by session and preset storage; called only by owned jobs.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const NO_FOLLOW_NONBLOCK: i32 = 0x20000 | 0x800;

/// Resolves only the current OS user's config area, never a music-file-relative destination.
pub fn default_directory() -> Result<PathBuf, &'static str> {
    if let Some(root) = std::env::var_os("XDG_CONFIG_HOME") {
        let root = PathBuf::from(root);
        if root.is_absolute() {
            return Ok(root.join("resolibre"));
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or("No user configuration directory")?;
    Ok(home.join(".config/resolibre"))
}

/// Rejects a replaced app root; OS-configured parent directories remain trusted.
fn check_directory(directory: &Path) -> Result<bool, &'static str> {
    match directory.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err("Invalid application storage directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Cannot inspect application storage"),
    }
}

/// Creates only application storage with private permissions, never a music-relative path.
fn ensure_directory(directory: &Path) -> Result<(), &'static str> {
    if !check_directory(directory)? {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .map_err(|_| "Cannot create application storage")?;
    }
    check_directory(directory)?;
    Ok(())
}

/// Reads a bounded regular file without following file symlinks or blocking on FIFOs.
pub fn read(directory: &Path, name: &str, limit: u64) -> Result<Option<Vec<u8>>, &'static str> {
    if !check_directory(directory)? {
        return Ok(None);
    }
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW_NONBLOCK)
        .open(directory.join(name))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot read application settings"),
    };
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect application settings")?
        .is_file()
    {
        return Err("Application settings must be a regular file");
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read application settings")?;
    if bytes.len() as u64 > limit {
        return Err("Application settings exceed the size limit");
    }
    Ok(Some(bytes))
}

/// Syncs a private atomic replacement; pre-rename failure preserves the previous file.
pub fn write(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), &'static str> {
    ensure_directory(directory)?;
    let target = directory.join(name);
    if let Ok(metadata) = target.symlink_metadata()
        && !metadata.is_file()
    {
        return Err("Invalid application settings destination");
    }
    let temporary = directory.join(format!(".{name}-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| "Cannot create settings transaction")?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|_| "Cannot write application settings")?;
        file.sync_all()
            .map_err(|_| "Cannot flush application settings")?;
        fs::rename(&temporary, &target).map_err(|_| "Cannot replace application settings")?;
        File::open(directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "Cannot flush settings directory")
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Owns the lock independently of transient descriptor copies in forked native children.
pub struct SessionLease(File);

impl SessionLease {
    /// Upgrades the activity lease only when no other window is using this profile.
    pub fn make_exclusive(&self) -> Result<(), &'static str> {
        self.0
            .try_lock()
            .map_err(|_| "Close other ReSolibre windows before resetting")
    }
}

impl Drop for SessionLease {
    /// Explicitly unlocks after the last storage owner exits, even if a child has not reached exec yet.
    fn drop(&mut self) {
        if self.0.unlock().is_err() {
            eprintln!("warn: session storage lock release failed");
        }
    }
}

/// Holds an exclusive nonblocking profile lease; another instance cannot overwrite its session.
pub fn lock_session(directory: &Path) -> Result<SessionLease, &'static str> {
    ensure_directory(directory)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(NO_FOLLOW_NONBLOCK)
        .open(directory.join("session.lock"))
        .map_err(|_| "Cannot open session storage lock")?;
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect session storage lock")?
        .is_file()
    {
        return Err("Invalid session storage lock");
    }
    file.try_lock()
        .map_err(|_| "Session storage is in use; this window will not save its session")?;
    Ok(SessionLease(file))
}

/// Protects the profile for a window's lifetime; reset needs exclusive access to this same inode.
pub fn share_profile(directory: &Path) -> Result<SessionLease, &'static str> {
    ensure_directory(directory)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(NO_FOLLOW_NONBLOCK)
        .open(directory.join("profile.lock"))
        .map_err(|_| "Cannot open profile access lock")?;
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect profile access lock")?
        .is_file()
    {
        return Err("Invalid profile access lock");
    }
    file.try_lock_shared()
        .map_err(|_| "Profile reset in progress; reopen ReSolibre")?;
    Ok(SessionLease(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A duplicate descriptor models the fork/exec window; it must not outlive the application's lock ownership.
    #[test]
    fn session_lease_releases_despite_inherited_descriptor() {
        let directory = std::env::temp_dir().join(format!("resolibre-lease-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let lease = lock_session(&directory).unwrap();
        let inherited = lease.0.try_clone().unwrap();
        assert!(lock_session(&directory).is_err());
        drop(lease);
        let restarted = lock_session(&directory).unwrap();
        drop(inherited);
        assert!(lock_session(&directory).is_err());
        drop(restarted);
        fs::remove_dir_all(directory).unwrap();
    }
}
