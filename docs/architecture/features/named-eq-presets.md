# Named EQ presets

## Read When

Changing preset names, Save/Replace, import/export, migration or catalog storage.
Follow the [desktop](../desktop-project-map.md) and [core](../core-project-map.md) rules.

## Do Not Read For

DSP response, manual headroom, session restore or future named playlists.
See [local EQ](local-equalizer.md) and [session persistence](session-persistence.md).

## Current Behavior

EQ's header has On/Off on the left, with the selector and floppy Save on the
right. Saved presets appear first, sorted by name, then Flat/Bass/Voice/Bright.
Custom identifies a curve that no longer matches a preset, including bypass.
Flat resets the curve; double-click resets one slider. No Reset button or footer.

Save captures settled settings and opens an embedded name editor. Names are
trimmed, 1–64 Unicode characters, without control characters. Matching is
case-insensitive; a matching saved name requires a separate Replace action.
Cancel/Escape discards the draft; Enter requests Save but never confirms Replace.
Accepted writes finish before shutdown. The selector updates after a successful
write; changing its selection applies the stored curve and On/Off state.

Import/Export live inside the editor. Import picks one file, validates it in the
background and opens a rename/Save draft without changing playback. Export writes
the named draft to a new user-chosen `.resolibre-eq` file. A missing extension is
added; other extensions and existing destinations are rejected. The original
imported file is never rewritten. Copies can be shared and imported by another user.

The catalog is `$XDG_CONFIG_HOME/resolibre/eq-presets/` for an absolute XDG root,
otherwise `$HOME/.config/resolibre/eq-presets/`. Each saved preset is a separate
UTF-8 `.resolibre-eq` file. Files copied into this folder are discovered on restart.
Generated numeric filenames are independent of names, so names cannot escape the
folder. Up to 256 presets and 1,024 directory entries are accepted; each preset is
at most 4,096 bytes. Invalid/duplicate files are skipped with a safe warning.
There is no live directory watcher or in-app deletion in this version.

Version one contains three lines: `ReSolibre-EQ-Preset-1`, the name, then enabled
(0/1), manual preamp and ten gains separated by spaces. Band order/frequencies and
Q are fixed: 31.25/62.5/125/250/500/1000/2000/4000/8000/16000 Hz, Q=1.4;
gains ±12 dB, preamp ±2 dB. No paths, filter expressions, music
metadata, scripts or dependencies are serialized. Gains are finite and validated
against current core limits, without clamping new imports.

Legacy `equalizer-v1.txt` slots migrate into `legacy-1.resolibre-eq` through
`legacy-3.resolibre-eq` as Saved 1/2/3, omitting empty slots. Existing migration files
are preserved, including later replacements; the original legacy file stays intact.
Legacy preamps use the established v1 clamping rule. Removing a migrated file
manually while retaining the legacy source will recreate that slot on restart.

## Key Files

- `crates/resolibre-core/src/equalizer/preset.rs`: portable names and format validation.
- `apps/desktop/src/preset_store{.rs,/files.rs,/legacy.rs}`: catalog, jobs and migration.
- `apps/desktop/src/presentation/presets.rs`: shared save/load/transfer actions.
- `apps/desktop/src/picker.rs`: existing owned chooser process with fixed dialog purposes.
- `apps/desktop/ui/components/{equalizer-panel,eq-preset-dialog}.slint`: selector/editor.

## Tests

Core round-trips Unicode/tiny finite values and rejects malformed, oversized,
unknown-version and out-of-range data. Temporary filesystem tests cover more
than three saves, replacement, reload, export/import, catalog limits, migration,
symlinks/FIFOs, failed atomic replacement and unrelated-file preservation.
Compiled production bindings cover saved-first ordering, Custom, Save/Replace,
cancellation and imported drafts without changing playback. The opt-in native
`native_equalizer_journey` saves a named curve and reloads it during paused playback.

## Edge Cases

One owned I/O job per catalog; UI/audio threads never perform filesystem work.
The shared private-storage adapter writes catalog replacements atomically, with
0600 files and a private folder. A short-lived exclusive catalog lease serializes
load/save; save rereads disk before checking duplicate names. Failed writes retain
the confirmed UI catalog. File readers reject symlinks/nonregular files and cap
bytes before parsing. Kernel stalls remain unbounded. Config parents and explicit
export directories are OS-user-trusted; there is no sandbox against their owner.
Chooser cancellation/timeout leaves audio and catalog unchanged; shutdown kills
and reaps its owned helper. No music writes, network or new dependencies.

## Related Docs

[Audio target](../target/audio-and-equalizer.md),
[security](../target/security-and-privacy.md), [tests](../../testing/test-plan.md).
