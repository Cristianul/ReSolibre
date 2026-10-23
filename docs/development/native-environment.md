# Native development environment

Use this for setup and environment reproduction. For behavior, see the
[local playback prototype](../architecture/features/local-playback.md).

Reference checked on 2026-10-20: Fedora 45 prerelease, x86_64; distribution Rust
and Cargo 1.99.0, rustfmt, Clippy, C linker and pkg-config. Rustup is absent and
is not required by these commands. This records a working environment, not a
minimum supported toolchain or a release matrix.

System libmpv was absent. For this session, signed Fedora RPMs were downloaded
and extracted under ignored `.local/native/`: mpv-libs/mpv-devel 0.41.0-10.fc45,
mujs 1.3.7-3.fc45, libXpresent 1.0.0-29.fc45 and vapoursynth-libs 79-3.fc45.
No packages were installed system-wide. The library uses installed FFmpeg
9.0.2 components. `mpv.pc` reports **client API 2.5.0**, not engine 2.5.0.
The adapter checks ABI major 2 and fails on unsupported configuration options.
Other engine builds/distributions are unverified. Core uses pure-Rust RustFFT;
audio also uses serde_json. [Desktop setup](desktop-environment.md) covers GUI dependencies.
The bundled live tap needs a C11 compiler; `cc`, `libc` and `libloading` stay in
the native adapter. Its LADSPA ABI header and license are vendored.

For a normal system setup, provide libmpv and its linker/development package
(`mpv-libs`/`mpv-devel` on this Fedora reference). Verify with
`pkg-config --modversion mpv`. To reuse this session's extracted packages, set
the following in the current shell instead:

```sh
export RUSTFLAGS="-L native=$PWD/.local/native/usr/lib64"
export LD_LIBRARY_PATH="$PWD/.local/native/usr/lib64"
export PKG_CONFIG_PATH="$PWD/.local/native/usr/lib64/pkgconfig"
```

Run from the repository root:

```sh
cargo test -p resolibre-core --offline
cargo test --workspace --all-targets --offline
# Single-threaded GNU allocator startup regression (included above):
cargo test -p resolibre-audio --test process_memory --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo fmt --all -- --check
cargo build -p resolibre-audio --example local-play --offline
python3 tools/check_local_playback.py
python3 tools/check_local_playback.py --device
# Silent, restricted-output evidence (also included in the workspace tests):
cargo test -p resolibre-audio --lib output_adaptation_tests --offline -- --nocapture
python3 -m unittest discover -s tools -p 'test_pcm_*.py' -v
python3 tools/check_pcm_conversion.py
python3 -m unittest discover -s tools -p 'test_output_*.py' -v
python3 tools/check_output_failure.py
python3 -m unittest discover -s tools -p 'test_responsiveness*.py' -v
python3 tools/check_playback_responsiveness.py --check-reference-budgets
python3 tools/check_audible_playback.py
cargo run -p resolibre-audio --example local-play --offline -- /path/to/music.wav
python3 tools/check_docs.py
git diff --check
git diff --cached --check
```

`--device` plays generated silence through the real default output; it requires
a working audio session. Ordinary native tests use `ao=null`; the separate PCM
harness drives an ignored test worker with `ao=pcm`. Core-only tests need no
libmpv or native exports. The output-failure harness requires PipeWire,
pipewire-pulse, pactl, parec and pw-link; it owns a private virtual server.
See the [failure protocol](../testing/output-failure-validation.md) for limits.

The audible check plays generated quiet mono/stereo tones. Follow the
[listening protocol and evidence](../testing/local-audio-validation.md) when
performing it; human confirmation is separate from automatic success. The
example now reports decoder-source and audio API formats, not physical DAC
measurements. libmpv availability and dependency resolution were rechecked
for the recorded listening run; no system packages were installed.

The Fedora mpv package declares GPL-2.0-or-later plus LGPL/BSD/ISC/MIT
components. RPM signatures were verified before extraction. Full native
dependency advisory and distribution-license audits remain release work.
