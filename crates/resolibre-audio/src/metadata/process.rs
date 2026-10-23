//! Bounded helper processes: descriptor-only input, no network, no native diagnostics or output files.

use std::fs::File;
use std::io::{ErrorKind, Read, Seek};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(3);

struct Helper(Child);

impl Drop for Helper {
    /// Reaps the child on success, cancellation, timeout and output overflow alike.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Requests bounded tags, durations, audio identity and attached-picture dimensions.
pub(super) fn probe(file: &File, cancel: &AtomicBool, started: Instant) -> Result<Vec<u8>, ()> {
    let mut command = command("ffprobe");
    command.args(["-show_entries", "format=duration:format_tags=artist,title,album,album_artist,date,year,disc,discnumber:stream=index,codec_type,width,height,duration:stream_tags=artist,title,album,album_artist,date,year,disc,discnumber:stream_disposition=attached_pic", "-of", "json", "fd:"]);
    run(command, file, 65536, cancel, started)
}

/// Decodes only the selected embedded image, scaling/cropping to the UI's square cover-fill shape.
pub(super) fn cover(
    file: &File,
    stream: u32,
    cancel: &AtomicBool,
    started: Instant,
) -> Result<Vec<u8>, ()> {
    let mut command = command("ffmpeg");
    command.args([
        "-nostdin",
        "-filter_threads",
        "1",
        "-i",
        "fd:",
        "-map",
        &format!("0:{stream}"),
        "-an",
        "-sn",
        "-dn",
        "-frames:v",
        "1",
        "-vf",
        "scale=512:512:force_original_aspect_ratio=increase,crop=512:512",
        "-threads",
        "1",
        "-pix_fmt",
        "rgba",
        "-f",
        "rawvideo",
        "pipe:1",
    ]);
    run(command, file, super::COVER_BYTES, cancel, started)
}

/// Shares safe probing limits and protocol/demuxer restrictions across both executables.
fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.env_remove("FFREPORT").args([
        "-hide_banner",
        "-loglevel",
        "quiet",
        "-max_alloc",
        "33554432",
        "-threads",
        "1",
        "-probesize",
        "1048576",
        "-analyzeduration",
        "1000000",
        "-protocol_whitelist",
        "fd",
        "-format_whitelist",
        crate::media_formats::DEMUXERS,
        "-fd",
        "0",
    ]);
    command
}

/// Drains a capped socket with short cancellation checkpoints; never uses wait_with_output.
fn run(
    mut command: Command,
    file: &File,
    limit: usize,
    cancel: &AtomicBool,
    started: Instant,
) -> Result<Vec<u8>, ()> {
    if cancel.load(Ordering::Relaxed) || started.elapsed() >= DEADLINE {
        return Err(());
    }
    let (mut reader, writer) = UnixStream::pair().map_err(|_| ())?;
    reader
        .set_read_timeout(Some(Duration::from_millis(20)))
        .map_err(|_| ())?;
    let mut input = file.try_clone().map_err(|_| ())?;
    input.rewind().map_err(|_| ())?;
    let mut helper = Helper(
        command
            .stdin(Stdio::from(input))
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ())?,
    );
    // Command retains configured descriptors after spawn; EOF requires dropping its socket copy.
    drop(command);
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() >= DEADLINE {
            return Err(());
        }
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                if bytes.len() + length > limit {
                    return Err(());
                }
                bytes.extend_from_slice(&buffer[..length]);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => return Err(()),
        }
    }
    loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() >= DEADLINE {
            return Err(());
        }
        if let Some(status) = helper.0.try_wait().map_err(|_| ())? {
            return if status.success() { Ok(bytes) } else { Err(()) };
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
