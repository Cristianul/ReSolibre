# MIX transitions

## Read When

Changing the MIX key, next-track rhythm preparation, overlap, tempo ramps or
transition-only intro/outro cuts. Use the `dj-mix` reading profile.

## Do Not Read For

[TRIM/FADE](playback-edges.md), [spectrograms](track-analysis.md), loudness
normalization, library analysis or a replacement playback engine.

## Current Behavior

MIX follows FADE, defaults off and persists as a bounded v4 boolean.
Versions v1–v3 migrate with MIX off; restoration never opens audio. PREP means preparing, READY a prepared
tempo/beat plan, FADE… a prepared ordinary overlap, FADE an unadjusted overlap,
TEMPO matched rates without verified phase, SYNC measured
clock alignment within 30 ms (a confirmed non-unity ratio displays `1:2`, `1:3`,
`2:3` or its inverse), and a dash an unavailable transition. Timing is approximate.

Current BPM starts asynchronously from a bounded 64-second excerpt.
Only MIX analyzes the next occurrence in the queue's shuffle/repeat order. Current rhythm (excerpt then full timeline, even with MIX off)
precedes next-track work and requested plots. No spectrogram is constructed.
One cancellable worker, two-recording cache and ten-second job deadline remain. Disabling MIX cancels lookahead work; current BPM remains available.
File/stream identity is rechecked against the retained source.

Overlap targets 10% of the shorter track (24 seconds for four-minute songs).
Short clips use a four-second floor capped at one quarter of either duration.
Outgoing gain falls as incoming gain rises.
Beat matching rounds to whole intervals and accepts at least nine observed attacks
spanning the overlap, regularity >=0.55 and phase errors <=25 ms. Up to three missed
attacks between observations are allowed; no beat timestamps are fabricated.
Adjacent windows may join when tempo and observed phase agree. MIX may omit a beat-free
intro/outro, limited to 16 seconds and 10% of that track's effective duration,
whichever is smaller. It affects only the outgoing end and incoming beginning;
MIX preserves the first intro, final ending and internal pauses.
Ordinary TRIM intent remains independent. No music file is changed.
While a new track plays during pending TRIM measurement, MIX waits for its
confirmed outgoing bounds before building a transition plan.

Prefer equal BPM within ±20% on both streams. Otherwise choose among 1:2, 1:3,
2:3 and inverses, minimizing the largest rate adjustment. Complete shared cycles
align; source BPM stays unchanged. Speed is never clamped to an off-grid limit.
Ratio overlaps round down to whole cycles; both sides still need nine observations.
Nominal rates preserve the shared ratio. Missing phase permits TEMPO;
unsupported ratios retain crossfade.
Pitch correction uses libmpv's Rubber Band R3 with separate stereo channels and
48 kHz internal MIX processing. Source files and normal playback are unchanged.
Startup padding/output trimming preserves the first samples; a persistent 1 ppm
pitch guard (<0.002 cents) prevents a phase reset at exact 1× speed.
The graph survives handover/unity and is removed during paused scan reconstruction.
Both tracks reach the geometric BPM midpoint before audible overlap. The outgoing
six-source-second approach ends one second early for buffered audio to settle;
the incoming stream starts muted at that midpoint (120→142 meets at 130.54 BPM).
Both speeds share a six-second smooth return starting about one second before handover.
The incoming stream continues afterward; handover never resets speed.
`TEMPO_MEET_DURATION_SECONDS`/`TEMPO_MEET_SETTLE_SECONDS` tune approach/settling.
`TEMPO_RETURN_LEAD_SECONDS` and `TEMPO_RETURN_DURATION_SECONDS` in
`transition/tempo_return.rs` tune lead and total playing time. Short overlaps cap
the lead at half their span. Source-clock integration freezes on pause/buffering;
seek, navigation, scan and MIX-off cancel it. Generations reject stale returns.
TEMPO also covers this solo return; no phase alignment is claimed after handover. Missing phase permits BPM:
covered local estimates can drive TEMPO; a fully analyzed stable track needs two
agreeing independent windows to bridge uncovered regions. Variable gaps, conflicting
estimates and unsupported ratios retain the ordinary crossfade without SYNC.
Tracks allowing less than 0.5 seconds of overlap retain ordinary queue playback.
Fallback keeps supported MIX cuts even with TRIM off; it never invents beats.
Joining edges use observed beat groups and per-channel peak activity measured in
the existing rhythm pass: 5 ms bins above -40 dBFS, at least 100 ms total activity,
with 50 ms sound padding. Quiet or beat-free edges share the same 16-second/10%
cut budget. A prefix cannot authorize a longer track's tail cut; silence or isolated
clicks alone supply no activity bounds. No additional decode job is needed.

