//! Event-driven, bounded pipe reads for Windows analysis and helper processes.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_INBOUND, ReadFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS,
};
use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

const BUFFER_BYTES: usize = 65536;

#[derive(Clone, Copy)]
enum Pending {
    Connection,
    Read,
}

pub struct PipeReader {
    file: File,
    event: OwnedHandle,
    // Windows retains these addresses until completion, even if the Rust reader moves.
    operation: Box<OVERLAPPED>,
    buffer: Box<[u8; BUFFER_BYTES]>,
    pending: Option<Pending>,
    buffered: std::ops::Range<usize>,
    timeout: Duration,
    ended: bool,
}

impl PipeReader {
    /// Begins an asynchronous connection; the caller can now open the sole write endpoint.
    fn listen(file: File) -> io::Result<Self> {
        // SAFETY: null attributes/name create a private, non-inheritable manual-reset event.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: ownership of the newly created event is transferred exactly once.
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        let mut reader = Self {
            file,
            operation: Box::new(OVERLAPPED {
                hEvent: event.as_raw_handle(),
                ..Default::default()
            }),
            event,
            buffer: Box::new([0; BUFFER_BYTES]),
            pending: None,
            buffered: 0..0,
            timeout: Duration::from_millis(20),
            ended: false,
        };
        // SAFETY: the overlapped pipe and boxed operation remain live until completion or Drop.
        let connected =
            unsafe { ConnectNamedPipe(reader.file.as_raw_handle(), &mut *reader.operation) };
        if connected == 0 {
            let error = io::Error::last_os_error();
            match error.raw_os_error().map(|code| code as u32) {
                Some(ERROR_IO_PENDING) => reader.pending = Some(Pending::Connection),
                Some(ERROR_PIPE_CONNECTED) => {}
                _ => return Err(error),
            }
        }
        Ok(reader)
    }

    /// Sets the maximum wait for data, rather than a fixed sleep before looking again.
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout.unwrap_or(Duration::ZERO);
        Ok(())
    }

    /// Polls an already-issued read without blocking the UI; pending storage remains owned.
    pub fn set_nonblocking(&mut self, enabled: bool) -> io::Result<()> {
        if !enabled {
            return Err(io::Error::other("Blocking helper reads are unsupported"));
        }
        self.timeout = Duration::ZERO;
        Ok(())
    }

    /// Issues at most one read into stable private storage, never into the caller's borrowed buffer.
    fn start_read(&mut self) -> io::Result<()> {
        // SAFETY: no operation is pending, so the event and OVERLAPPED can be reused.
        if unsafe { ResetEvent(self.event.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        *self.operation = OVERLAPPED {
            hEvent: self.event.as_raw_handle(),
            ..Default::default()
        };
        // SAFETY: the boxed buffer and operation survive asynchronous completion and cancellation.
        let completed = unsafe {
            ReadFile(
                self.file.as_raw_handle(),
                self.buffer.as_mut_ptr(),
                BUFFER_BYTES as u32,
                std::ptr::null_mut(),
                &mut *self.operation,
            )
        };
        if completed == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return Err(error);
            }
        }
        self.pending = Some(Pending::Read);
        Ok(())
    }

    /// Wakes immediately on completion and retains pending storage across a bounded timeout.
    fn complete(&mut self) -> io::Result<usize> {
        let milliseconds = self.timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32;
        // SAFETY: the event belongs to the sole outstanding operation on this reader.
        let status = unsafe { WaitForSingleObject(self.event.as_raw_handle(), milliseconds) };
        if status == WAIT_TIMEOUT {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        if status != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut transferred = 0;
        // SAFETY: a signaled manual-reset event confirms completion; output is live aligned storage.
        let success = unsafe {
            GetOverlappedResult(
                self.file.as_raw_handle(),
                &*self.operation,
                &mut transferred,
                0,
            )
        };
        self.pending = None;
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(transferred as usize)
    }

    /// Copies only completed bytes, preserving any remainder for smaller caller buffers.
    fn copy_buffered(&mut self, destination: &mut [u8]) -> usize {
        let count = destination.len().min(self.buffered.len());
        let end = self.buffered.start + count;
        destination[..count].copy_from_slice(&self.buffer[self.buffered.start..end]);
        self.buffered.start = end;
        count
    }

    /// Completes a connection and one read, preserving cancellation checkpoints on an idle writer.
    fn read_pending(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        loop {
            if let Some(pending) = self.pending {
                let count = self.complete()?;
                if matches!(pending, Pending::Read) {
                    self.buffered = 0..count;
                    return Ok(self.copy_buffered(destination));
                }
            }
            self.start_read()?;
        }
    }
}

impl Read for PipeReader {
    /// Returns buffered data, waits for fresh data, or observes EOF without delaying ready chunks.
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() || self.ended {
            return Ok(0);
        }
        if !self.buffered.is_empty() {
            return Ok(self.copy_buffered(bytes));
        }
        match self.read_pending(bytes) {
            Err(error) if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => {
                self.ended = true;
                Ok(0)
            }
            result => result,
        }
    }
}

impl Drop for PipeReader {
    /// Cancels and reaps pending kernel I/O before freeing either retained address or closing handles.
    fn drop(&mut self) {
        if self.pending.is_some() {
            let mut transferred = 0;
            // SAFETY: all handles/storage are still live. Cancellation alone is insufficient:
            // GetOverlappedResult waits for completion before Rust can release the borrowed storage.
            unsafe {
                CancelIoEx(self.file.as_raw_handle(), &*self.operation);
                GetOverlappedResult(
                    self.file.as_raw_handle(),
                    &*self.operation,
                    &mut transferred,
                    1,
                );
            }
        }
    }
}

/// Creates one unguessable, local-only endpoint; collisions cannot attach to another server.
pub(crate) fn named_pipe() -> io::Result<(PipeReader, String)> {
    let mut random = [0_u8; 16];
    // SAFETY: the system RNG writes exactly the live buffer and retains no pointer.
    if unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            random.as_mut_ptr(),
            16,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
    {
        return Err(io::Error::other("Cannot allocate private analysis channel"));
    }
    let token = u128::from_le_bytes(random);
    let path = format!(r"\\.\pipe\resolibre-pcm-{token:032x}");
    let name: Vec<_> = path.encode_utf16().chain(Some(0)).collect();
    // SAFETY: terminated name, private non-inheritable handle, one bounded inbound byte stream.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_REJECT_REMOTE_CLIENTS,
            1,
            BUFFER_BYTES as u32,
            BUFFER_BYTES as u32,
            0,
            std::ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the newly created server handle is transferred to one owner.
    let file = unsafe { File::from_raw_handle(handle) };
    Ok((PipeReader::listen(file)?, path))
}

/// Connects a synchronous child stdout writer to the asynchronous parent reader.
pub(crate) fn output_pipe() -> io::Result<(PipeReader, std::process::Stdio)> {
    let (reader, path) = named_pipe()?;
    let writer = OpenOptions::new().write(true).open(path)?;
    Ok((reader, std::process::Stdio::from(writer)))
}
