# Local desktop player implementation plan

Status: controls, native session and temporary queue implemented, 2026-10-22.
Acceptance remains active for the gaps below; this is not a release-support claim.
The [validation record](../testing/local-desktop-validation.md) owns measurements
and failures. The [feature](../architecture/features/local-desktop-player.md) owns
current behavior and bounds; the [design](../design/compact-player.md) preserves the
approved image and independent PL/EQ surfaces.

## Delivered scope

Linux Slint window over the existing libmpv adapter: Open files, Play/Pause, Stop,
time, seek, app volume, editable temporary queue, Add, selection, Next/Previous
and EOF advance. Queue rows now support removal and adjacent reordering. Separate Playlist and fixed [EQ](../architecture/features/local-equalizer.md) drawer
start visible; PL/EQ hide/show them without reconstructing the session.

Shared controls, tokens and licensed Lucide assets implement the graphite/green
reference. Portable queue/volume policy remains separate from native ownership
and presentation. A bounded mailbox and one latest snapshot connect the UI to one
native worker. The non-Send engine is created, used and destroyed on that worker.
Stop/shutdown have priority; stale generations cannot seek another occurrence.
The startup allocator policy runs before toolkit/audio threads.

Open appends and starts the first new item; Add leaves playback uninterrupted.
Next/Previous preserve pause. [Shuffle/Repeat](../architecture/features/queue-playback-order.md)
extends the original ordered navigation. Errors stop and
retain selection for retry/navigation. Stop releases the engine and resets time;
Play reopens the selection. [Later session persistence](../architecture/features/session-persistence.md) restores intent without autoplay.

Volume starts at 50% slider / 25% amplitude, reaches silence at 0 and unity at 100,
and ramps changes. It never changes the OS mixer. Configuration before load avoids
initial full-volume audio. Unknown duration stays unknown; the UI disables seek
without duration/capability. Typed seeks never clamp to a growing duration estimate.

No catalog, persistent playlists, M3U import, tag/art decoding or editing, SMB,
normalization, gapless, crossfade, tray or media keys in this slice.
These remain local-first phases in the [roadmap](product-roadmap.md).

## Validation contract

Retain core-only compilation and original CLI behavior. Test state transitions,
paused navigation/seeks, duplicate occurrences, boundaries, EOF, missing/corrupt
files, fresh recovery, stale requests, queue floods and repeated stop/shutdown.
Verify actual output markers, not only acknowledgements or a moving slider.

Use generated WAV/FLAC markers and the two user-selected MP3/FLAC originals.
Originals remain read-only; compare hash, size, mtime and mode. Keep music and
private paths/names out of Git and reports. Inject faults only into owned fixtures.
Test picker cancel/framing, nonregular/FIFO paths, hostile native config, malformed
media, external references, bounded text/paths, invalid seek/volume, output loss
and close during native work. Review logs, dependencies and process ownership.

Run workspace/core tests, compiled/native desktop journeys, rustfmt/Clippy,
CLI privacy, PCM/output-failure/control and Python regressions. Record source
preservation, actual commands, visual inspection, measured CPU/RSS and control
latency; distinguish virtual-monitor bounds from physical output/listening.

User approved 200 MiB whole-desktop RSS on this machine for the first version,
2026-10-22, after an initial ~174 MiB measurement. Audio-only remains 100 MiB;
UI acknowledgement p95 remains <=50 ms. Keep the existing audio comparison gates.
These reference-machine budgets are not universal hardware guarantees.

## Remaining acceptance and boundaries

Native filesystem/open/destruction calls cannot be forcibly cancelled by a thread.
The UI stays responsive and visibly waits for cleanup rather than detaching it;
measured local fixtures must close within five seconds. A universal deadline for
stalled native/kernel operations is unresolved and must precede broader sources.

Complete manual keyboard/focus, DPI/resize, portal-cancel and user listening checks;
full accessibility is a later gate; skin compatibility moves near the roadmap end. Record gaps
explicitly. Do not infer listening quality from silent-output or virtual-monitor
checks. Keep this plan active until the required local acceptance checks above
are recorded; then move it to implemented with final evidence and updated links.
SMB stays deferred until everyday local playback/features/catalog gates
and the Jellyfin phase in the [roadmap](product-roadmap.md).

## Reading routes

Use `local-desktop-player-plan` for this plan/reference; `local-desktop-ui` for
current session/presentation behavior; `local-desktop-validation` for evidence;
`desktop-environment` for setup. Processing changes use `audio-processing` and
native validation uses `playback-responsiveness-validation`. Complete deduplicated
flows, including all applicable rules, must fit within 32 KiB.
