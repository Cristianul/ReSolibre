# Analysis and radio target

Status: partially implemented. Spectral/tempo analysis works on Linux; Track
Radio and genre inference need experiments. Internet radio and similar-track queues are distinct. A [live spectrum](../features/live-spectrum.md)
uses shared STFT; [full-track analysis/BPM](../features/track-analysis.md) works on Linux.

## Useful audio diagnostics

Provide a Spek-inspired full-track spectrogram, spectrum and technical metrics,
with a lightweight live spectrum, requested alongside full analysis. Use bounded FFT/STFT processing and
multiresolution/aggregated results for long tracks rather than retaining all PCM
or a full-resolution matrix in memory. Estimate sustained meaningful bandwidth
using time aggregation, noise-floor-aware thresholds, smoothing and outlier
rejection. Isolated high-frequency spikes are not useful sustained bandwidth.

Report codec, sample rate, bit depth when known and estimated spectral bandwidth.
Keep lossless encoding distinct from original provenance: a FLAC container can
hold audio previously transcoded lossily. A cutoff alone cannot prove provenance.
Use suspected/inconclusive results and provenance unknown unless verified; never
issue an unsupported "true lossless" badge. Record whether analysis concerns the
original file or a server-transcoded representation. This is audio diagnosis.

Approved, 2026-10-24: [lossy-source detection](../features/lossy-source-detection.md)
uses pinned offline lossprint, including Windows. Preserve uncertainty; measure
resources and integration accuracy separately from upstream benchmarks.

Approved UI: lossless-encoding badge and measured local BPM below the title; one full-analysis button appears
on artwork hover/keyboard focus and opens a grouped window with clear
measurements, interpretation and uncertainty. The always-enabled live spectrum
occupies the space between playback buttons and volume dial, preserving artwork.
Continuous unsegmented bars appear as diffused light beneath the chassis plastic,
without a framed display. Smooth attack/release motion follows measured audio;
presentation cadence is independent of analysis cadence, targeting 60 frames/s
where supported within existing resource budgets.
It reacts automatically during playback; pause freezes, stop/EOF clears, and
hidden/minimized views suspend visual work. The [focused plan](../../plans/track-analysis-plan.md)
records prerequisites and acceptance. The [technical strip](../features/track-information.md)
and analysis work on Linux; other platform adapters and device validation remain pending.

## Shared background execution

Track facts, spectral analysis and interpretation must work across Linux, Windows
and Android, with iOS considered. Share typed results and algorithms; OS-specific
decoder/output/tap implementations are allowed. Do not make a Linux monitor a
required feature dependency. Prove PCM access, timing, resource cost and privacy
for the intended platforms before selecting the live-analysis route.

Decision, 2026-10-22: retain libmpv and pursue an owned PCM filter. Engine migration
is not selected. The [isolated Linux proof](../../testing/live-tap-validation.md)
preserves captured audio and tolerates a stalled reader. Linux lifecycle checks
pass; physical synchronization and Windows/mobile packaging remain open. This is no performance or
sound-quality claim for a replacement engine.

Playback must not depend on analysis completion. Share reusable analysis logic
across clients where possible; platform scheduling/storage remain capabilities.
Use a managed, cancellable bounded job queue with this default ordering:

1. Playback-critical work and controls.
2. Current metadata and rhythm (quick estimate, then full timeline).
3. Metadata/artwork for visible UI content.
4. Upcoming queue content.
5. Remaining library indexing and optional expensive analysis.

Spectrograms, fingerprints, loudness, embeddings and genre inference are expensive. Promote useful work without blocking
audio. Bound concurrency, memory, source reads/network traffic and cache size;
throttle/pause for contention, constrained devices or battery policy. Avoid
redundant decodes when practical, without coupling playback to analysis workers.
Live analysis consumes samples through an isolated bounded path; drop visual
frames instead of blocking audio. No extra decoder or whole-library download
may be hidden behind simply displaying a track.

## Simple cache and algorithm evolution

