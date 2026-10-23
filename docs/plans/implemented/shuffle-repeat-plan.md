# Shuffle and repeat implementation plan

Status: implemented and scoped checks passed, 2026-10-22.
The [base plan](../local-desktop-player-plan.md) retains its manual/native-stall gaps;
this slice does not claim to close those or unlock SMB.

## Contract

Implement reusable queue policy, typed commands/snapshots and desktop controls.
Default Shuffle off, Repeat Off; modes are session-only. Toggling preserves the
current engine, position, pause, volume and visible queue order.

Repeat Off ends after one pass. All wraps ordered playback or starts a fresh
shuffle pass without immediately repeating the previous pass's last occurrence
when alternatives exist. One repeats current only on normal EOF; manual Next
still advances (without All's wrap). Errors stop, never repeat or skip silently.

Shuffle visits each occurrence once per automatic pass. Duplicate files remain
separate occurrences. Previous follows actual selection history; Next replays
forward history before consuming unvisited IDs. Bound history to 1024 entries.
Selecting a different row branches history and consumes that occurrence from the pass;
explicit user selection/history replay may revisit an item. Stop/replay retains
modes and traversal. Changing Shuffle starts a new traversal anchored at current;
changing Repeat does not reset history.

Append inserts new IDs into remaining shuffle work. Reorder preserves shuffle
order/history. Removing other IDs prunes pending/history; removing current stops
and anchors a fresh traversal at the existing fallback selection. Stale IDs are
ignored. Core is portable and deterministic with an injected seed; production
supplies a fresh seed without paths/secrets. No new dependencies, network, storage,
music writes, cryptographic use or unbounded history.

## Delivery and checks

1. Extend core queue policy and seeded tests for modes, history, edits, duplicates,
   empty/single queues, cycle boundaries, stale IDs and bounded repeated use.
2. Route explicit Next/Previous/normal EOF through core; snapshots supply navigation
   availability. Verify real native EOF/reopen, paused navigation and failed media.
3. Shared accessible desktop controls show confirmed modes in compact/expanded
   layouts. Verify callbacks and a native UI journey changing modes during playback.
4. Run workspace tests, core-only build, rustfmt/Clippy, marker regression, native
   UI/resource/visual checks. Preserve 200 MiB desktop / 100 MiB audio reference
   budgets and p95 <=50 ms UI acknowledgement; document measurement limits.
5. Update current feature/maps/roadmap and record evidence. Archive this plan only
   after the scoped implementation/checks pass; keep base acceptance gaps explicit.

Reading profile: `shuffle-repeat`. Existing behavior:
[desktop feature](../../architecture/features/local-desktop-player.md).


## Final evidence and limits

- `cargo test --workspace --all-targets --locked`: 41 passed; eight opt-in tests
  excluded from this default run. Core-only `cargo check`, desktop build, rustfmt
  and all-target Clippy with warnings denied passed.
- Native `native_modes_journey`, `native_queue_edit_journey` and
  `native_session_journey` passed. Minimum 760x366 layout at 2x scale passed;
  compact/playlist screenshots inspected. Twelve session cycles and 1,080 panel
  toggles: 181.793 MiB sampled process RSS, panel callback-group p95 7.117 ms,
  max 9.497 ms. Linux/debug/Slint Winit/FemtoVG, glibc threshold 65536 at exec;
  RSS sampled every 20 ms, excluding external portal/GPU services.
- Production-session virtual-monitor smoke (`--session-worker --trials 3`) passed
  three WAV and three FLAC journeys plus protocol/pending-command loss, malformed
  media and dead-monitor negatives. Three 60-cycle resource runs per codec:
  largest sampled active/post-drop RSS 73.30 MiB WAV, 73.83 MiB FLAC. This is a
  regression/resource check, not a rerun of the earlier 100-trial latency gate.
- One supplied FLAC and MP3 passed the silent-output journey; SHA-256, size,
  mtime and mode unchanged. No new listening/physical-output claim.
- No dependency changes. Occurrence/history bounds, stale IDs and failed-media
  no-retry behavior tested; source paths stay outside traversal and diagnostics.
  Documentation profiles/link checks and staged/unstaged whitespace checks passed.

An initial probe launch lacked the workstation native-library environment and
failed to build; rerunning with documented RUSTFLAGS/LD_LIBRARY_PATH passed.
Modes remain session-only. EQ/gapless and the base plan's manual/native-stall
acceptance remain separate work. Ignored local logs/screenshots are not shipped.
