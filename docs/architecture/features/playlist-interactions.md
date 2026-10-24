# Library interactions

## Read When

Changing queue row input, selection, drag order, current-track reveal or its marker.
Use root, [desktop](../desktop-project-map.md) and [core](../core-project-map.md) rules.

## Do Not Read For

Window docking/stacking: [detachable Library](detachable-playlist.md).
Importing files: [file drops](local-file-import.md). Traversal: [order](queue-playback-order.md).

## Current Behavior

Single click selects a row without audio commands. Double click or Enter plays the
selected occurrence through the existing session action. Selection belongs to the
view and uses string occurrence IDs; current audio identity remains worker-owned.
Automatic track changes do not move the user's browsing selection or scroll.

Only the playing row inserts a musical note between its number and title; other
rows and pause/stop reserve no marker space. It sways in the chosen palette unless
motion is disabled. Analysis status sits immediately left of the fixed duration.
Hidden windows stop animation; virtualized rows avoid timers for the entire queue. Set
`RESOUL_REDUCED_MOTION=1` before launch to disable UI motion. Automatic OS
reduced-motion preference detection is not implemented.

Overflowing titles use shared `ScrollingText`: Library hover, selection or active
playback starts a slow back-and-forth scroll; the main title scrolls automatically.
Each end pauses for one second; smoothstep travel averages 24 logical px/s.
Short/idle titles stay still, with ellipsis when truncated. Row numbers and durations
remain fixed. New text/width resets the cycle. Hidden/minimized/reduced-motion
hosts, offscreen rows and reorder drags suspend motion; only active overflow reads
the frame clock. Text remains complete for accessibility and main-title clicks
retain the existing Library reveal action.

Rows have no inline move/delete buttons. Right click selects the row and opens
**Analyze** and **Delete**; Escape/outside click dismisses it.
[Analysis](library-analysis.md) runs without starting playback. Delete key uses the same removal
action. This removes only the queue occurrence, never the source file. Removing
the current entry retains the existing stop/fallback behavior.

Dragging beyond 6 logical pixels displays an insertion line. Edge autoscroll runs
only during a drag. Release inside the list submits one `QueueAction::MoveTo` with
source/target occurrence IDs and before/after placement. No intermediate edits are
submitted. Escape, capture cancellation, focus loss, hiding, closing or model
replacement cancels the preview. A release outside the list does not reorder.
Core validates both IDs, rejects self/stale/no-op moves, and preserves playback
identity, pause, time, shuffle history and repeat. Existing session persistence
stores the confirmed order.

Clicking the main player's track title opens/unminimizes and requests focus for
Library, clears [search](library-search.md), selects current and scrolls it into view. Empty queues still open safely.
Dragging the title past 6 logical pixels moves the player instead and suppresses
the reveal click; the incoming MIX title uses the same gesture.
Arrow keys browse without playback; Enter activates. Accessible row default action
selects; the list retains keyboard playback/removal actions.

## Key Files

- `ui/components/queue-view.slint`: selection, capture, menu, autoscroll and reveal.
- `ui/components/queue-row.slint`: lightweight virtualized row/marker.
- `ui/components/scrolling-text.slint`: shared measured/clipped title animation.
- `ui/components/playlist-panel.slint`: view/footer composition.
- `src/playlist_window.rs`: creation, stable-ID lookup, focus and shared actions.
- `src/presentation.rs`: opaque IDs to typed actions.
- `crates/resolibre-core/src/queue.rs`: atomic relative move policy.
- `crates/resolibre-audio/src/session/worker.rs`: move without engine reload.

`src/scrolling_text_tests.rs` checks overflow, activation, reset, reduced motion
and the unchanged reveal action.

UI/src paths above are relative to `apps/desktop/`.

## Tests

`playlist_window/interaction_tests.rs` exercises actual compiled pointer recognition:
single/double click, duplicate titles/large IDs, Escape, drag, contextual deletion,
model replacement, disabled input, long-list autoscroll and reveal. Host tests check
hidden/show/reveal selection. Core tests cover direction, no-ops and stale IDs;
native session tests verify paused position/generation and unchanged originals.
`tools/check_window_snap.py` covers real X11/Wayland rendering and native host reveal,
plus connected window, import and playback journeys. FemtoVG snapshots verify
note motion, paused/reduced-motion rest and palette changes; the software renderer
does not implement rotation. Physical pointer/compositor
behavior and assistive technology remain manual acceptance checks.

Verification (2026-10-22): `cargo test --workspace` passed 93 tests (20 opt-in
excluded); `check_window_snap.py` passed 16 native cases. After caption layout
polish, 45 desktop tests and both native Library previews passed again. Workspace
Clippy, build, formatting and documentation checks passed. Initial checks exposed
pre-map placement timing and software-renderer rotation limits; both are covered
by the corrected native tests.

## Edge Cases

Queue bound remains 1,024 entries. IDs are not floating-point values or titles.
Gestures never delete music. Explicit [analysis](library-analysis.md) reads media;
queue edits need no new persistence format, dependency or global input capture. Model replacement cancels captured edits; worker
validation rejects stale neighbour IDs even if they disappear after release.
Title reveal checks placement after mapping; an immediate refresh can incorrectly
accept pre-map coordinates before the WM recenters the window. Only explicit
show/reveal actions request focus; routine playback does not.

## Related Docs

[Desktop player](local-desktop-player.md), [session storage](session-persistence.md),
[library target](../target/library-and-playlists.md), [test plan](../../testing/test-plan.md).
