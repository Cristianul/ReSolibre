# Local desktop player

## Read When

Slint/session/queue or file selection: [desktop](../desktop-project-map.md),
[core](../core-project-map.md).

## Do Not Read For

For EQ use [its feature](local-equalizer.md); catalog/SMB use the
[roadmap](../../plans/product-roadmap.md).

## Current Behavior

Linux/Windows x86_64: Slint 1.18.1/FemtoVG and libmpv. Open appends/plays the first addition;
cancel preserves playback. Duplicates have distinct IDs.
Navigation/EOF use [queue modes](queue-playback-order.md); [hold scans](transport-scan.md) run at 3×.
Failures retain selection for retry/navigation. Stop releases the engine; Play
restarts it. [Session settings persist](session-persistence.md) without autoplay.

[Gestures](playlist-interactions.md): click selects.
Shared `QueueAction` uses occurrence IDs; stale IDs are ignored. Reordering preserves
identity, time and pause. Removing another row preserves audio; removing current
stops and selects its successor (predecessor at end); removing the last returns idle.
[Drops](local-file-import.md) support Add/Replace and folders. No source writes.

One worker owns each non-Send engine. Mailbox: 32 commands plus a scan-release slot, one pending selection,
one latest snapshot. Adjacent seeks/volume coalesce; Stop clears pending work;
shutdown has priority. Native events are serviced between requests. Seek gestures
retain their initial generation until release/cancel, rejecting edits to later tracks.
Native work stays outside locks/UI.

Volume: 50% default / 25% amplitude, squared curve, 0 silence, 100 unity; no OS mixer.
Short stationary dial clicks target the exact angle on its 270° arc and glide for
2 s with smoothstep easing. Holds >=1 s, cancellation, the center and points outside
the circle do nothing; movement >4 logical pixels retains two-axis manual dragging.
The bottom gap selects the nearest endpoint. Double click resets volume to 50%.
Manual/keyboard/reset controls cancel glides; another click retargets from current
volume. Worker ticks update audio and display even while idle/paused; persistence
stores the destination. `volume_glide.rs` owns the bounded timing policy.
The adapter inverts mpv's cubic gain; its 120 ms/full-scale slew guard remains.
Pre-load gain/pause avoid bursts. Double click on seek returns to the start.
Seek capability is separate from unknown/estimated duration: UI slider is bounded
and disabled without duration; absolute commands never clamp to that estimate.

[Appearance](player-appearance.md) and [layouts](../../design/compact-player.md): an open
[EQ](local-equalizer.md) drawer and [separate Library](detachable-playlist.md); LIB/EQ toggle independently.
[Tags/covers](track-metadata.md) load asynchronously. Text is bounded/sanitized;
paths stay intact. UI time observes the engine, not physical output.

## Key Files

- `crates/resolibre-core/src/queue.rs`: reusable policy/actions; `session.rs`: gain/state.
- `crates/resolibre-audio/src/session/`: mailbox/worker; `controls.rs`: native timing/gain.
- `apps/desktop/src/`: composition, bindings, owned chooser process.
- `apps/desktop/ui/`: controls/tokens/Lucide assets; `components/queue-row.slint`: row.

## Tests

Tests: bounds, duplicate IDs, edits, pause/seek, late EOF seeks, failures, stale
commands, cleanup, accessible controls, independent surfaces and native edits.
Three-track held-scrub tests use generated WAV or explicit MP3/FLAC fixtures.
Angle/hold/drag/reset tests, deterministic easing and worker retarget/cancel/idle
tests cover volume; close/restart checks retain a glide destination.
PCM checks mute/midpoint/unity; marker harness `--session-worker` checks production
output. [Evidence](../../testing/local-desktop-validation.md),
[commands](../../development/desktop-environment.md).

## Edge Cases

Queue: 1024 occurrences / 1 MiB raw paths / 4096 bytes each; rejected batches
preserve playback. Titles: 160 characters. Chooser frames use a private inherited
socket, <=1 MiB plus framing, without shell/private path arguments or diagnostics.
One helper: cancelled/reaped on close, ten-minute timeout; no remote listener.
Linux re-executes `/proc/self/exe`; unlink/replacement is regression-tested.

UI refresh: 33 ms; each continuous load/reconfiguration wait is bounded to ten seconds.
Seek/EOF keeps draining events; elapsed track age cannot expire a new wait.
Close waits for release, warns after five seconds and stays responsive; no detached thread. Native/kernel calls have
no hard cancellation bound; resolve before broader sources. Gapless, release
accessibility, physical latency and classic skins remain unverified.

## Related Docs

[Adapter](local-playback.md), [security](../target/security-and-privacy.md),
[tests](../../testing/test-plan.md), [contracts](../target/core-and-platforms.md).
