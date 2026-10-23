# Shared spectral analysis

## Read When

Changing decoded-PCM frequency analysis. Use root/core rules, the core map,
security and tests (`spectral-analysis` reading profile).

## Do Not Read For

Native loading, UI motion, full-track jobs or provenance interpretation. See [full analysis](track-analysis.md). The
[tap experiment](../../testing/live-tap-validation.md) is separate from this library.

## Current Behavior

`resolibre-core::spectrum` implements a streaming STFT using RustFFT 6.4.1 with
runtime-selected SIMD and a scalar fallback. It has no native/GUI/decoder dependency and is
used by [live](live-spectrum.md) and [full analysis](track-analysis.md). A worker can supply interleaved Float32
PCM and immediately consume borrowed, reused spectrum results. Allocate the
analyzer outside the audio callback; analysis does not belong in that callback.

Each periodic Hann window contains 2,048 frames, advances by 1,024 frames and
emits 1,025 nonnegative-frequency bins. Per-channel power is accumulated before
averaging, so opposite-phase channels do not cancel. The one-sided normalization
includes window energy and doubles interior bins only. Summed linear bin powers
represent window-weighted mean-square energy relative to a full-scale sample
squared. These are bin powers, not power per Hz or an amplitude scale. Silence
uses a -160 dB display floor. Over-full-scale input is not silently clipped.

Results include sample rate, start/end frame positions and the actual frame count
before EOF padding. The adapter must attach track/generation identity and map
positions to its playback timeline; these counters are not native timestamps.
`reset` discards history after seek/replacement. `finish` emits one padded window
when new input remains; repeated calls without new input emit nothing. A partial
window includes zero padding in its normalization and is not an unbiased loudness
measurement. No bandwidth, tempo or provenance inference exists here.

Storage and work are bounded: 1–32 channels, sample rate 1–1,536,000 Hz and at most
16,384 frames per input block. Invalid dimensions, incomplete frames, nonfinite
samples, oversized blocks and position overflow are rejected before mutation.
The f64 transform avoids numeric overflow from finite Float32 extremes. History
uses at most 256 KiB; FFT scratch/window/results add fixed allocations, independent
of track duration. Construction allocates; processing reuses storage. Consumers
must bound their own callbacks, queues, aggregation and retention.

## Key Files

- `crates/resolibre-core/src/spectrum.rs`: validated streaming transform and results.
- `crates/resolibre-core/src/spectrum/tests.rs`: deterministic signals and invalid input.
- `crates/resolibre-core/Cargo.toml`, `Cargo.lock`: pinned pure-Rust FFT dependency.

## Tests

Six direct tests cover calibrated tones at two rates, mono/stereo/six/32 channels,
opposite phase, DC/Nyquist, gain, chunk-independent overlap/timing, partial EOF,
reset/silence, extreme finite PCM and atomic rejection of malformed input.
`cargo test -p resolibre-core spectrum --locked`, workspace/all-target tests and
workspace Clippy passed on Linux. Live consumer tests also cover cadence,
bounded retention, stale/gapped input, seek offsets and malformed clocks/chunks.
No Windows/Android/iOS execution, live cadence benchmark, whole-app resource
acceptance or native synchronization is implied by portable source code.

## Edge Cases

The 2,048-frame window is an initial shared primitive, not a validated final
resolution for every sample rate. Live bands and motion are implemented separately;
offline aggregation, cancellation, bandwidth and tempo belong to `analysis`.
Callers must reset when the input has a discontinuity rather than
silently joining unrelated PCM. The EOF window must not extend the track duration.

RustFFT is MIT/Apache-2.0; its normal dependency graph is pure Rust. Registry
checksums are locked. No native SDK, network access, file input, cache or profile
state is introduced into core. A full release dependency advisory/license audit
remains outstanding; version pinning is not a security audit.

## Related Docs

[Technical information](track-information.md), [analysis target](../target/analysis-and-radio.md),
[test plan](../../testing/test-plan.md), [RustFFT](https://docs.rs/rustfft/6.4.1/rustfft/).
