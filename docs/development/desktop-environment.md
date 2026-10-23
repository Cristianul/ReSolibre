# Desktop development environment

Linux x86_64 reference: same libmpv/FFmpeg and Rust setup as the
[native guide](native-environment.md). Provide fontconfig, FreeType, Wayland and
xkbcommon development packages, an X11/Wayland session and a working desktop portal.
Slint 1.18.1 uses Winit/FemtoVG with accessibility; rfd 0.17.2 uses the XDG portal.
No Qt, webview, embedded test server, tray or broad image-format features enabled.
Cargo.lock pins dependencies; see [local build scripts](local-builds.md) for debug/release setup.

On this workstation, signed Fedora development RPMs were extracted under ignored
`.local/native`, without system installation. The native guide's RUSTFLAGS and
LD_LIBRARY_PATH apply; additionally set PKG_CONFIG_PATH to its `usr/lib64/pkgconfig`.
The first dependency fetch needs internet; subsequent commands use the lockfile.

```sh
cargo run -p resolibre-desktop --locked
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
python3 tools/check_session_volume.py
python3 tools/check_equalizer.py
python3 tools/check_playback_responsiveness.py --session-worker --trials 3 --skip-resources
```

Use Open in Library to choose local music. Play/Pause, Stop, Previous/Next, time/seek
and app volume are functional. Shuffle and Repeat Off/All/One share core policy.
LIB/EQ toggle separate Library/fixed EQ drawer; both start visible. EQ has ten bands, manual preamp, bypass/reset
and built-in/saved presets. Save writes only the selected app slot. Queue rows provide Up/Down and Remove; removing
the playing row stops audio. Exit stops playback; [session intent restores](../architecture/features/session-persistence.md) without autoplay.
Library [docks/snaps](../architecture/features/detachable-playlist.md) via default X11/XWayland; `--native-display` opts out.
CLI music paths remain unsupported. The internal
`--file-picker` helper emits binary frames only to its parent's inherited channel.

For native visual inspection, create an ignored output directory, set
`RESOUL_PREVIEW_DIR` to its absolute path, and run:

```sh
cargo test -p resolibre-desktop native_preview -- --ignored --nocapture
MALLOC_MMAP_THRESHOLD_=65536 cargo test -p resolibre-desktop native_session_journey -- --ignored --nocapture
MALLOC_MMAP_THRESHOLD_=65536 cargo test -p resolibre-desktop native_queue_edit_journey -- --ignored
MALLOC_MMAP_THRESHOLD_=65536 cargo test -p resolibre-desktop native_modes_journey -- --ignored
MALLOC_MMAP_THRESHOLD_=65536 cargo test -p resolibre-desktop native_equalizer_journey -- --ignored
```

The preview uses fabricated display values; it is not audio evidence. Explicit
real-file native journeys use `RESOUL_TEST_FLAC` and `RESOUL_TEST_MP3` as local env
inputs with `cargo test -p resolibre-audio --test session supplied_music_journey -- --ignored`.
Default output is silent; `RESOUL_TEST_DEVICE=1` selects the real device. Keep paths
out of tracked files/reports and compare source hashes/mtime/mode before and after.

Slint is used under its GPL-3.0-only option; rfd is MIT. Lucide 0.468.0 assets retain
`apps/desktop/ui/assets/LICENSE-lucide` (ISC plus upstream MIT notice). Preserve all
transitive notices/source obligations when packaging; no distribution audit is
claimed. See the [validation record](../testing/local-desktop-validation.md) for
advisory results and limitations.
