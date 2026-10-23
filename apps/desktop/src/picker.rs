//! A disposable native dialog process keeps picker waits out of the UI/audio threads.
//! Raw path bytes travel over a private inherited socket, never logs or shell text.

use crate::path_encoding;
use resolibre_audio::platform::PipeReader;
use resolibre_audio::session::validate_selection;
use resolibre_core::queue::MAX_QUEUE_ITEMS;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MAX_WIRE_BYTES: usize = path_encoding::MAX_SELECTION_BYTES + 4 * (MAX_QUEUE_ITEMS + 1);

#[derive(Clone, Copy)]
pub enum Kind {
    Music,
    ImportPreset,
    ExportPreset,
}

impl Kind {
    /// Maps fixed helper switches to dialog purposes; paths never enter process arguments.
    pub fn argument(self) -> &'static str {
        match self {
            Self::Music => "--file-picker",
            Self::ImportPreset => "--import-eq-picker",
            Self::ExportPreset => "--export-eq-picker",
        }
    }

    /// Recognizes only supported helper entry points.
    pub fn from_argument(argument: &std::ffi::OsStr) -> Option<Self> {
        [Self::Music, Self::ImportPreset, Self::ExportPreset]
            .into_iter()
            .find(|kind| argument == kind.argument())
    }
}

#[cfg(unix)]
unsafe extern "C" {
    /// Signals the owned picker process group; negative pid addresses that group.
    fn kill(pid: i32, signal: i32) -> i32;
}

/// Owns one native chooser and its bounded result buffer; no detached picker thread.
pub struct Picker {
    child: Child,
    socket: PipeReader,
    bytes: Vec<u8>,
    started: Instant,
    pub append: bool,
    reaped: bool,
}

impl Picker {
    /// Launches this executable's picker helper with no private path arguments.
    pub fn start(append: bool) -> Result<Self, &'static str> {
        Self::start_for(Kind::Music, append)
    }

    /// Reuses the owned, cancellable helper for music and single-preset transfers.
    pub fn start_for(kind: Kind, append: bool) -> Result<Self, &'static str> {
        let (mut socket, output) = resolibre_audio::platform::output_pipe()
            .map_err(|_| "Cannot create file chooser channel")?;
        socket
            .set_nonblocking(true)
            .map_err(|_| "Cannot configure file chooser channel")?;
        let executable = helper_executable()?;
        let mut command = Command::new(executable);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let child = command
            .arg(kind.argument())
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "Cannot start file chooser")?;
        Ok(Self {
            child,
            socket,
            bytes: Vec::new(),
            started: Instant::now(),
            append,
            reaped: false,
        })
    }

    /// Reads a small chunk without blocking; returns cancellation as an empty selection.
    pub fn poll(&mut self) -> Result<Option<Vec<PathBuf>>, &'static str> {
        if self.started.elapsed() > Duration::from_secs(600) {
            return Err("File chooser timed out; open it again");
        }
        let mut buffer = [0_u8; 16 * 1024];
        match self.socket.read(&mut buffer) {
            Ok(length) => {
                if self.bytes.len() + length > MAX_WIRE_BYTES {
                    return Err("File selection is too large");
                }
                self.bytes.extend_from_slice(&buffer[..length]);
                if length > 0 {
                    return Ok(None);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(_) => return Err("File chooser channel failed"),
        }
        let Some(status) = self
            .child
            .try_wait()
            .map_err(|_| "Cannot check file chooser")?
        else {
            return Ok(None);
        };
        self.reaped = true;
        if !status.success() {
            return Err("File chooser failed; check desktop portal availability");
        }
        decode(&self.bytes).map(Some)
    }
}

