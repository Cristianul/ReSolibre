//! Private bounded files shared by session and preset storage; called only by owned jobs.

use crate::profile_files;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Resolves only the current OS user's config area, never a music-file-relative destination.
#[cfg(unix)]
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
        Ok(metadata) if metadata.is_dir() && !profile_files::linked(&metadata) => Ok(true),
        Ok(_) => Err("Invalid application storage directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Cannot inspect application storage"),
    }
}

/// Creates only application storage with private permissions, never a music-relative path.
fn ensure_directory(directory: &Path) -> Result<(), &'static str> {
    if !check_directory(directory)? {
        profile_files::create_directory(directory)
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
    let file = match profile_files::options()
        .read(true)
        .open(directory.join(name))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot read application settings"),
    };
    if !profile_files::regular(&file).map_err(|_| "Cannot inspect application settings")? {
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
        && (!metadata.is_file() || profile_files::linked(&metadata))
    {
        return Err("Invalid application settings destination");
    }
    let temporary = directory.join(format!(".{name}-{}.tmp", std::process::id()));
    let mut file = profile_files::options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "Cannot create settings transaction")?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|_| "Cannot write application settings")?;
        file.sync_all()
            .map_err(|_| "Cannot flush application settings")?;
        profile_files::replace(&temporary, &target)
            .map_err(|_| "Cannot replace application settings")?;
        profile_files::sync_directory(directory).map_err(|_| "Cannot flush settings directory")
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Owns the lock independently of transient descriptor copies in forked native children.
pub struct SessionLease {
    file: File,
    locked: std::sync::Mutex<bool>,
}

impl SessionLease {
    /// Requests exclusive reset access after the caller has stopped all profile workers.
    /// A failure must not resume storage: another reset may have won the Windows upgrade gap.
    pub fn make_exclusive(&self) -> Result<(), &'static str> {
        let mut locked = self.locked.lock().map_err(|_| "Profile lock unavailable")?;
        #[cfg(windows)]
        if *locked {
            // LockFileEx cannot upgrade a shared lock in place. No profile work is active here.
            self.file
                .unlock()
                .map_err(|_| "Cannot release profile access lock")?;
            *locked = false;
        }
        if self.file.try_lock().is_err() {
            #[cfg(windows)]
            {
                *locked = self.file.try_lock_shared().is_ok();
            }
            return Err("Close other ReSolibre windows before resetting");
        }
        *locked = true;
        Ok(())
    }
}

impl Drop for SessionLease {
    /// Explicitly unlocks after the last storage owner exits, even if a child has not reached exec yet.
    fn drop(&mut self) {
        let locked = self
            .locked
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if *locked && self.file.unlock().is_err() {
            eprintln!("warn: session storage lock release failed");
        }
    }
}

/// Holds an exclusive nonblocking profile lease; another instance cannot overwrite its session.
pub fn lock_session(directory: &Path) -> Result<SessionLease, &'static str> {
    ensure_directory(directory)?;
    let file = profile_files::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("session.lock"))
        .map_err(|_| "Cannot open session storage lock")?;
    if !profile_files::regular(&file).map_err(|_| "Cannot inspect session storage lock")? {
        return Err("Invalid session storage lock");
    }
    file.try_lock()
        .map_err(|_| "Session storage is in use; this window will not save its session")?;
    Ok(SessionLease {
        file,
        locked: std::sync::Mutex::new(true),
    })
}

/// Protects the profile for a window's lifetime; reset needs exclusive access to this same inode.
pub fn share_profile(directory: &Path) -> Result<SessionLease, &'static str> {
    ensure_directory(directory)?;
    let file = profile_files::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("profile.lock"))
        .map_err(|_| "Cannot open profile access lock")?;
    if !profile_files::regular(&file).map_err(|_| "Cannot inspect profile access lock")? {
        return Err("Invalid profile access lock");
    }
    file.try_lock_shared()
        .map_err(|_| "Profile reset in progress; reopen ReSolibre")?;
    Ok(SessionLease {
        file,
        locked: std::sync::Mutex::new(true),
    })
}

/// Keeps Windows settings in this user's local application-data directory.
#[cfg(windows)]
pub fn default_directory() -> Result<PathBuf, &'static str> {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("No user configuration directory")?;
    Ok(root.join("ReSolibre"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A duplicate descriptor models the fork/exec window; it must not outlive the application's lock ownership.
    #[test]
    fn session_lease_releases_despite_inherited_descriptor() {
        let directory =
            std::env::temp_dir().join(format!("resolibre-lease-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let lease = lock_session(&directory).unwrap();
        let inherited = lease.file.try_clone().unwrap();
        assert!(lock_session(&directory).is_err());
        drop(lease);
        let restarted = lock_session(&directory).unwrap();
        drop(inherited);
        assert!(lock_session(&directory).is_err());
        drop(restarted);
        fs::remove_dir_all(directory).unwrap();
    }
}
