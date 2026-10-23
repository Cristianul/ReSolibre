# Queue playback order

## Read When

Changing Shuffle, Repeat, navigation/history or their session/UI bindings. Use
[core](../core-project-map.md) and [desktop](../desktop-project-map.md) guidance.

## Do Not Read For

Persistent playlists, gapless, crossfade, EQ, source access or global hotkeys.
The [local player](local-desktop-player.md) owns native lifecycle/input bounds.

## Current Behavior

Core owns traversal and the Back threshold. Shuffle/Repeat default to Off; [settings persist](session-persistence.md). UI mirrors worker
availability. Mode changes preserve playback, pause, gain and displayed order.

Back restarts the current track from one playable second onward; before that it
navigates to the previous occurrence. TRIM defines the playable start. Restart
preserves pause, EQ, generation and history, and works on the first/lone track.
Unknown/nonseekable timing uses traversal; absent predecessors are no-ops.
Rapid Back uses restart intent until timing catches up (at most 1 s).
Back's yellow LED follows this policy: on only when Back can navigate to a predecessor.

| Mode | Normal EOF | Manual Next / Previous |
| --- | --- | --- |
| Off | Advance until pass ends | Advance within order/history |
| All | Wrap / start a fresh shuffled pass | Next wraps; ordered Previous wraps |
| One | Reopen current occurrence | Navigate normally, without All wrapping |

Shuffle visits occurrence IDs once per automatic pass; duplicate source files
remain separate entries. Previous follows actual selections, and Next first
replays forward history. History retains at most 1024 visits.
Shuffle Previous never invents a predecessor at the retained beginning. Repeat
All shuffles a full new pass, avoiding an immediate boundary repeat when possible.
Explicit selection/history replay and Repeat One may intentionally revisit items.

Selection branches history and consumes its pending ID. Append adds pending IDs;
reorder preserves shuffle. `MoveTo` validates both IDs and retains current audio;
[gestures](playlist-interactions.md) commit on release. Removal prunes history/work;
removing current stops audio and anchors traversal at the fallback selection.
Shuffle toggles reset traversal; Repeat and Stop/replay retain it. Open still
starts the first newly selected file; Play on an unselected Add-only queue starts
according to the mode. Errors halt; they never trigger automatic retries/skips.

## Key Files

- `crates/resolibre-core/src/queue.rs`: policy facade, shared actions and edits.
- `crates/resolibre-core/src/queue/order.rs`: bounded pending/history and modes.
- `crates/resolibre-audio/src/session/`: seed, commands, EOF and snapshots.
- `apps/desktop/src/presentation.rs`, `ui/player.slint`: bindings and mode controls.

## Tests

Seeded core tests cover permutations, repeat, history/branching, edits, empty/single
queues and 5000-cycle history bounds. Native session tests verify normal EOF loops,
Shuffle exhaustion, paused history, unchanged mode-toggle generation/time, Stop/
replay and failure stability. Back tests cover the exact threshold, invalid timing,
restart/history, rapid presses, first track, pause and TRIM-relative timing. UI tests verify callbacks/state;
`native_modes_journey` exercises production bindings. See the
[execution record](../../plans/implemented/shuffle-repeat-plan.md) for checks and limits.

## Edge Cases

Pending work is bounded by the 1024-occurrence queue. IDs are session-scoped and
never path-derived; stale IDs cannot target replacement rows. Core accepts a seed
for reproducible tests. The native worker supplies a fresh `RandomState` hash seed;
the deterministic generator is for music ordering only, never cryptography.
No new dependencies, source writes, networking, persistent listening history or
private diagnostics. Native blocking-call limits remain those of the local player.

## Related Docs

[Local player](local-desktop-player.md), [audio target](../target/audio-and-equalizer.md),
[roadmap](../../plans/product-roadmap.md), [test plan](../../testing/test-plan.md).
