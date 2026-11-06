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
        Ok(metadata) => check_directory(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = directory.parent().ok_or(())?;
            if !parent.exists() {
                ensure_directory(parent)?;
            } else {
                let metadata = parent.symlink_metadata().map_err(|_| ())?;
                if !metadata.is_dir() || linked(&metadata) {
                    return Err(());
                }
            }
            let mut builder = fs::DirBuilder::new();
            builder.recursive(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(directory) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Another cache worker may create it first; its entry still needs validation.
                    check_directory(&directory.symlink_metadata().map_err(|_| ())?)
                }
                Err(_) => Err(()),
            }
        }
        Err(_) => Err(()),
    }
}

/// Requires a real private cache directory even when a concurrent worker created it.
fn check_directory(metadata: &fs::Metadata) -> Result<(), ()> {
    if !metadata.is_dir() || linked(metadata) {
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

/// Checks existing SQLite files before native open; private parent permissions protect journal creation.
fn check_file(path: &Path) -> Result<(), ()> {
    #[cfg(windows)]
    {
        check_windows_file(path)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        match path.symlink_metadata() {
            Ok(metadata) => {
                if !metadata.is_file() || linked(&metadata) || metadata.len() > MAX_DATABASE_BYTES {
                    return Err(());
                }
                if metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
                    return Err(());
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(()),
        }
    }
}

/// Treats Windows reparse points, including junctions, as untrusted profile links.
fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        metadata.file_type().is_symlink()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
}

/// Rejects hardlinks and leaf replacement with a reparse point before SQLite receives a path.
#[cfg(windows)]
fn check_windows_file(path: &Path) -> Result<(), ()> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(0x0020_0000)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(()),
    };
    check_windows_handle(&file)
}

/// Validates the opened object; a deleted journal has zero links and cannot alias an outside file.
#[cfg(windows)]
fn check_windows_handle(file: &fs::File) -> Result<(), ()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let metadata = file.metadata().map_err(|_| ())?;
    if !metadata.is_file() || linked(&metadata) || metadata.len() > MAX_DATABASE_BYTES {
        return Err(());
    }
    let mut facts = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: a live read-only handle and correctly sized writable structure outlive this query.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut facts) } == 0
        || facts.nNumberOfLinks > 1
    {
        return Err(());
    }
    Ok(())
}

/// Refuses a database hardlink without changing the file outside the owned cache.
#[cfg(all(test, windows))]
#[test]
fn rejects_linked_database_without_writing_source() {
    let root = std::env::temp_dir().join(format!("resolibre-cache-links-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let cache = root.join("cache");
    fs::create_dir(&cache).unwrap();
    let source = root.join("original");
    fs::write(&source, b"preserve original").unwrap();
    for name in [
        "analysis.sqlite3",
        "analysis.sqlite3-journal",
        "analysis.sqlite3-wal",
        "analysis.sqlite3-shm",
    ] {
        fs::hard_link(&source, cache.join(name)).unwrap();
        assert!(database_path(&cache).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"preserve original");
        fs::remove_file(cache.join(name)).unwrap();
    }
    fs::remove_file(source).unwrap();
    fs::remove_dir(cache).unwrap();
    fs::remove_dir(root).unwrap();
}

/// SQLite may unlink a journal during validation; its retained zero-link handle remains harmless.
#[cfg(all(test, windows))]
#[test]
fn accepts_deleted_journal_handle_without_accepting_hardlinks() {
    let root = std::env::temp_dir().join(format!("resolibre-cache-journal-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let journal = root.join("analysis.sqlite3-journal");
    fs::write(&journal, b"owned journal").unwrap();
    let retained = fs::File::open(&journal).unwrap();

    fs::remove_file(&journal).unwrap();
    assert!(check_windows_handle(&retained).is_ok());
    drop(retained);
    assert!(check_file(&journal).is_ok());

    fs::write(&journal, b"replacement journal").unwrap();
    fs::hard_link(&journal, root.join("outside")).unwrap();
    assert!(check_file(&journal).is_err());
    fs::remove_file(root.join("outside")).unwrap();
    fs::remove_file(journal).unwrap();
    fs::remove_dir(root).unwrap();
}

/// Concurrent first use accepts only real cache directories, preserving private Unix permissions.
#[cfg(test)]
#[test]
fn concurrent_directory_creation_revalidates_the_winning_entry() {
    let root = std::env::temp_dir().join(format!("resolibre-cache-create-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..8 {
            workers.push(scope.spawn(|| {
                let mut accepted = true;
                for index in 0..32 {
                    let directory = root.join(format!("cache-{index}"));
                    barrier.wait();
                    accepted &= ensure_directory(&directory).is_ok();
                    barrier.wait();
                }
                accepted
            }));
        }
        for worker in workers {
            assert!(
                worker.join().unwrap(),
                "a concurrent cache directory was rejected"
            );
        }
    });
    let invalid = root.join("regular-file");
    fs::write(&invalid, b"not a directory").unwrap();
    assert!(ensure_directory(&invalid).is_err());
    fs::remove_file(invalid).unwrap();
    for index in 0..32 {
        fs::remove_dir(root.join(format!("cache-{index}"))).unwrap();
    }
    fs::remove_dir(root).unwrap();
}
