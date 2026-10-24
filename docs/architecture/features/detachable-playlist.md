# Detachable Library

## Read When

Changing Library hosts, native window lifecycle or magnetic placement.
Follow the [desktop map](../desktop-project-map.md) and desktop/root rules.

## Do Not Read For

Queue traversal, EQ, persistence or future named playlists.

## Current Behavior

Library is the session track list; `playlist_*` names remain internal. Its host shares the session. Library and EQ default to visible. LIB toggles Library; its thin top inset and passive margins/footer drag, with no title or X.
EQ collapses without bypassing DSP. Main Close shuts down owned hosts and audio/storage.

X11/XWayland hosts share the main player's ICCCM leader/group. Only main appears
in Alt+Tab/taskbar/pager; auxiliary hosts request EWMH/KDE exclusion flags. The
registry owns setup/remapping, deferring native access until window events finish.
Focus/click raises visible peers directly below the selected host, retaining keyboard
focus. Library/Themes share `window_group.rs`; future hosts register there.
Hidden/minimized peers stay put. Eight 16 ms checks cover WM click-to-raise ordering,
reject stale activation, then stop. No focus stealing or always-on-top state.
Native Wayland leaves stacking/switcher grouping to the compositor.

`PlaylistPanel` shares [queue gestures](playlist-interactions.md) and actions.
The footer holds Open/track count; errors/import progress sit above the queue.
Selection and picker/close state follow main. Hide preserves the host, scroll and
X11 geometry; show refreshes its model. Wayland recreates hidden native surfaces.
Visibility persists; placement is per-run.

Startup restores intent while hidden, initializes both native surfaces and warms
Library's frame before showing them in one UI turn. The X11 pair is centered with
DPI-aware sizes and explicit position hints before mapping. EQ starts at its saved
height without a drawer animation. Remapping supplies saved coordinates before
showing; a bounded 30-tick correction covers unavailable geometry. Pending placement
cannot overwrite a saved position on rapid hide/show.
Small screens may need resizing. Hosts retain native move/resize and 6 px corners.

The magnetic rule accepts a gap of at most **5 physical pixels**, including slight
overlap, along any of the four adjacent edges. Nearby perpendicular edges align
as well; six pixels is outside the threshold. Either window can snap. Moving the
main carries edge-connected peers, preserving offsets on both axes. Moving Library
independently detaches it; snapping back reattaches it on any side. EQ remains part
of main. X11 uses live decorated extents, zero frameless extents and explicit gravity.

## Key Files

- `apps/desktop/ui/components/playlist-panel.slint`: single reusable queue surface.
- `apps/desktop/ui/player.slint`, `playlist-window.slint`: window hosts.
- `apps/desktop/src/playlist_window.rs`: owned auxiliary component and shared actions.
- `apps/desktop/src/window_snap.rs`: snapping; `window_group.rs`: activation/registration.
- `apps/desktop/src/window_group/membership.rs`: native identity and switcher hints.
- `apps/desktop/src/window_snap/{geometry,movement,drag}.rs`: group state/gestures; `x11.rs`: native moves.

## Tests

Compiled controls exercise default visibility, independent toggles, shared model
updates, scroll, close and edits through the production worker. Geometry checks cover all
sides, 5/6 px, alignment, corners, negative coordinates and integer extremes.
`python3 tools/check_window_snap.py` runs isolated KWin/XWayland geometry checks at
1x/2x, verifies hidden startup geometry/theme/model, and renders the production hosts on X11 and Wayland. It owns its compositor,
process group and temporary profile; it does not alter the user's display.
[Evidence and limits](../../testing/detachable-playlist-validation.md).
Existing queue, persistence and scrub tests remain connected regressions.

## Edge Cases

Only explicitly registered live ReSolibre components are peers: no global window
enumeration, title matching, cross-process manipulation or global input hooks.
Registries hold at most 16 weak surfaces; hidden/minimized/maximized/fullscreen
peers cannot snap. Bounded acknowledgements reject duplicate feedback while newer
moves are pending; suppression expires after 500 ms.
Independent moves detach immediately. Checked offsets cannot wrap coordinates.
Attached main passive-surface gestures move the captured group directly from total physical
pointer displacement, avoiding the compositor snapping main to its following peers.
Library losing focus as the player activates does not cancel that press.
Release, Escape, player focus loss, resize or hidden peers end the gesture. Only the owned
window is queried during its press; no global input hooks. Detached/native-Wayland
hosts retain toolkit drag. Other moves use batched live geometry.

X11/XWayland is selected when DISPLAY exists, enabling the requested default dock
and snap. `--native-display` restores toolkit selection; `--x11` explicitly selects
compatibility mode. Native Wayland cannot expose global positions through winit;
it keeps windows without docking/snap or explicit group raising. No desktop settings change.
Slint's pinned `unstable-winit-030`, direct `x11rb` 0.13.2 and `raw-window-handle`
0.6.2 reuse existing locked packages. Native API use stays in the window adapter.

Activation uses EWMH restacking with application source, only for owned handles;
the current active-window property guards delayed focus. Display calls lack hard cancellation.
Physical dragging was user-confirmed on KDE Wayland/XWayland at 125%;
accessibility, mixed-monitor DPI and resource gates remain manual.

## Related Docs

[Local player](local-desktop-player.md), [design](../../design/compact-player.md),
[interface target](../target/interface-and-themes.md),
[security](../target/security-and-privacy.md), [tests](../../testing/test-plan.md).
Native limitation: [winit window geometry](https://docs.rs/winit/0.30.13/winit/window/struct.Window.html#method.outer_position).

Contracts: [EWMH](https://specifications.freedesktop.org/wm/latest-single/), [KDE NET states](https://api.kde.org/net.html).
