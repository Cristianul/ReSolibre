# Original-track spectrogram and tempo

## Read When

Changing on-demand original-audio diagnostics, BPM, observed beat times, analysis
jobs or their grouped desktop window. Use `track-analysis` in `tools/check_docs.py`.

## Do Not Read For

[Live visualization](live-spectrum.md), [TRIM/FADE](playback-edges.md),
[MIX transitions](dj-mix.md) or [library analysis](library-analysis.md).

## Current Behavior

Analyze on artwork hover/focus opens a window with title, progress,
Cancel/retry and time/frequency/power scales. Completed inspections retain one
immutable report, title, facts and image across navigation/background completions;
only explicit Analyze selects a replacement. Navigation before completion marks
the unfinished view interrupted. Closing cancels its own request; the
[persistent cache](analysis-cache.md) reuses completed measurements.

A selectable BPM lane shares the time axis, with boundaries, local estimates
and gray unknown gaps. Hover, click or keyboard selection
shows interval timestamps. Variable results show the measured min–max range.
At most 512 measured zones plus gaps are rendered; invalid/overlapping bounds are
rejected. Windows span up to 32 seconds; boundaries are approximate. The live player follows local tempo; the inspected snapshot stays fixed.

`resolibre-core::analysis` owns algorithms and typed results. It streams the shared
2,048-frame STFT, averages channel power without destructive downmix, and merges
adjacent time columns when the matrix fills. At most 512 columns × 256 linear
frequency rows are retained; the matrix uses mean power of the strongest FFT bin
per row, displayed in dBFS/bin (-120 to 0). The final partial window cannot extend
the duration axis. Input is capped at four hours, 32 channels and bounded blocks.
The original decoder accepts 8–384 kHz. Unsupported limits fail explicitly.

Spectral extent is the highest non-DC bin present in at least 20% of active full
windows, above both -90 dBFS/bin and 50 dB below that window's strongest bin. At
least four active windows are required. This is a conservative content observation,
not a calibrated transcode detector, loudness measurement or listening-quality
score. Silence/insufficient evidence is inconclusive. Codec compression and origin
are separate: FLAC can hold previously lossy audio. Mixed-mode codecs stay unknown;
native FLAC STREAMINFO supplies encoded bit depth when rate/channels match the selected stream; other formats remain unknown. No extension,
cutoff, high rate or clean-looking plot certifies original losslessness.

Tempo uses per-channel broadband/low-pass energy at 200 bins/s; the same pass
measures bounded sound edges for [MIX](dj-mix.md).
Four successive autocorrelation lags distinguish repetition from short-lag side
peaks; fractional periods and strongest phase clusters avoid integer timing drift
and averaging between subdivisions. A periodic peak must rise above the noise
correlation floor. Tempo confidence and observed-beat confidence are separate:
a supported BPM can have an empty beat timeline. Eight-second neighborhoods retain
only supported attacks; missing beats are never filled or extrapolated. Windows
with incompatible tempos stay variable; half/double/meter ambiguity remains.
Unresolved windows retain at most 128 attack times per envelope (at most 512 KiB
over 512 windows), never full PCM. Agreeing independent tempo supplies a prior:
at least four notes, 65% phase agreement and 40% window span must support recovery,
using a measured anchor within 64 seconds. Recovered intervals have lower confidence
and cannot authorize SYNC. Silence, off-phase notes, noise and conflicting tempo
stay unknown; no missing beat timestamps are fabricated.

Current rhythm analyzes 64 seconds, then the full track even with MIX off. MIX then
prepares the queue's next occurrence, respecting shuffle/repeat. The strip follows
local variable BPM; gaps stay unknown. Jobs have a 10-second deadline, no FFT/plot,
one worker and two identity-keyed reports. Playback revalidates stream/file before
lookahead reuse; changed sources or stale generations cannot label another track.
These reports alone do not authorize transitions. Current requests preempt library
work; plots share the decoder and wait for rhythm. Library jobs resume afterward.
libmpv remains the engine; MIX owns validated tempo changes.

