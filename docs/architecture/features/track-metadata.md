# Track labels and embedded covers

## Read When

Changing read-only tags, durations, Library labels or covers.
Use root/core/desktop rules, both area maps and the test plan.

## Do Not Read For

Tag editing, library catalogs, technical codec/provenance analysis or live spectra.
The [analysis plan](../../plans/track-analysis-plan.md) remains separate and active.

## Current Behavior

The session loads unchanged Library metadata from private SQLite storage in one
background batch on restore. Misses are probed; startup never hashes audio. Container artist/title tags take precedence over the first audio stream's
tags. Display `Artist – Title`, title alone when artist is absent, or the sanitized
filename when title is absent/unreadable. Case-insensitive keys, Unicode, control/
bidi neutralization and 160 characters per field apply. Both views share labels; paths/order/playback are unchanged.

One cancellable job prioritizes the selected occurrence. Only its embedded attached
picture is decoded; ordinary video frames, folder images and remote artwork are
excluded. The square cover uses centered fill; missing/rejected images retain the
forest illustration. Changing/reopening an occurrence marks its new cover pending;
the UI retains the last image until a definitive cover or missing-art result, so
the fallback never flashes between covers. Accessibility reports loading during
this handover. Removing the last track clears it. Stale results never become
another track's cover. Pause/Stop retain the selected cover; restore never autoplays.

`ffprobe` and `ffmpeg` run with descriptor-only input, allowlisted demuxers, no
network protocols, no FFREPORT, private stdout and discarded native diagnostics.
Regular files only; symlinks/FIFOs are rejected. Both phases share a 3 s deadline,
20 ms cancellation checkpoints, 1 MiB probing, one decoder/filter thread and a
32 MiB native per-allocation limit (not a process RSS cap). Probe JSON is capped
at 64 KiB; source pictures at 4096 px per side and 4,194,304 pixels. Result RGBA is
512² / 1 MiB. File identity/size/change timestamps must match after inspection.
Only current cover bytes (plus the previous UI image during handover) and at most
1024 bounded labels/search records survive in memory.
Reopening re-probes that source; edits to unselected files await re-add/reopen.
Unavailable helpers/media use fallbacks, with one sanitized warning per session.
Sources are never modified. No technical lossless/authenticity claims are inferred.

[Library search](library-search.md) also retains album, album artist, date/year and
disc/discnumber from the same probe, with container precedence and 160 characters
per field. Only these eight allowed tags form the lowercase search text; filenames
are included separately without directories. Search-only changes advance the queue
presentation revision, while absent tags preserve the filename fallback unchanged.

The same probe reads first-audio-stream duration (container fallback), validates
finite 0..u32::MAX seconds, truncating fractions. Rows/totals show `mm:ss` below
one hour, else `HH:mm:ss`. `library_duration.rs` sums all occurrences, independent
of search/TRIM/FADE. Unknown: `--:--`; partial totals: `+`; empty: `00:00`.
Durations refresh on queue revisions. `analysis/analysis.sqlite3` also holds bounded
`metadata_v1` rows (4,096, no artwork): title, search tags and duration. Keys include
device/inode, size, mtime and ctime; they are freshness hints, not content hashes.
Changed/inaccessible files cannot reuse rows; current cover still probes separately.
Storage errors fall back to probing; database security/size limits remain shared.

## Key Files

- `crates/resolibre-audio/src/metadata.rs`, `metadata/process.rs`: owned job, tags/image validation, bounded helpers.
- `crates/resolibre-audio/src/session/metadata.rs`: scheduling/bulk restore.
- `crates/resolibre-storage/src/metadata.rs`: bounded durable metadata.
- `apps/desktop/src/{artwork,library_duration,presentation}.rs`: covers, durations and totals.
- `apps/desktop/ui/components/instruments.slint`: cover/fallback view.

## Tests

`metadata::tests`: generated tagged MP3/FLAC, embedded PNG, missing tags/art,
malformed duration/input, Unicode/bidi/length limits, oversized/video rejection, symlinks,
FIFO, forbidden network references, cancellation, replacement and source bytes.
Restart cache tests cover probe bypass, changed file facts, cancellation and profile
isolation; storage rejects bad durations, oversized text and bidi controls.
`artwork::tests`: square presentation, clearing, invalid-buffer fallback and pending
handover without placeholder flashes, including rapid changes and missing covers.
`native_processing_preview`: cover pixels/yellow LED in both palettes at 1x/2x.
2026-10-23: 225 workspace tests passed; native previews at 1x/2x passed.
`cached_library_startup_timing`: 1,024 duplicate occurrences loaded in 16 ms,
66.4 MiB test-process RSS. This excludes UI/GPU and cold physical-disk costs.
Isolated software UI: first X11 window mapped in 589 ms; idle X11/Wayland
295/280 MiB, above 200 MiB. Instant/glitch-free startup remains unproven.

## Edge Cases

Duplicate paths have separate IDs; only current cover is retained.
Oversized tags can reject the probe entirely; cover decode failure preserves valid
tags. Only the first usable attached picture is selected. Helpers are existing
FFmpeg prerequisites; serde_json reuses the pinned workspace version. Missing
helpers never fail audio playback. Timed-out optional probing is not retried until
reopen/re-add. Local filesystem open/metadata calls retain OS I/O latency limits.

## Related Docs

[Player](local-desktop-player.md), [appearance](player-appearance.md),
[import](local-file-import.md), [security](../target/security-and-privacy.md).
