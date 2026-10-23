# Local file drops

## Read When

Changing native drops, folder expansion, Add/Replace or import lifecycle.
Use [desktop](../desktop-project-map.md) and [core](../core-project-map.md) rules.

## Do Not Read For

Catalog indexing, metadata, SMB, M3U or saved named playlists.

## Current Behavior

Drop local files/folders on Player or Library. Native per-file events form one
bounded batch after 100 ms without another drop. A worker recursively expands
folders; roots retain drop order, children use lexical path order. The shared
chooser/scan list comes from `resolibre_audio::media_formats`: audio and video
containers, including MP4/M4A, MKV and WebM. Only audio plays. Extensions select
candidates; decoding validates content. Unrelated files are ignored silently.

For a nonempty queue, the receiving window shows Add / Replace:
- Add appends without changing current audio.
- Replace atomically installs the batch, stops audio and selects its first entry.
- Escape or an outside click cancels. A repeated choice has no effect.
- An empty queue adds directly, without prompting or autoplay.

The menu is clamped inside its window near the last native pointer position
(fallback near the header). Hiding Library cancels its menu; a scan finishing
after it is hidden presents the choice in Player. Playback controls remain live
during scanning. Open waits while a drop is pending; additional drops while
scanning/choosing are rejected with feedback. Startup restore precedes imports.
Close cancels scanning, requests audio shutdown and polls cleanup before exiting.

All consumers use the same session Add/Replace commands. Replacement retains
volume, EQ, Shuffle and Repeat, resets traversal/timing and never reuses occurrence
IDs. Invalid/oversized replacement preserves the existing queue/audio.
Session persistence saves the resulting queue through its existing owner.

## Key Files

- `apps/desktop/src/file_import.rs`: bounded inbox, worker ownership and native listener.
- `file_import/scan.rs`: read-only expansion; `tests.rs`, `native_tests.rs`: checks.
- `crates/resolibre-audio/src/media_formats.rs`: shared candidates and native allowlist.
- `presentation.rs`: confirmed session/menu state; `playlist_window.rs`: second host.
- `ui/components/import-menu.slint`: shared contextual decision surface.
- `crates/resolibre-core/src/queue.rs`: atomic replacement and occurrence identity.
- `crates/resolibre-audio/src/session/`: command validation and stopped replacement.

## Tests

Compiled UI checks cover empty/Add/Replace/cancel, duplicate choices, stale IDs,
playback preservation and both hosts. Scanner checks cover order, Unicode/case,
symlinks, unsupported/missing/relative paths, cancellation and limits.
Core/audio tests cover atomic rejection, modes/settings and fresh IDs.
`tools/check_window_snap.py` includes real XDND selection delivery to owned X11
windows, multi-file/folder batches and a menu capture, alongside snap regressions.
It uses private display/profile state and generated WAV fixtures, never global input.
Executed results: [validation](../../testing/local-file-import-validation.md).

## Edge Cases

Queue/input bounds: 1024 files, 1 MiB path bytes, 4096 bytes/path. Traversal stops
at 16,384 entries, 8 MiB cumulative path bytes, depth 32 or ten seconds between I/O
operations. Failure rejects the entire batch. Empty/non-audio folders leave the
queue unchanged without a menu or notice. Unsupported entries, symlinks and special
files are ignored silently; unreadable entries fail the batch. No URL/playlist expansion or music writes.

One worker/batch is retained. Filesystem calls cannot be forcibly interrupted;
close waits for an in-flight call. Concurrent filesystem changes can invalidate
results before playback. No decoder or metadata work runs during expansion.
Errors omit private paths and raw OS diagnostics; no new dependency or network API.

The pinned winit backend delivers drops on X11/XWayland (normal startup), not
native Wayland. Open remains available with `--native-display`. Physical Dolphin
dragging, alternate window managers and sustained resource measurements remain
manual acceptance; protocol tests do not claim them.

## Related Docs

[Import target](../target/local-file-import.md), [local player](local-desktop-player.md), [order](queue-playback-order.md),
[Library](detachable-playlist.md), [restore](session-persistence.md).

Native contracts: [winit events](https://docs.rs/winit/0.30.13/winit/event/enum.WindowEvent.html),
[Slint event hook](https://docs.slint.dev/latest/docs/rust/i_slint_backend_winit/trait.WinitWindowAccessor).