Analyze decodes the retained file/selected stream in an on-demand libmpv handle,
reusing loaded libraries. It opens no output device; Float32 WAV enters an owned bounded pipe.
An idle handle retains libmpv's log callback through overlapping teardown, preventing
unfiltered stderr. A preload hook selects the exact stream; the WAV reader rejects
missing or mismatched declared dimensions. EQ,
volume processing, ReplayGain, scripts, external references and disk output are
excluded. The reader closes its pipe before engine teardown to unblock native
writers. Source identity is revalidated before publication. TRIM/FADE retains its
FFmpeg adapter; both reuse the validated PCM reader/source identity. FFmpeg inputs
are reopened through retained descriptors for independent cursors on retries.

PCM streams through fixed buffers, never a whole-file allocation. A five-minute
wall deadline bounds work; per-block pacing targets roughly one-quarter of a core
for the Rust consumer, without claiming a total process CPU cap. Cancellation is
signalled immediately and the session polls cleanup; shutdown reaps owned work.
No source writes. Explicit [Library batches](library-analysis.md) share this scheduler;
[storage](analysis-cache.md) runs in these jobs.

## Key Files

- `crates/resolibre-core/src/analysis/`: matrix, bandwidth, tempo/beats, activity.
- `crates/resolibre-audio/src/rhythm_analysis.rs`, `session/rhythm.rs`: automatic current/next excerpts.
- `crates/resolibre-audio/src/original_audio/`: decoder and validated PCM.
- `crates/resolibre-audio/src/native_logging.rs`: shared native log-owner lifetime.
- `crates/resolibre-audio/src/track_analysis.rs`, `session/analysis.rs`: jobs/generations.
- `apps/desktop/src/analysis_window{.rs,/timeline.rs}`, `ui/analysis-window.slint`: retained inspection and heatmap.
- `apps/desktop/ui/components/tempo-timeline.slint`: selectable tempo zones.
- `apps/desktop/src/track_info.rs`, `ui/player.slint`: BPM strip and artwork action.

## Tests

Core: calibrated moving tones, silence, short/invalid input, fixed matrix bounds,
opposite-phase channels, isolated high-frequency spikes, 40/60/90/113/120/128/140/174/220/240 BPM,
beat timing, variable tempo, noise, gain, layering, subdivisions and contextual sparse notes. Adapter:
FLAC/MP3/transcodes, filtering, upsampling, misleading extensions, originals,
log ownership/restart, malformed/cancelled/changed input, one-sample streams,
multichannel phase, video/default stream selection and repeated retained-file reads. Session: completion during
playback, pause/cancel/replacement and stale generations; automatic BPM/next-track cache without Analyze. UI: keyboard artwork
activation, close/focus restoration, frozen inspection/replacement, local BPM/ranges,
zone selection, unknown gaps and invalid/oversized geometry;
native 1x/2x captures inspect scales and wrapping. A production UI journey exercises
Cancel/retry, measured BPM, closing and keyboard reopening. Run workspace tests and the
`analysis_window::tests::native_analysis_preview` isolated window case.

`check_playback_responsiveness.py --session-worker --track-analysis --trials 3`
adds analysis to the private real-audio marker/control workload and samples owned
child decoder RSS alongside the player. Short runs are not the 100-trial gate;
platform/device/GPU results must be reported separately.

Earlier Linux runs passed 360 lifecycle cycles (96.47 MiB audio). CPU 6.60–9.79%
exceeded 5%; software desktop 327/365 MiB exceeded 200 MiB. Hardware rendering,
100-trial gates and broad music accuracy remain open. Optimized core/RustFFT reduced
one authorized 232.88 s FLAC analysis from 34.28 s to 1.83 s; its rhythm excerpt took
0.24 s. These are local observations; no user media is bundled.

## Edge Cases

Cached measurements are observations, not verified provenance.
Variable/no-beat regions must not be extrapolated into a synthetic beat grid.
Four-hour/deadline limits fail with retry available. Native teardown can depend on
OS scheduling. File identity guards jobs; audio hashes key persistent results.
Linux execution is validated here; Windows/Android/iOS native decoding, loading,
packaging and device lifecycles remain unvalidated. Shared Rust code alone does not
establish a working port. MIX owns transitions separately.

## Related Docs

[Plan](../../plans/track-analysis-plan.md), [shared FFT](spectral-analysis.md),
[analysis target](../target/analysis-and-radio.md), [tests](../../testing/test-plan.md).
References: [Ellis](https://labrosa.ee.columbia.edu/projects/beattrack/),
[FLAC STREAMINFO](https://www.rfc-editor.org/rfc/rfc9639.html#section-8.2).
