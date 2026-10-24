# Native Windows prototype

## Read When

Building, testing or changing the Windows x86_64 native adapter and desktop.
Use the `windows-native` reading profile; root, core and desktop rules apply.

## Do Not Read For

Linux setup, installers, distribution approval, or a claim of Windows feature parity.
The [delivery target](../architecture/target/delivery-and-licensing.md) owns packaging.

## Current Behavior

The native MSVC build produces `resolibre-desktop.exe`. It uses the shared Slint
UI, session, queue, EQ, metadata and original-audio analysis. No WSL is involved.
Live Spectrum remains unavailable on Windows: its sealed Linux LADSPA loader
has not been ported. The UI reports this without stopping playback; file analysis
and its spectrogram are separate and remain available.

Prerequisites: Python 3.9+, x64 MSVC Rust/Cargo, Visual C++ Build Tools with the
C++ desktop workload and Windows SDK. Provide x64 `libmpv-2.dll`, `ffmpeg.exe`
and `ffprobe.exe` in `.local/native/windows`, or set `RESOLIBRE_NATIVE_DIR` to
their directory. The build checks PE architecture and stages these three files
beside the executable. It does not download/install dependencies or modify PATH
globally. Use trusted builds; PE inspection is not signature verification.

```powershell
.\build_windows.ps1
.\build_windows.ps1 -Profile Release -Jobs 4
.\build_windows.ps1 -Profile Release -Run
```

If local script policy blocks execution, invoke a single process without changing
machine policy: `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\build_windows.ps1`.
`-Offline` uses cached crates. `-Python C:\path\python.exe` selects an installed
interpreter explicitly; otherwise the wrapper uses `py -3`, then `python` if
the launcher is absent. The existing Python debug/release entry points also work.
Default output: `target/x86_64-pc-windows-msvc/{debug,release}/resolibre-desktop.exe`.
Custom Cargo output directories and invocation from another directory are supported.

Windows playback registers a read-only libmpv stream over a retained file handle.
Each stream gets an independent cursor through `ReOpenFile`; writes/deletion of
an active source are denied. No original path enters libmpv commands. Device
names, device namespaces and alternate data streams are rejected. Bounded
helper pipes carry metadata; randomized local-only named pipes carry original
PCM. Reads use overlapped I/O and a completion event: a 20 ms timeout bounds
cancellation checks without sleeping after every temporarily empty chunk.
Each reader owns a fixed 64 KiB buffer and one pending operation. Shutdown
cancels and reaps that operation before releasing its storage or native engine.
Helpers are resolved beside the executable, not relative to music or the working directory.

Profiles use `%LOCALAPPDATA%\ReSolibre` and inherit the current user's profile ACL.
Settings use flushed temporary files and write-through atomic replacement.
Session files carry a Windows-specific header and length-prefixed UTF-16 paths;
foreign Unix session files fail closed. Native selection limits remain 1024
entries and 1 MiB; UTF-16 wire/storage permits up to twice the encoded path bytes.
Reset runs only after workers stop and exclusive profile access succeeds. It
pins ancestors against replacement and deletes validated allowlisted files by
handle; reparse points, hardlinks and unknown entries are refused.
Win32 placement uses shared snap geometry; owned auxiliary windows are grouped
with the player without enumerating unrelated windows.

## Key Files

- [PowerShell entry](../../build_windows.ps1), [shared build](../../tools/build_support.py).
- `crates/resolibre-audio/src/platform/`: retained input streams and helper pipes.
- `crates/resolibre-audio/src/platform/windows/pipe.rs`: event-driven pipe reads and cancellation.
- `crates/resolibre-audio/src/original_audio/native/pipe.rs`: bounded PCM channel.
- `apps/desktop/src/{path_encoding,profile_files,profile_storage}.rs`: native profile I/O.
- `apps/desktop/src/profile_reset/windows.rs`, `window_snap/windows.rs`, `window_group/windows.rs`.

## Tests

Reference: Windows x86_64, Rust 1.99.1 MSVC, Visual Studio Build Tools 17.14.42,
Python 3.13.17, checked 2026-10-24. Both PowerShell build profiles produced PE
executables. Owned native tests cover silent playback/seek, Unicode paths,
source preservation, metadata, multi-buffer PCM analysis, prefix cancellation
and device-path rejection. Desktop tests cover profile replacement, native path
round-trips, competing leases, reset refusal and production restart/close flows.
The complete workspace suite passed on this host (environment-dependent
tests skipped, Windows device checks run explicitly). Native boundary checks cover maximum path lengths
and database hardlink refusal. Clippy passed with warnings denied; Python build
checks passed (9 tests, 1 Linux-only fixture skipped), as did documentation checks.

