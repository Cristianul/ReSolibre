# Application menu and profile reset

## Read When

Changing the header menu, full reset, profile ownership or shutdown before deletion.
Use the `profile-reset` reading profile.

## Do Not Read For

Deleting music, named playlist design, external skins or analysis algorithms.

## Current Behavior

The top-right application menu contains a Theme submenu (Green/Spectrum) and
Reset application data. Theme choices use the existing shared action. Escape and
outside click dismiss; theme selection returns focus to the menu button.

Reset opens a confirmation naming imported library entries, analyses, cached
metadata, named EQ presets and settings. Cancel leaves everything untouched.
Confirm stops playback/analysis, cancels imports/pickers and waits for accepted
commands and pending preset/session writes. One owned background job then removes
known data from the current profile; ReSolibre closes. Reopening starts with defaults.
No music paths are read by reset and no original music or exported preset is deleted.

A shared `profile.lock` lease is acquired before normal desktop profile use and
retained even if session settings are corrupt or another window owns session saving.
Reset upgrades it to exclusive access; another current-version window blocks reset.
Preset loading waits for profile access. If access fails, controls remain blocked
until reopening; window Close remains available. A reset failure stays visible with a Close
button; old autosave state cannot resume. No worker is detached during shutdown.

Only known root settings, SQLite cache files, preset files and recognized atomic-write
leftovers are removed. Bounded enumeration validates everything first. Retained
directory descriptors anchor removal; links, hard links, special files, unexpected
entries and nested folders are rejected. Empty application directories and lock
inodes remain. They contain no settings; keeping locks prevents competing lock names.
I/O failure after deletion starts may leave a partial reset and is reported honestly.

Windows pins ancestor directories and deletes validated leaves by retained handle.
Shared-to-exclusive lock conversion happens after workers stop; failed conversion
never resumes old autosaves. Reparse points and hardlinks fail closed.

## Key Files

- `apps/desktop/ui/components/app-menu.slint`, `ui/player.slint`: menu/confirmation.
- `apps/desktop/src/presentation/reset.rs`: confirmed shutdown/reset state.
- `apps/desktop/src/profile_reset.rs`: bounded allowlisted removal.
- `apps/desktop/src/{profile_storage,session_store}.rs`: profile leases.
- `apps/desktop/src/main.rs`: auxiliary-window shutdown.

## Tests

Profile tests cover known data, unchanged original music, competing users, symlinks,
hard links and unknown entries. Controller tests cover Cancel, pending autosaves,
shutdown before deletion, no stale resave and default restart. Theme tests cover
submenu dispatch and focus. `native_app_menu_preview` captures submenu/confirmation
at 1x/2x through the isolated desktop harness; no real profile is reset in tests.

## Edge Cases

Close other ReSolibre versions before reset: older builds do not use the activity lease.
Missing/corrupt media never blocks reset. Unrecognized files require inspection;
reset does not recursively erase arbitrary profile content. Disk failures or stalled
filesystem operations cannot promise atomic deletion or a hard completion deadline.

## Related Docs

[Session](session-persistence.md), [presets](named-eq-presets.md),
[cache](analysis-cache.md), [security](../target/security-and-privacy.md).
