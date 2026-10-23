//! Shared read-only, descriptor-scoped decoder for bounded original-audio consumers.

pub(crate) mod native;
pub(crate) mod pcm;

use crate::platform::PipeReader;
use std::fs::File;
#[cfg(unix)]
use std::fs::Metadata;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const UNAVAILABLE: &str = "Original audio analysis unavailable";

/// File identity includes change time and selected stream; no paths or stale cross-file results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceKey {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    pub(super) stream: u32,
}

impl SourceKey {
    /// Uses retained Windows volume/file identity and write/change timestamps for cache invalidation.
    #[cfg(windows)]
    fn from_windows_file(file: &File, stream: u32) -> Result<Self, &'static str> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandle,
            GetFileInformationByHandleEx,
        };
        let mut identity = BY_HANDLE_FILE_INFORMATION::default();
        let mut facts = FILE_BASIC_INFO::default();
        // SAFETY: both Win32 APIs receive a live retained handle and correctly sized output structures.
        let success = unsafe {
            GetFileInformationByHandle(file.as_raw_handle(), &mut identity) != 0
                && GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    FileBasicInfo,
                    (&mut facts as *mut FILE_BASIC_INFO).cast(),
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                ) != 0
        };
        if !success {
            return Err(UNAVAILABLE);
        }
        Ok(Self {
            device: u64::from(identity.dwVolumeSerialNumber),
            inode: (u64::from(identity.nFileIndexHigh) << 32) | u64::from(identity.nFileIndexLow),
            size: (u64::from(identity.nFileSizeHigh) << 32) | u64::from(identity.nFileSizeLow),
            modified: (facts.LastWriteTime, 0),
            // Negative nanoseconds cannot occur in Unix metadata keys: distinguish OS encodings.
            changed: (facts.ChangeTime, -1),
            stream,
        })
    }
    /// Snapshots the already-open regular file used by playback, rather than resolving its path again.
    pub(crate) fn read(file: &File, stream: u32) -> Result<Self, &'static str> {
        let metadata = file.metadata().map_err(|_| UNAVAILABLE)?;
        if !metadata.is_file() {
            return Err(UNAVAILABLE);
        }
        #[cfg(unix)]
        {
            Ok(Self::from_metadata(&metadata, stream))
        }
        #[cfg(windows)]
        {
            Self::from_windows_file(file, stream)
        }
    }

    /// Serializes local file facts for metadata reuse, never as proof of audio content identity.
    pub(crate) fn metadata_key(self) -> Vec<u8> {
        let fields = [
            self.device,
            self.inode,
            self.size,
            self.modified.0 as u64,
            self.modified.1 as u64,
            self.changed.0 as u64,
            self.changed.1 as u64,
        ];
        let mut bytes = Vec::with_capacity(56);
        for field in fields {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        bytes
    }

    /// Makes cache invalidation explicit for rewrites, replacements and alternate audio streams.
    #[cfg(unix)]
    fn from_metadata(metadata: &Metadata, stream: u32) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            stream,
        }
    }
}

struct Decoder(Child);
impl Drop for Decoder {
    /// No subprocess survives a cancelled, malformed or timed-out analysis.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Decodes the selected original stream as floating-point WAV, preserving every channel and rate.
pub(crate) fn decode<T>(
    file: File,
    key: SourceKey,
    cancel: &AtomicBool,
    deadline: Duration,
    consume: impl FnOnce(&mut PipeReader, Instant) -> Result<T, ()>,
) -> Result<T, &'static str> {
    let started = Instant::now();
    let (mut reader, writer) = crate::platform::output_pipe().map_err(|_| UNAVAILABLE)?;
    reader
        .set_read_timeout(Some(Duration::from_millis(20)))
        .map_err(|_| UNAVAILABLE)?;
    let input = crate::platform::independent_file(&file).map_err(|_| UNAVAILABLE)?;
    let mut child = Decoder(
        crate::platform::helper("ffmpeg")
            .env_remove("FFREPORT")
            .args([
                "-hide_banner",
                "-loglevel",
                "quiet",
                "-nostdin",
                "-max_alloc",
                "33554432",
                "-threads",
                "1",
                "-filter_threads",
                "1",
                "-probesize",
                "1048576",
                "-analyzeduration",
                "2000000",
                "-protocol_whitelist",
                "fd",
                "-format_whitelist",
                crate::media_formats::DEMUXERS,
                "-fd",
                "0",
                "-i",
                "fd:",
                "-map",
                &format!("0:{}", key.stream),
                "-vn",
                "-sn",
                "-dn",
                "-map_metadata",
                "-1",
                "-c:a",
                "pcm_f32le",
                "-f",
                "wav",
                "pipe:1",
            ])
            .stdin(Stdio::from(input))
            .stdout(writer)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| UNAVAILABLE)?,
    );
    let analysis = consume(&mut reader, started).map_err(|_| UNAVAILABLE)?;
    loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() >= deadline {
            return Err(UNAVAILABLE);
        }
        if let Some(status) = child.0.try_wait().map_err(|_| UNAVAILABLE)? {
            if !status.success() || SourceKey::read(&file, key.stream)? != key {
                return Err(UNAVAILABLE);
            }
            return Ok(analysis);
        }
        thread::sleep(Duration::from_millis(2));
    }
}
