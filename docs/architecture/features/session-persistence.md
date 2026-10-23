# Session persistence

## Read When

Changing desktop restart, autosave, profile files or startup/close ordering.
Read the [desktop](../desktop-project-map.md) and [core](../core-project-map.md) guidance.

## Do Not Read For

Named playlists/catalog, M3U, credentials, playback position/history or
[analysis cache](analysis-cache.md).
[Library/playlists](../target/library-and-playlists.md) owns those future contracts.

## Current Behavior

Desktop automatically preserves volume, ordered local queue (including duplicate
occurrences), selection, Shuffle/Repeat and active EQ: all ten gains,
manual preamp/bypass, TRIM/FADE/MIX, theme and EQ/Library visibility. Restart never opens media or plays. Explicit Play starts the retained track from zero.
Missing references remain; playback revalidates them read-only. Empty queues/mute
are saved. [Metadata](track-metadata.md) restores separately in a background batch.
[Processing bounds](playback-edges.md) are recomputed, never persisted.

`SavedSession` owns audio intent; `StoredSession` adds interface state. Restore is
initial-only; windows wait hidden for restore, then show the saved theme/panels.
Profile errors remain visible; failed access keeps controls blocked.
Restart creates fresh occurrence IDs and shuffle history around the retained selection.

One off-thread storage job keeps one latest pending snapshot. Autosave coalesces changes for 500 ms, excluding playback timing. Close
waits for accepted commands, requests audio cleanup and flushes final intent before
exit. A killed process may lose changes since its latest completed transaction;
filesystem/native stalls have no hard cancellation bound.

## Key Files

- `crates/resolibre-audio/src/session/saved.rs`: validated local intent, redacted Debug.
- `session/mod.rs`, `session/worker.rs` in that crate: initial restore and observations.
- `apps/desktop/src/session_store.rs`, `session_store/format.rs`: owned jobs and format.
- `apps/desktop/src/profile_storage.rs`: shared bounded/private I/O, also used by presets.
- `apps/desktop/src/presentation.rs`, `interface_state.rs`: restore, UI intent and close.

## Tests

Storage tests cover round-trip, all truncations, unknown/oversized/invalid input,
non-UTF8 paths, duplicates, missing references, empty queues, last-write coalescing,
profile isolation, competing writers, symlinks/FIFOs and preserving failed writes.
The production controller restart test changes order/modes/EQ/volume, closes with
pending commands, restores theme/panels, empties/mutes and closes during startup.
Native restore tests prove no initial engine opens and explicit Play uses retained
selection/gain/EQ. Existing preset, queue, scrub/EOF and UI tests remain regression gates.

## Edge Cases

Storage: `$XDG_CONFIG_HOME/resolibre` for absolute XDG roots, otherwise
`$HOME/.config/resolibre`. `session-v1.bin` holds v4 (v1–v3: MIX off; v1/v2: Green, panels visible; v1: processing off) and
length-prefixed raw paths, separately from portable named EQ presets.
Bounds: 1024 occurrences, 4096 bytes/path, 1 MiB total paths, 1,052,805-byte file.
Decode rejects paths with NULs/relative roots, invalid selections/modes, nonfinite/out-of-range
gains, unknown fields/versions and trailing bytes before accepting any state.

Private transactions use mode 0600, atomic rename, file and directory sync.
App directories are created with mode 0700. No music-file writes or network I/O.
A held `session.lock` lease prevents two live instances from overwriting each
other's session; another window works without restoring/saving and shows a warning.
[Reset](profile-reset.md) adds a shared profile-use lease, retained on load errors.
The final owner unlocks explicitly; transient fork/exec copies cannot retain the lease.
Corrupt, unknown or unreadable state disables saving for that run, preserves the
file and warns once; playback remains available. Resolve storage errors or move
the bad application file aside before restarting. Write failure before rename
preserves the prior file; a later directory-sync error cannot promise rollback.
Private paths never appear in storage errors or snapshot diagnostics. Existing
OS-configured parent directories are trusted; named EQ presets remain independent.

## Related Docs

[Local player](local-desktop-player.md), [EQ](local-equalizer.md),
[order policy](queue-playback-order.md), [security](../target/security-and-privacy.md),
[test plan](../../testing/test-plan.md).
