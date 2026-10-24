# Lossy-source detection

## Read When

Changing recording-provenance evidence, detector sampling, model packaging or its
Analyze presentation. Use the `lossy-source-detection` reading profile.

## Do Not Read For

Codec badges ([track information](track-information.md)), spectral bandwidth or
BPM ([track analysis](track-analysis.md)), or playback output quality.

## Current Behavior

Full Analyze, including automatic current/MIX-next and library requests, adds an
experimental local detector of earlier lossy encoding. The compact Lossless badge
still describes the codec. Analyze separately reports **Likely lossy source**,
**No lossy traces detected**, or **Inconclusive**, with the raw model score and
number of usable excerpts. No result certifies an original master or listening
quality. There are no uploads, runtime downloads or GPU requirements.

The pinned MIT `lossprint` 1.1.0 crate embeds model 1.0 and uses CPU-only `tract`.
Its build verifies model SHA-256
`ca86c67b4035485a9c1a3b3120b4a555cb7af87b4dd28837c46b297f82c48e7d`.
Cargo.lock fixes transitive dependencies. The native build scripts stage the
lossprint/model and tract MIT notices from `licenses/` beside the executable.
The published crate carries the model; no additional Windows DLL is needed for
inference. Existing libmpv runtime requirements still apply.

After the ordinary full decode establishes actual duration, a second retained-file
native decode supplies mono/stereo float32 PCM at the original 44.1–192 kHz rate.
It applies no EQ, ReplayGain, resampling, downmix or normalization. Formats outside
these evaluated dimensions are explicitly unsupported; they remain playable.
Tracks below two seconds and excerpts below -70 dBFS mean power are inconclusive.

At most six non-overlapping 14.5-second excerpts span the middle 92%. Short tracks
use one full or centered excerpt. Unlike upstream's potentially overlapping six
windows, the count is limited by available non-overlapping audio. This adaptation
avoids repeatedly weighting nearly identical samples in short tracks. It is not
covered by upstream's exact benchmark protocol. Only one PCM excerpt is retained
(at most 21.3 MiB); inference has additional working memory. A private automatically
deleted temporary WAV of the same bound bridges the library's File-only API.
The model never decodes an untrusted original/container length directly.

Model scores are averaged over audible excerpts. The portable policy uses >=0.9
for likely lossy, <=0.1 for no traces, otherwise inconclusive. Excerpts spanning
both decision bands force inconclusive. Scores are not calibrated probabilities
of authenticity. A single suspicious excerpt is not a whole-track certificate.
Unavailable detector work preserves completed spectrum/tempo; retry is explicit.

The existing single full-analysis worker owns decoding and inference. A separate
90-second cooperative deadline applies to this stage. Cancellation is checked
between blocks and before/after model calls; initialization and one bounded
inference call cannot be interrupted. Unix file quotas are checked before writing
the scratch WAV, avoiding fatal SIGXFSZ; quota failure is recoverable. Native pipe backpressure pauses decoding
while inference runs. Transport never joins unfinished work. Shutdown still reaps
the worker. Input identity is checked before publication/cache write, and stale
session results remain rejected. Originals are read-only.

[Cache schema 6](analysis-cache.md) invalidates schema-5 full reports; separate
rhythm rows survive. Earlier schemas retain their existing tempo invalidation.
Finite scores, ordered ranges and 1–6 excerpts are validated on load/save. Missing
provenance is a cache miss, not invented evidence. Unsupported/inconclusive outcomes
can be cached; forced Analyze retries unavailable results.

## Key Files

- `crates/resolibre-core/src/analysis/provenance.rs`: typed evidence and verdicts.
- `crates/resolibre-audio/src/track_analysis/provenance.rs`: bounded decoder/model bridge.
- `crates/resolibre-storage/src/{lib,validation}.rs`: migration and validation.
- `apps/desktop/src/analysis_window/provenance.rs`, `ui/analysis-window.slint`: presentation.
- `licenses/`, `tools/build_support.py`: required detector notices.

## Tests