Approved: persist results by audio hash, excluding tags/artwork; store artist/title
separately. The local [cache](../features/analysis-cache.md) uses SHA-256 and SQLite.
Future adapters must preserve source isolation and input revalidation.
Approved: [Library analysis](../features/library-analysis.md) adds batch/row actions,
forced replacement and success/failure badges. Keep small measurements and five plots in RAM; reload historical plots from disk. Playback and fresh analyses revalidate.

Persist source/profile-scoped identity, changes, job status and results. Revalidate
cache hits and reject stale completions. Allow interrupted-job retries, bounded
retention and clearing/rebuild.

No per-algorithm versioning now. When changing analysis,
explicitly invalidate/rebuild affected incompatible results in the application
upgrade path; code history alone cannot identify which algorithm produced a
particular installed cache. Source modification data is not an integrity proof.

Selectable older algorithms remain an uncommitted future option. Evaluate
quality, maintenance, security, licenses and resource cost before adding a choice.
If multiple algorithms coexist, distinguish their result provenance and prevent
mixing incompatible features/embeddings; decide the minimal storage contract then.

## Intelligent Track Radio

Generate a radio-like queue from available, authorized library music, fully offline.
Explore acoustic vectors/embeddings, rhythm/BPM, timbre, energy, spectral/harmonic
features and trustworthy genre metadata. Evaluate suitable existing Rust libraries
and compatible offline models/native adapters; do not train a large model from
scratch or assume inference must be reimplemented per platform.

Estimate BPM separately from the live display. Approved, 2026-10-24: combine energy,
short multiband spectra and longer bass analysis with bounded whole-track alternatives.
Verifying an independent anchor needs less evidence than finding a new tempo:
direct recurrence or locally fitted phase with held-out note/subdivision support.
Offbeat notes may confirm tempo without beat phase. Adjacent independent regions may establish a different tempo;
metrical relatives alone remain ambiguous. A soft preference must not override
convincing changes or force silence/unsupported noise onto a fixed tempo.

Delayed spectral timing cannot supply precise beats. Original-source attacks may
enrich the same contextual interval at lower confidence, never invent timestamps.
Context alone cannot authorize MIX synchronization. Preserve unknown regions and
evaluate false variability, metrical ambiguity and phase separately; spectral-only
half/double changes may remain ambiguous. The bounded offline [implementation](../features/track-analysis.md)
retains learned trackers as comparison candidates. [MIX](../features/dj-mix.md) owns transitions.

Detailed styles such as melodic/minimal techno, progressive/deep/tech house are
desired distinctions, inspired by Every Noise at Once and Chosic. These are not
promised APIs, bundled taxonomies or guaranteed classifications. Use confidence
and musical similarity; missing tags or uncertain genres must not disable radio.
Evaluate recommendation usefulness on representative music, inference resources,
offline availability and separate library/model/data redistribution rights.

Optional Last.fm/ListenBrainz suggestions require user-enabled network access.
Match candidates conservatively against available music: if only 18 of 100 exist,
use eligible matches and fill from local similarity. Never automatically download
or substitute unavailable recordings. Listening data leaves the device only with
explicit opt-in. Like/dislike feedback, local preference learning, repetition
avoidance and stricter genre matching are optional later improvements.

## Internet and hardware radio

Later support station favorites, explicit stream URLs and streaming metadata.
Investigate Radio Browser for country/language/genre/name search; directory use
is optional and direct streams must remain possible. Enumerate supported stream
formats and seek/live capabilities. Bound redirects, playlists and reconnect
attempts; a station response cannot grant arbitrary network/filesystem access.
Directory failure must not break local music or saved direct stations.

Hardware tuners such as USB SDR are a low-priority possibility only where suitable
hardware/drivers and platform access exist. No phone radio hardware is assumed.
Future radio timeshift provides pause, delayed resume, bounded rewind and return
to LIVE through a circular buffer with duration and byte/storage limits. Handle
disk-full/disconnects and clear private buffered content by a defined policy;
never accumulate an uncontrolled recording archive.

## Evidence

[Planned analysis tests](../../testing/expanded-features-test-plan.md),
[performance protocol](../../testing/test-plan.md) and
[processing](audio-processing.md) cover resource and audio interaction.
[Essentia models](https://essentia.upf.edu/models.html) illustrate analysis options
and model-specific licensing; they are not an approved dependency or bundle.