A release-mode regression found that the original Windows peek/sleep reader
throttled fast consumers: a 180-second stereo WAV took 6.32 s through the helper
and 4.00 s through libmpv. Event-driven reads reduced those checks to 0.067 s and
0.051 s. A 356-second, 192 kHz stereo FLAC previously exhausted TRIM's 15-second
deadline; with a fresh profile it now starts with confirmed TRIM in about 1 s.
MIX also completed the overlap and adopted the second stream on the Windows
device with application volume zero. These are local observations, not universal
latency or listening guarantees. Generated tests cover staggered writes, short
reads, EOF, duplicate clients, pending connect/read cancellation and compressed
stereo preparation. Run native performance regressions in Release as well as
Debug: slower debug consumers masked the fixed-sleep bottleneck.
One workspace run encountered an intermittent cache-reuse test failure while a
build was also running. The isolated test and subsequent complete suite passed;
the cache failure's cause was not established in this pipe fix.

For Cargo tests, stage the runtime beside test executables or put its directory
on this shell's PATH; helper binaries must be beside the audio test executable:

```powershell
$env:PATH = "$PWD\.local\native\windows;$env:PATH"
cargo test --workspace --all-targets --locked --offline --no-run
Copy-Item .local/native/windows/*.exe,.local/native/windows/*.dll target/debug/deps/
cargo test --workspace --all-targets --locked --offline -- --test-threads=1
py -3 -m unittest discover -s tools -p test_build_support.py -v
py -3 tools/check_windows_startup.py target/x86_64-pc-windows-msvc/debug/resolibre-desktop.exe
```

For the pipe regression, repeat `platform::windows_tests` with `--release
--target x86_64-pc-windows-msvc`; stage the same runtime files in
`target/x86_64-pc-windows-msvc/release/deps/`. In the `session` integration target,
run the `windows_` tests with `--ignored --test-threads=1` on a Windows audio
device. They generate their own media and keep the application volume at zero.

Repeated live-scrub session checks cover muted WAV, 192 kHz stereo FLAC and MP3
in Debug/Release. The desktop [scrub regression](../../apps/desktop/src/scrub_tests.rs)
drives 450 native pointer moves with MIX, checking confirmed time and rendered frames.
Run `native_repeated_scrub_keeps_clock_and_renderer_running --ignored`; Release
builds need `SLINT_EMIT_DEBUG_INFO=1` for control lookup. The reported intermittent
visual freeze has not been reproduced by these checks.

The startup check owns a temporary profile, observes only its child's main
window and sends WM_CLOSE. It never imports user music. Linux/Pulse/Wayland
harnesses remain platform-scoped; device listening, device loss, Windows DPI,
native dialog interaction and extended playback quality need separate evidence.

## Edge Cases

Missing/wrong-architecture runtime files fail before Cargo. Compiler errors
propagate and never launch stale output. Profile reset may fail inside restricted
sandboxes that cannot pin ancestors; it must not weaken the deletion checks.
Closed Linux-only tests are not Windows validation. Linux builds were not rerun
on this Windows reference; their current evidence remains the earlier Linux run.

Local runtime provenance: [mpv's Windows build list](https://mpv.io/installation/)
links the third-party shinchiro builds; this check used
`mpv-dev-x86_64-20261021-git-e471496c56.7z`, SHA-256
`efc719eff3dc5954b4a5efc800632808bcec01cc9034940dd14e5eb7fe9ea633`.
The [Gyan FFmpeg build](https://www.gyan.dev/ffmpeg/builds/) supplies the paired
FFmpeg/ffprobe 9.0.2 executables; its published archive checksum was verified.
Native dependency and redistribution audits remain release work. Runtime files
are ignored local inputs, not committed distribution artifacts.

## Related Docs

[Local builds](local-builds.md), [playback](../architecture/features/local-playback.md),
[session persistence](../architecture/features/session-persistence.md),
[test plan](../testing/test-plan.md).

## Lossy-source detector

Full analysis embeds the pinned `lossprint` model and CPU-only Rust `tract` runtime;
no extra ONNX DLL or runtime download is needed. Native build scripts include
`licenses/lossprint-MIT.txt` and `licenses/tract-MIT.txt` beside the executable.
See [sampling, resource bounds and test probes](../architecture/features/lossy-source-detection.md).

Detector: GNU cross-build and Wine model/native-decoder probes passed. The new
feature remains untested on the real MSVC host above.
