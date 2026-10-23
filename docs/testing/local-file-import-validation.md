# Local file import validation

Read with [local file drops](../architecture/features/local-file-import.md).
Linux x86_64, debug, pinned Slint/winit and libmpv.

## Checks

- Workspace/all targets: 76 tests pass; 16 opt-in tests excluded.
- Includes 32 desktop tests: shared menus/actions, empty queue, Add during playback,
  Replace, Escape/outside cancellation, hidden-host fallback and rejected batches.
- Scanner negatives cover missing/relative paths, symlink loops, empty folders,
  cancellation, recursive queue/depth limits and non-UTF-8 names.
- Core/native session checks cover atomic replacement, stale occurrence IDs,
  stopped timing reset and preserved volume/EQ/Shuffle/Repeat.
- Native harness: all 13 isolated cases pass. XDND supplies folders, Unicode/spaced
  paths and multiple files through the production hook/worker; snap and UI
  regressions pass too. [Inspected menu](../design/player-drop-menu.png).
- Workspace Clippy with warnings denied, desktop build, formatter and docs checks pass.

Earlier logs: `.local/drop-final-*.log`; XDND: `.local/window-snap/drop.log`.

The outside-click regression initially exposed a full-window FocusScope swallowing
the press. Its bounds now match the menu card; the cancellation test passes.
Generated inputs and private profiles preserve user music and session data.

## Decisions and limits

Add and empty-queue import do not autoplay; Replace stops and selects the first
new occurrence. Recursive folder order is deterministic. Unsupported files and
links are skipped; errors/limits reject the batch rather than partially replacing
a playlist. Existing queue limits also apply to import. These are local queue
operations, not catalog scanning or M3U import.

Native XDND checks own both sender and target windows and the private selection.
They do not inject global mouse input. Physical Dolphin dragging and native Wayland
drop support are not claimed. No new memory benchmark was run.

## Silent filtering and audio from video

84 workspace/all-target tests and 14 native cases pass (17 opt-in excluded). Mixed selections ignore
images silently; unrelated-only and empty scans leave playback/queue unchanged
without a menu or notice. Chooser and scanner use the audio adapter's candidate
list. Real libmpv checks decode generated MP4/MKV/WebM/M4A audio to EOF, assert
video output/parameters stay absent, reject video-only input and preserve bytes
and modification times. Fixtures contain generated blue frames and a sine tone;
`media_tests.rs` records generation settings. Other admitted codecs depend on the
native build and are not individually certified by these fixtures.

CLI probes confirm no network access from disguised M3U/HLS/concat inputs; nested protocols, scripts and
sidecars stay disabled. Read failures and bounded traversal errors remain visible.
EOF, queue, seek, EQ, persistence and import regressions pass. No physical listening
or new memory benchmark was performed. Logs: `.local/media-import-*.log`.