impl Drop for Picker {
    /// Cancels/reaps the owned helper and any fallback chooser in its process group.
    fn drop(&mut self) {
        if !self.reaped {
            #[cfg(unix)]
            if let Ok(pid) = i32::try_from(self.child.id()) {
                // SAFETY: this is our unreaped child's group established at spawn.
                // It cannot be reused while the direct child remains unreaped.
                unsafe {
                    kill(-pid, 9);
                }
            }
            #[cfg(windows)]
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Selects the running image, even if a rebuild replaced its on-disk pathname.
#[cfg(target_os = "linux")]
fn helper_executable() -> Result<PathBuf, &'static str> {
    // Do not resolve this kernel link: current_exe() returns a deleted pathname after updates.
    // Executing the link also keeps parent/helper versions aligned instead of running a replacement.
    Ok(PathBuf::from("/proc/self/exe"))
}

/// Locates the current executable on other Unix platforms.
#[cfg(not(target_os = "linux"))]
fn helper_executable() -> Result<PathBuf, &'static str> {
    std::env::current_exe().map_err(|_| "Cannot locate file chooser helper")
}

/// Runs only in the helper process; picker cancellation writes a terminal zero frame.
pub fn run_helper(kind: Kind) -> Result<(), &'static str> {
    let files = match kind {
        Kind::Music => rfd::FileDialog::new()
            .set_title("Open local music")
            .add_filter(
                "Audio / video (sound only)",
                resolibre_audio::media_formats::EXTENSIONS,
            )
            .pick_files()
            .unwrap_or_default(),
        Kind::ImportPreset => rfd::FileDialog::new()
            .set_title("Import ReSolibre EQ preset")
            .add_filter("ReSolibre EQ preset", &["resolibre-eq"])
            .pick_file()
            .into_iter()
            .collect(),
        Kind::ExportPreset => rfd::FileDialog::new()
            .set_title("Export EQ preset to a new file")
            .add_filter("ReSolibre EQ preset", &["resolibre-eq"])
            .set_file_name("preset.resolibre-eq")
            .save_file()
            .into_iter()
            .collect(),
    };
    if !files.is_empty() {
        validate_selection(&files)?;
    }
    let mut output = std::io::stdout().lock();
    for path in files {
        let bytes = path_encoding::encode(path.as_os_str());
        output
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .map_err(|_| "Picker output failed")?;
        output
            .write_all(&bytes)
            .map_err(|_| "Picker output failed")?;
    }
    output
        .write_all(&0_u32.to_le_bytes())
        .map_err(|_| "Picker output failed")
}

/// Validates complete framing before constructing a batch, preserving non-UTF-8 names.
fn decode(bytes: &[u8]) -> Result<Vec<PathBuf>, &'static str> {
    let mut rest = bytes;
    let mut paths = Vec::new();
    loop {
        if rest.len() < 4 {
            return Err("Incomplete file selection");
        }
        let length =
            u32::from_le_bytes(rest[..4].try_into().map_err(|_| "Invalid frame")?) as usize;
        rest = &rest[4..];
        if length == 0 {
            if !rest.is_empty() {
                return Err("Unexpected chooser data");
            }
            break;
        }
        if length > path_encoding::MAX_PATH_BYTES
            || rest.len() < length
            || paths.len() >= MAX_QUEUE_ITEMS
        {
            return Err("Invalid file selection");
        }
        let path = path_encoding::decode(&rest[..length]).ok_or("Invalid local file path")?;
        paths.push(PathBuf::from(path));
        rest = &rest[length..];
    }
    if !paths.is_empty() {
        validate_selection(&paths)?;
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Truncation, extra data and oversized frames cannot turn into unintended paths.
    #[test]
    fn validates_picker_frames() {
        assert!(decode(&[0, 0, 0, 0]).unwrap().is_empty());
        for bytes in [
            &[][..],
            &[1, 0, 0, 0],
            &[0, 0, 0, 0, 1],
            &[255, 255, 255, 255],
        ] {
            assert!(decode(bytes).is_err());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bytes = [2, 0, 0, 0, b'a', 0xff, 0, 0, 0, 0];
            assert_eq!(
                decode(&bytes).unwrap()[0].as_os_str().as_bytes(),
                &[b'a', 0xff]
            );
        }
    }

    /// Re-executes a running test image after unlink/replacement, without opening a real dialog.
    #[cfg(target_os = "linux")]
    #[test]
    fn helper_survives_executable_replacement() {
        const CHILD_STAGE: &str = "RESOUL_TEST_PICKER_REEXEC";
        if std::env::var_os(CHILD_STAGE).is_some() {
            // The parent signals only after replacing our executable on disk.
            let mut ready = [0_u8];
            std::io::stdin().read_exact(&mut ready).unwrap();
            assert!(!std::env::current_exe().unwrap().exists());
            let mut child = Command::new(helper_executable().unwrap())
                .args(["--exact", "picker::tests::validates_picker_frames"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("running image remains executable after replacement");
            assert!(wait_for_test_child(&mut child).success());
            return;
        }

        use std::os::unix::fs::DirBuilderExt;
        let directory =
            std::env::temp_dir().join(format!("resolibre-picker-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let executable = directory.join("test-image");
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let mut child = Command::new(&executable)
            .args([
                "--exact",
                "picker::tests::helper_survives_executable_replacement",
            ])
            .env(CHILD_STAGE, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::fs::remove_file(&executable).unwrap();
        // Never execute a newer or unrelated file placed at the old pathname.
        std::fs::write(&executable, b"not the running application").unwrap();
        child.stdin.take().unwrap().write_all(&[1]).unwrap();
        let status = wait_for_test_child(&mut child);
        std::fs::remove_dir_all(directory).unwrap();
        assert!(
            status.success(),
            "helper must use its running image, not the removed path/replacement"
        );
    }

    /// Bounds subprocess regression tests and always reaps an overdue child.
    #[cfg(target_os = "linux")]
    fn wait_for_test_child(child: &mut Child) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("helper regression deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
