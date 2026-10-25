# Original-track spectrogram and tempo

## Read When

Changing diagnostics, BPM, beats, jobs or UI. Use `track-analysis`.

## Do Not Read For

[Live visualization](live-spectrum.md), [TRIM/FADE](playback-edges.md),
[MIX transitions](dj-mix.md) or [library analysis](library-analysis.md).

## Current Behavior

Artwork and [Library badges](library-analysis.md) open diagnostics, including after
Stop. Source-checked spectrum/BPM appears before lossy-source detection finishes.
Its progress then becomes a verdict without rebuilding the plot or selection.
Complete views survive navigation; pending current-track views reject other generations.
Cancel can keep a preview in the view; only final reports enter the [cache](analysis-cache.md).

A selectable BPM lane shows boundaries, local estimates and gray unknown gaps on
the shared time axis. Hover, click or keyboard selection shows interval timestamps;
variable results show the measured min–max range. At most 512 zones plus gaps render;
invalid/overlapping bounds are rejected. Windows span up to 32 seconds with approximate
boundaries. The player follows local tempo; the inspected snapshot stays fixed.

`resolibre-core::analysis` owns typed results and streams the shared 2,048-frame STFT
with channel-power averaging, avoiding destructive downmix. Adjacent columns merge
when full: at most 512 columns × 256 linear frequency rows remain. Each row uses
mean power of its strongest FFT bin, displayed in dBFS/bin (-120 to 0). A final
partial window cannot extend the duration axis. Input limits are four hours,
32 channels and bounded blocks; the original decoder accepts 8–384 kHz.
Unsupported limits fail explicitly.

Spectral extent requires four active full windows: the highest non-DC bin present
in at least 20%, above -90 dBFS/bin and within 50 dB of each window's strongest bin.
This conservative observation cannot certify transcoding, loudness or listening
quality. Silence/insufficient evidence is inconclusive. Codec compression and
origin differ: FLAC may contain previously lossy audio. Mixed-mode codecs stay
unknown; native FLAC STREAMINFO supplies encoded bit depth only when rate/channels
match the selected stream. Other formats remain unknown. Extensions, cutoffs,
high rates and clean-looking plots cannot certify original losslessness.
Full reports add [lossy-source evidence](lossy-source-detection.md) via a separate model.

Tempo combines 200 Hz energy rises with three 512-frame spectral bands at ~100 FFTs/s
and independent 2,048-frame bass at ~50 FFTs/s, held between observations. Only bass
sets its modulation threshold. Analysis uses at most 11,025 Hz, anti-alias filtering
and channel-power averaging. The same
pass measures [MIX sound edges](dj-mix.md).

Up to 512 windows retain spectral autocorrelation (~2.5 MiB) and 64 packed events
per band (~564 KiB). Noise-floor gates and direct recurrence establish an anchor
in two windows, weighted by duration/local evidence with a soft metrical preference.
Adjacent windows can establish another tempo without RMS beats; their confirming
neighbor must not also support the dominant tempo. Metrical relatives alone do
not establish a change. Strong conflicting evidence wins; unsupported gaps remain.

Known BPM needs less evidence than discovering tempo: direct recurrence near the
local maximum, or 1×–4× note changes with first-half phase fitting and second-half
count/strength/span/chance validation. Offbeat notes can confirm tempo without beat
phase. Context has lower confidence, no beats and cannot become an anchor or
authorize SYNC. Half/double/meter ambiguity includes spectral-only changes.

Precise beats use original energy onsets, fractional-period fitting and eight-second
phase checks. Weak windows retain 128 attacks/envelope (≤512 KiB). Recovery needs
four notes, 65% phase agreement, 40% span and a measured anchor within 64 seconds.
Actual attacks can replace an empty tempo-only row for the same interval without
overlap; confidence stays below SYNC. Beats are never fabricated; silence/noise stay
unmeasured. Incompatible resolved tempi stay variable; use the [cache upgrade](analysis-cache.md).

Rhythm analyzes 64 seconds, then the full track even with MIX off. MIX prepares the
queue's next occurrence, respecting shuffle/repeat. The strip follows local BPM;
gaps stay unknown. Jobs have a 10-second deadline, no spectrogram plot, one worker
and two identity-keyed reports. Playback revalidates stream/file before lookahead
reuse; changed sources or stale generations cannot label another track. Reports
alone do not authorize transitions. Current requests preempt library work; plots
share the decoder and wait for rhythm. Library jobs resume afterward. libmpv remains
the engine; MIX owns validated tempo changes.

