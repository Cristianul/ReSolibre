# Playback edges: TRIM and FADE

## Read When

Changing edge detection, playback bounds, fades, source-time seeking or their UI/storage.
Read both area rules/maps and the [test plan](../../testing/test-plan.md).

## Do Not Read For

Crossfade mixes two tracks and remains separate. Spectrum/loudness/provenance analyses
read original audio; they must not consume the playback interval or processed PCM.

## Current Behavior

TRIM skips edge silence below -55 dBFS (amplitude ~0.001778), raised from -60 dBFS
for slightly stronger trimming. All channels must qualify for 0.5 s; retain 50 ms.
Internal pauses, short quiet edges and wholly silent tracks remain intact. Opposite
phase channels are checked separately, never averaged. Original files are read-only.
FADE cycles Off → In → Out → Both → Off. Linear sample ramps last one second at the
effective start/end. Short tracks cap single ramps to their length and dual ramps
to half their length. This is not crossfade or automatic headroom compensation.

The core owns settings and effective source bounds. Playback opens, all seeks,
replays, queue/repeat transitions and forward/backward scans use these bounds.
Native start/end enforce the interval; a separate lavfi envelope follows source
PTS through seeks. User volume/EQ remain independent. Original-audio analysis
explicitly bypasses playback processing via its separate decoder.

The seek bar retains original duration/timestamps; yellow segments show excluded
edges. Seeking into yellow clamps to the effective boundary. Scanning holds on the
current track; ordinary forward completion uses existing queue policy. Live toggles
preserve source position when inside the new bounds and retain pause intent.

Initial playback starts with original bounds while TRIM/FADE measurement runs in the
background. When complete, it applies the measured bounds and moves playback and
the source-time seek bar to the first retained sound if the current position is
excluded. Playback, pause and seek remain usable during measurement; completion
respects the latest pause intent. Stop/navigation cancel stale work. Partial
results never change playback bounds.
MIX waits for the current track's final TRIM bounds before planning an overlap;
the current audio continues during that wait.
Audio heard before measurement completes uses the original bounds and bypassed
fade filters; a very short opening may finish before its fade is available.

Live in-range changes retain buffered audio: no forced pause or position-preserving
seek. TRIM changes native bounds; FADE updates two fixed, initially bypassed
`afade` filters using runtime parameters/enable commands. Neither toggle rebuilds
the active graph. New processing reaches subsequently decoded audio; queued samples
are retained, including paused edits. Startup still flushes safely; enabling TRIM
outside its new bounds seeks to the nearest retained edge.

Before explicit seeks, fade intent is committed alongside EQ so graph resets retain
both. Format events restore only live parameters differing from that configuration;
short-file drain can leave no active graph to command. Seek reconstruction gates
live DSP updates until native negotiation is ready.

An owned FFmpeg process decodes the selected audio stream from the player's retained
read-only descriptor with an independent cursor per job. It has no playback
filters, resampling or channel downmix.
Only fd input and allowlisted containers are accepted; video/subtitles/data are
excluded. Protocol restrictions prohibit network and external file references.
Stderr is discarded and FFREPORT removed; raw diagnostics and paths are not logged.

The reader streams float PCM with ~64 KiB storage; headers <=4096 bytes, 1–32
channels, 8–384 kHz, finite samples. One job, one decoder/filter thread, 32 MiB per
native allocation, 1 MiB probe and 15-second wall deadline. These are not a total
process RSS guarantee. Cancellation kills/reaps the owned process. Missing FFmpeg,
unsupported/malformed sources or deadlines produce one safe warning and original
playback with an explicit unavailable status. No partial boundary is applied.

A 64-entry session cache uses device/inode, size, nanosecond modification/change
times and selected stream; identity is checked before/after decoding. It is not
persisted. TRIM/FADE intent is persisted: session payload v2 appends two bounded
bytes, reads v1 as Off/Off and retains the existing file/lock/atomic-write adapter.
Restart never automatically plays. Detected boundaries are recomputed after restart.

## Key Files

- `crates/resolibre-core/src/playback_edges.rs`: policy, interval and short-track ramps.
- `crates/resolibre-audio/src/edge_analysis/`, `original_audio/`: job and shared PCM decoder.
- `crates/resolibre-audio/src/playback_edges.rs`, `session/edges.rs`: native/session ownership.
- `apps/desktop/src/playback_edges.rs`, `session_store/format.rs`: bindings and migration.
- `apps/desktop/ui/components/{controls,instruments}.slint`: shared keys/seek overlays.

## Tests

Core boundary/cycle tests; streaming malformed/NaN/cancellation/antiphase tests;
real-session trim/seek/scan/queue regression; native PCM capture via
`python3 tools/check_playback_edges.py`; UI actions, full timeline, native palette
renders at 1x/2x; v1 migration, all four persisted modes and invalid byte rejection.
Executed: 102 workspace/all-target tests, 20 native window cases, eight PCM
captures (including FLAC/MP3), Clippy, build, formatter and docs checks.
[Rendered controls and yellow regions](../../design/player-processing.png).
Live-toggle regression: `python3 tools/check_live_edges.py` reproduced discontinuous
output before the fix. Afterwards ten live and five paused TRIM/FADE changes each
on WAV/FLAC retained source continuity through an isolated PipeWire/Pulse monitor.
Runtime fade-out activation/removal is measured on a separate sine fixture.
Cold-start session regression covers playback during pending analysis, the later
seek-bar correction and pause intent.
The closed test protocol also checks processing completed, rejects malformed settings
and preserves original bytes. Session tests cover live pending analysis and pause
intent; the eight existing PCM captures retain fade/seek and encoded-input coverage.
Full process RSS acceptance, physical listening, hostile-codec fuzzing and device timing remain
outside these digital fixtures. Native decoder/kernel teardown may still block.

## Edge Cases

A low threshold preserves quiet music conservatively, but cannot distinguish every
intentional quiet introduction from padding. Cache metadata cannot prove immunity
to concurrent in-place writes. Generation ownership cancels stale jobs on track
replacement; failed detection never removes a track or bypasses queue limits.
No source writes, external credentials, remote commands or new Rust dependencies.
FFmpeg with fd protocol and float WAV support is now a runtime processing dependency;
ordinary playback remains available without it. Future playback features must use
this interval by default; an explicit separate analysis path is the exception.

## Related Docs

[Processing target](../target/audio-processing.md), [transport](transport-scan.md),
[session persistence](session-persistence.md), [EQ](local-equalizer.md).

Native semantics: [mpv source-time bounds](https://mpv.io/manual/stable/#options-end),
[FFmpeg fades](https://ffmpeg.org/ffmpeg-filters.html#afade).