One muted, paused libmpv handle opens 23 seconds before the earliest overlap/cut.
Available lead-in is capped at 300 ms. Up to three seeks correct startup latency
while incoming gain stays zero and outgoing gain stays full. Alignment has one
second after overlap start, capped before handover; failure preserves current playback.
Verified alignment releases complementary gains over 150 ms; 2% speed feedback
centers drift within the 20% limit. Seeking/settling clocks cannot verify SYNC;
error must be <=30 ms. Missing audio clocks retain playback-time TEMPO fallback.
Complementary linear gains bound correlated mix peaks;
application volume and EQ remain shared intent. Boundary FADE is suppressed during
overlap and restored when ordinary playback resumes.

The original-time seek bar marks confirmed overlap with orange boundary ticks and hatch;
invalid or pending plans show none. Pre-overlap seeks retain muted preparation.
Native seek reconstruction finishes first; audible-overlap seeks cancel MIX and
restore speed/gain before moving. Pause affects both streams. The Library marks
both audible occurrences; the title switches quickly (instantly with reduced
motion). Next focuses incoming without audio handover. Artwork, stream facts,
analysis action and seek bar follow it. The incoming bar marks its overlap near
the start; `MIX ≈m:ss` beside the clock estimates remaining outgoing source time.
Seeking this bar adopts incoming; Previous returns focus to outgoing. Stale seeks
cannot affect later tracks. Other navigation and queue edits cancel MIX.
MIX-off restores speed/gain/edges over 150 ms, including the outgoing approach.
Next-source failure preserves playback, logging one redacted stage/error warning.
Handover advances the queue once and adopts the running stream without replaying its intro. No scripts, external references, system-mix capture or new decoder are introduced.

## Key Files

- `crates/resolibre-core/src/analysis/activity.rs`: bounded original-audio sound edges.
- `crates/resolibre-core/src/transition{.rs,/ratio.rs,/tempo_return.rs}`: policy and tuning.
- `crates/resolibre-audio/src/session/worker/mixing{.rs,/phase.rs}`: clocks and handover.
- `crates/resolibre-audio/src/controls.rs`: scoped native tempo filter and speed.
- `crates/resolibre-audio/src/session/rhythm.rs`, `rhythm_analysis.rs`: gated lookahead.
- `apps/desktop/src/playback_edges.rs`, `ui/player.slint`: shared key and status.
- `apps/desktop/src/session_store/format.rs`: strict v4 migration.
- `tools/check_mix.py`, `tools/check_mix_pitch.py`, `crates/resolibre-audio/src/mix_capture_tests.rs`: isolated PCM proof.

## Tests

Checks cover bounds, missing attacks, invalid clocks, startup latency/drift, gains,
variable BPM, shared midpoint and six-second return. Session checks cover gated lookahead, changed sources, natural EOF with
TRIM/shuffle, pause/handover and preserved originals. UI checks dispatch the shared
key and distinguish beat/crossfade readiness. Paused seeks retain readiness;
seeking out of overlap rearms without advancing. Invalid markers are rejected;
Persistence checks migration, MIX round-trip and malformed payloads.
Muted native/Windows clock checks cover 420 ms startup lag and preserved sources;
they do not measure audible attacks.

[Native audio validation](../../testing/mix-validation.md), profile
`dj-mix-validation`, documents private PipeWire/Pulse captures, midpoint/beat
timing and sustained-pitch regression checks. No desktop audio is captured.
The 55/440 Hz regression covers both ramps and unity settling, with negative
controls for pitch shifts, phase jumps, flutter, silence and dropouts. It also checks
invalid speed rejection, original preservation and external-config isolation.

## Edge Cases

Native clocks are approximate. Device timing, listening quality, variable-tempo
phrases and broad CPU/RAM acceptance remain open. MIX captures reached 112 MiB;
the 100 MiB audio/200 MiB desktop budgets are unchanged. Linux native execution does not validate Windows,
Android or iOS; policies stay in portable core, native scheduling stays in adapters.
Ratios follow measured BPM: half/double detector ambiguity remains, including a
variable result on the 224 BPM fixture. Unknown evidence cannot fabricate sync.

## Related Docs

[Processing target](../target/audio-processing.md), [analysis](track-analysis.md),
[queue order](queue-playback-order.md), [persistence](session-persistence.md),
[test plan](../../testing/test-plan.md),
[Rubber Band integration](https://breakfastquay.com/rubberband/integration.html).