Analyze decodes the retained file/selected stream through on-demand libmpv, reusing
loaded libraries without an output device. Float32 WAV enters an owned bounded pipe.
The idle handle's log callback survives overlapping teardown, preventing unfiltered
stderr. A preload hook selects the exact stream; the WAV reader rejects absent or
mismatched declared dimensions. EQ,
volume processing, ReplayGain, scripts, external references and disk output are
excluded. The reader closes its pipe before teardown to unblock native writers.
Source identity is revalidated before publication. TRIM/FADE retains its
FFmpeg adapter; both reuse the validated PCM reader/source identity. FFmpeg inputs
are reopened through retained descriptors for independent cursors on retries.

Progress: saved-plot loading, audio reading (%), BPM resolution, source detection
(completed/total excerpts) and saving; decode completion is not overall completion.
PCM uses fixed buffers. A five-minute deadline bounds work; per-block pacing targets
one-quarter core for Rust, not the whole process. Cancellation signals immediately;
the session polls cleanup and shutdown reaps work.
No source writes. Explicit [Library batches](library-analysis.md) share this scheduler;
[storage](analysis-cache.md) runs in these jobs.

## Key Files

- `crates/resolibre-core/src/analysis/`: matrix, bandwidth, activity;
  `tempo/{spectral,evidence,resolve,pulse,context}.rs`: tempo/phase evidence.
- `crates/resolibre-audio/src/rhythm_analysis.rs`, `session/rhythm.rs`: automatic current/next excerpts.
- `crates/resolibre-audio/src/original_audio/`: decoder and validated PCM.
- `crates/resolibre-audio/src/native_logging.rs`: shared native log-owner lifetime.
- `crates/resolibre-audio/src/track_analysis.rs`, `session/analysis.rs`: jobs/generations.
- `apps/desktop/src/analysis_window{.rs,/timeline.rs}`, `ui/analysis-window.slint`: retained inspection and heatmap.
- `apps/desktop/ui/components/tempo-timeline.slint`: selectable tempo zones.
- `apps/desktop/src/track_info.rs`, `ui/player.slint`: BPM strip and artwork action.

## Tests

Core: calibrated moving tones, silence, short/invalid input, matrix bounds,
opposite-phase channels, isolated high-frequency spikes, 40–240 BPM fixtures,
beat timing, variable tempo, noise, gain, layering, subdivisions and sparse notes.
Context: hi-hats/arpeggios, tempo steps/jitter/acceleration, legato bass/vibrato,
held-out phase/noise, regional clocks, attack provenance and bounded non-overlapping
intervals. Adapter: FLAC/MP3/transcodes, filtering, upsampling, misleading extensions,
original preservation, log ownership/restart, malformed/cancelled/changed input,
one-sample streams, multichannel phase, video/default streams and repeated retained-file
reads. Session: completion during playback, pause/cancel/replacement, stale generations
and automatic BPM/next-track cache without Analyze. UI: keyboard artwork activation,
close/focus restoration, frozen inspection/replacement, local BPM/ranges, zone
selection, gaps, invalid geometry and early preview/final-verdict updates. Tests
cover cancellation before saving, stale generations and preserved plot selection.
Native 1x/2x captures cover scales, Cancel/retry and keyboard reopening. Run tests and
`analysis_window::tests::native_analysis_preview`.

`check_playback_responsiveness.py --session-worker --track-analysis --trials 3`
adds analysis to the private marker/control audio workload and samples player/owned
decoder RSS. Short runs do not meet the 100-trial gate; report platform/device/GPU separately.

Earlier Linux: 360 cycles passed; audio 96.47 MiB, CPU 6.60–9.79% exceeded 5%,
software desktop 327/365 MiB exceeded 200 MiB. Hardware/100-trial/music gates remain
open. Core/RustFFT optimization: a 232.88 s FLAC fell from 34.28 s to 1.83 s; rhythm
excerpt 0.24 s. Local observations; no media bundled.

## Edge Cases

Cached measurements are observations, not verified provenance.
Variable/no-beat regions must not be extrapolated into a synthetic beat grid.
Four-hour/deadline failures allow retry; native teardown depends on OS scheduling.
File identity guards jobs; audio hashes key persistence.
Linux execution is validated here; Windows/Android/iOS native decoding, loading,
packaging and device lifecycles remain unvalidated. Shared Rust code alone does not
establish a working port. MIX owns transitions separately.

## Related Docs

[Plan](../../plans/track-analysis-plan.md), [shared FFT](spectral-analysis.md),
[analysis target](../target/analysis-and-radio.md), [tests](../../testing/test-plan.md).
References: [Ellis](https://labrosa.ee.columbia.edu/projects/beattrack/),
[FLAC STREAMINFO](https://www.rfc-editor.org/rfc/rfc9639.html#section-8.2).
