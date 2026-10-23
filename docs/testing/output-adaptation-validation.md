# Controlled output adaptation

## Scope and environment

Follow the core map, applicable rules, local playback feature, audio target and
test plan. This record covers simulated-output negotiation, separately from the
[speaker listening record](local-audio-validation.md).

On 2026-10-20, the existing extracted libmpv 0.41.0 resolved all dependencies.
`wpctl`, `pactl` and `pw-cli` were available and the Pulse-compatible session
answered; an `mpv` executable was absent. The libmpv tests need no executable or
audio server. No packages, system audio settings or physical devices changed.

The selected `null` sink supports restricted layouts and PCM representations.
Its [versioned manual](https://github.com/mpv-player/mpv/blob/v0.41.0/DOCS/man/ao.rst)
and [implementation](https://github.com/mpv-player/mpv/blob/v0.41.0/audio/out/ao_null.c)
establish that `ao-null-channel-layouts=stereo` and `ao-null-format=float`
constrain the sink. Global `audio-channels`, `audio-format` and `audio-samplerate`
are not forced. The sink has no sample-rate restriction option; rate conversion
is outside this experiment.

## Reproduction and evidence

From the repository root, using the existing local libraries:

```sh
export RUSTFLAGS="-L native=$PWD/.local/native/usr/lib64"
export LD_LIBRARY_PATH="$PWD/.local/native/usr/lib64"
cargo test -p resolibre-audio --lib output_adaptation_tests --offline -- --nocapture
```

The test generates two seconds of mono WAV PCM16 at 44,100 Hz (quiet 440 Hz
tone), in a private temporary directory with read-only file permissions. Three
fresh engines play the same fixture sequentially:

| Sink | Decoder source | Audio API output | Result |
| --- | --- | --- | --- |
| Unrestricted null | 44,100 Hz, mono, Signed16 | Same as source | Progress, EOF |
| Stereo/float-only null | 44,100 Hz, mono, Signed16 | 44,100 Hz, stereo, Float32 | Progress, EOF |
| Fresh unrestricted null | 44,100 Hz, mono, Signed16 | Same as source | Progress, EOF |

All three passed. Each requires paired source/output evidence, at least 0.5 s
of engine-position advancement and EOF within eight seconds of opening. Missing
formats during drain are allowed; formats must clear after EOF. Original bytes,
mtime and permissions are checked after each engine is destroyed. The final
engine checks that restrictions do not leak between instances.

`RestrictedSilent` and its configuration branch use `#[cfg(test)]`: Rust includes
them only in the adapter's unit-test build. Normal builds retain `Device` and
`Silent`; the CLI and production output policy have no new switches.

## Negative checks and limits

A second test requires malformed input to fail without a successful lifecycle
event or private diagnostics, within five seconds of opening, and preserves its
bytes/metadata. Both tests remove only their owned temporary directories.
The event-loop deadlines do not bound synchronous native open or destruction.

This establishes automatic channel/sample conversion and engine completion with
a constrained simulated sink. Null discards samples: neither converted waveform
quality, glitch-free physical playback, actual audio latency, DAC negotiation,
resampling nor bit-perfect output is established by this null check. A separate
[PCM check](pcm-conversion-validation.md) validates requested rate conversion.
Physical-device fallback and device-loss handling remain pending. No native dependencies or input trust
boundaries were added; existing native parser and resource-limit gaps remain.

The full verification run also passed twelve Rust tests, core-only tests without
native exports, Clippy, rustfmt, CLI rejection/isolation and default-device
silence, and documentation validation. The device check is a connected regression,
not evidence for the simulated constraint. Listening was not repeated.
