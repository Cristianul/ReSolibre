# Local playback prototype

## Read When

Rust/libmpv work; read the [core map](../core-project-map.md) and rules.

## Do Not Read For

UI, catalog, SMB, EQ or packaging.

## Current Behavior

`local-play` reads one regular file to EOF on default output, reporting
lifecycle/formats without names/tags. Failures exit nonzero with safe codes.
Ctrl-C exits; no interactive menu. The API supports pause/resume, absolute
exact seek and engine position (not evidence of heard sound).

`audio_formats()` returns optional typed decoder/audio API rate, channel count
and PCM representation, including planar storage. These do not establish file
codec/bit depth, speaker placement, OS mixing or DAC format. Unknown native
representations become `Unknown`; native strings never escape. Numeric values
must be positive `u32`; failed queries retain safe native codes.

Readings are non-atomic and can disappear during loading/changes/draining.
EOF/errors clear formats; the CLI warns only for never-observed formats.
Normal playback does not force rate/layout or establish bit-perfect output.

Each player owns one engine/track. Drop stops native work before closing the
read-only descriptor. Calls may block. `poll_event` waits at most 100 ms; `try_event`
does not wait. The [desktop session](local-desktop-player.md) owns its worker/queue
and creates a fresh player per track; it adds typed timing and software gain.
Seek/EQ rejections during confirmed native teardown yield to the pending terminal
event; real errors remain errors. No EOF is inferred from a seek rejection.

Linux x86_64 passes a retained read-only `/proc/self/fd/<descriptor>`.
Nonblocking open rejects non-regular files, including FIFOs. Paths never enter
FFI; non-UTF-8 names and symlinks work. Concurrent inode mutation is possible.

Windows x64 uses read-only libmpv stream callbacks and independent retained-handle
cursors; active files deny write/delete sharing. See [native Windows](../../development/windows-native.md)
for path validation, runtime setup and test limits.

Configuration/scripts, URL helpers, video/subtitles, sidecar discovery, resume
files and references are disabled. `media_formats.rs` owns audio/video candidates
and the lavf allowlist. Video containers play audio only; codecs remain native.
Nested protocols are disabled and disguised playlists fail. Read-ahead is
8 MiB with no backward/disk cache; probing uses 1 MiB and two seconds of analysis.
These configuration bounds do not guarantee total memory or wall-clock time.
Native parsers run in-process.

## Key Files

The [core map](../core-project-map.md) routes code.
Use the [native guide](../../development/native-environment.md) for commands.
Evidence: [listening](../../testing/local-audio-validation.md),
[adaptation](../../testing/output-adaptation-validation.md),
[PCM conversion](../../testing/pcm-conversion-validation.md),
[output failure](../../testing/output-failure-validation.md) and
[responsiveness](../../testing/playback-responsiveness-validation.md).

## Tests

Tests cover formats, EOF, controls, isolation, preservation, output adaptation
and MP4/MKV/WebM/M4A audio-only playback; video-only inputs fail. Ignored PCM/output-failure/responsiveness
workers require dedicated harnesses. Python tests cover signal identity,
bounds, protocol and process isolation. CLI checks cover arguments, FIFOs,
network/config/script isolation and output.

## Edge Cases

Decoder format may disappear while output drains. Retain prior evidence
without presenting it as current. The CLI handles failures once without native
diagnostics. Unknown events are ignored; queue overflow and unexpected
termination fail. Ctrl-C uses OS cleanup.

Pending: physical fallback/loss/latency, automatic recovery, hostile-media
fuzzing, write tracing/read-only mounts, wider codecs, SMB and EQ. Desktop seek
coalescing has its own validation. Virtual control/resource evidence is reference-only;
raw library callers retain the system allocator policy. The Linux/glibc CLI
sets a fixed 128 KiB mmap threshold before threads, reducing retained memory
without changing audio buffers. This affects its whole process, not other apps;
configuration failure exits nonzero. Other libc implementations skip it.

## Related Docs

[Audio target](../target/audio-and-equalizer.md),
[security](../target/security-and-privacy.md), [tests](../../testing/test-plan.md).
FFI/property references: upstream
[client header](https://github.com/mpv-player/mpv/blob/v0.41.0/include/mpv/client.h)
and [mpv manual](https://github.com/mpv-player/mpv/blob/v0.41.0/DOCS/man/input.rst).
