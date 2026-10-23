# Detachable playlist validation

2026-10-22, Linux x86_64, Rust 1.99 debug, Slint 1.18.1/winit 0.30.13.
Read with the [feature](../architecture/features/detachable-playlist.md).

## Environment and limits

The harness owns its compositor, process group and profile; prerequisites:
KWin/XWayland, dbus-run-session, xprop, xmessage, fusermount3 and native libraries.
Bounds: 120 s overall, 15 s per case; private 2560×1440 desktop fits both hosts at 2×.
Generated music bytes are checked before cleanup.
Other WMs, accessibility and mixed DPI/audio remain manual. Wayland lacks snap/group raising.

## Click raising and docked-move follow-up

Eight 16 ms checks cover asynchronous raising; `xmessage` tests foreign stacking,
focus retention, Themes and hidden peers.
Typed input uses production routing; XTest/XSendEvent failed to deliver XI2 clicks
on the headless seat. Physical clicks remain manual.

A duplicate-observation regression reproduced a follower reverting from 540 to 520.
Bounded acknowledgements preserve newer destinations and expire denied requests.
Forty-sample checks compare batched/serial geometry, not physical drag FPS.

Production uses bounded weak peers; foreign discovery is test-only. The harness
rejects unknown cases and requires one passing test per case. Earlier follow-up:
51 desktop/21 native passed; query p50 improved 75→25 us (1x), 66→22 us (2x),
without claiming physical drag latency.

Switcher check: shared leader/WM hints, one eligible entry, foreign activation,
hide/remap and main minimize/restore passed in isolated KWin. It waits for WM
acknowledgement, not submission; physical Alt+Tab remains manual.

## Premapping and direct group drag

Remap initially failed at (750, 282), expected (80, 140), with the same native ID.
Initial geometry/DPI needed correction. That version waited for main WM state.
ICCCM hints and 20 hide/show cycles verify stable identity/position without refresh.
Geometry-query changes did not fix physical dragging. A bounded diagnostic on the
user's KDE Wayland desktop showed 13 px main moves at 125%, with follower acknowledgements
under 2 ms: KDE's 10-logical-pixel window snap was catching its moving follower.
Direct title-bar group movement bypasses that interactive snap. The user confirmed
fluid motion and group raising; temporary tracing was removed. Pointer displacement
keeps offsets without cumulative rounding; header routing, release, overflow and
native one-pixel forward/reverse movement are regression checks. Detached windows
retain toolkit drag. [KWin snapping](https://github.com/KDE/kwin/blob/master/src/workspace.cpp).

Checks: 53 desktop/27 native; build/Clippy/format/docs passed.

## Prepared startup and Library rows — 2026-10-23

Hidden restore now precedes mapping; native sizes fix the 2× stale-size regression.
Eight native cases passed: prepared startup/theme/model at 1×/2×, drawer animation,
note motion and Library previews on X11/Wayland. The note exists only during play;
analysis sits left of duration. Workspace checks passed 230 tests, with 39 opt-in
cases excluded; desktop/storage were rerun after an executable access failure.

Final debug binary, empty private profile, software rendering: both X11 clients
mapped at 857 ms (1×), 972 ms (2×). Polling measures mapping, not compositor scanout;
reference-machine cold/large-profile acceptance remains open. Idle RSS: 260/279 MiB
X11, 231 MiB Wayland; the 200 MiB reference-machine target is not yet demonstrated.