Direct tests cover sample planning/bounds, PCM preservation, silence, cancellation,
contradictory/invalid scores, cached JSON/schema upgrades and honest UI labels.
An isolated child with a real 2 MiB file quota proves scratch writes fail safely.
The playback stress harness allows 24 MiB only for the analysis worker; monitors
and captured diagnostics retain their smaller limits.
Existing native analysis tests cover malformed/changed inputs, cancellation,
retagged cache reuse and original-byte preservation. Native previews inspect text
wrapping at 1x/2x. `measure_authorized_provenance` is an opt-in full-pipeline probe
using `RESOUL_PROVENANCE_PROBE`; it prints only measurements. The decoder-free
`measure_decoded_provenance` probe accepts an owned 14.5-second, 44.1 kHz stereo
float32 WAV in `RESOUL_PROVENANCE_PCM`; `RESOUL_PROVENANCE_EXPECT` optionally checks
the expected verdict. It exercises the same model bridge on Linux/Windows.

Initial Linux development-build evidence: the authorized 209.72-second 96 kHz
recording returned no lossy traces across six excerpts, in 14.24 seconds including
full spectrum/tempo analysis; peak test-process RSS was 569.6 MiB. This recording
has unknown provenance and is not a negative ground-truth label. A 14.5-second
reference excerpt returned no traces; MP3 320 kbps and AAC 256 kbps versions decoded
back to FLAC both returned likely lossy. These are smoke checks, not population
accuracy, calibration or a physical-device performance gate. A generated PCM
musical control and its low-pass/96 kHz upsampled variant both returned no traces.

The audio crate and tests cross-compiled for Windows x86_64 GNU. Under an isolated
Wine 11 prefix, three direct tests and four real-model probes passed with matching
Linux verdicts; the full Windows libmpv/retained-file pipeline also detected the
MP3-to-FLAC control. This validates Windows code execution under Wine, not native
MSVC packaging, real Windows devices or interactive desktop behavior. Those remain
unverified for this feature. The downloaded runtime/toolchains are ignored local
validation inputs, not application dependencies.

Linux control/resource smoke: six WAV/FLAC sessions and 360 open/seek/drop
cycles passed with analysis active. Command dispatch maxima were 1.07/1.02 ms;
peak audio-process RSS was 511.26 MiB, with final post-drop samples 120.96–123.11
MiB. Background-analysis workloads used 74.96–113.14% of one core. These exceed
the existing idle/playback resource objectives; they are bounded expensive-work
measurements, not a lightweight steady-state or physical-device acceptance claim.
The 100-trial/reference-hardware gates remain open. Core/storage/desktop suites,
audio integrations and focused analysis regressions passed; native 1x/2x previews,
build-wrapper tests, documentation checks and Clippy also passed (the pre-existing
picker unused-mut warning is allowed).

Cargo audit found no known vulnerabilities in the locked graph. Existing
unmaintained dependencies and the yanked clipboard-win version remain reported;
this is not a complete redistribution or supply-chain audit.

## Edge Cases

Bandlimited masters, unusual material, unseen encoders and post-encoding processing
can cause mistakes. Lossprint's reported false positives and detection rates are
the author's benchmarks, not independently reproduced acceptance evidence.
High-bitrate AAC remains a documented weak case. Cutoffs, noise above 24 kHz,
upsampling and padded bit depth alone never decide our verdict. The model cannot
recover a recording's complete production history. Temporary-file failure yields
unavailable evidence; it must not interrupt playback or erase useful analysis.

## Related Docs

[Analysis target](../target/analysis-and-radio.md), [test plan](../../testing/test-plan.md),
[Windows](../../development/windows-native.md).
Research: [lossprint library](https://github.com/maxdjohnson/lossprint),
[model card](https://huggingface.co/maxdj/lossprint), and
[Koops et al., ISMIR 2024](https://arxiv.org/abs/2407.21545), which demonstrates
generalization failures on unseen codec settings and why cutoff-only reasoning
is insufficient. AES 2019's AAC-specific quantization-error approach is another
research direction; the current implementation uses the reusable multicodec model.
