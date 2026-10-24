# Library analysis

## Read When

Changing batch diagnostics, cached batches, reanalysis, queue badges or off-track Analyze.
Use the `library-analysis` profile in `tools/check_docs.py`.

## Do Not Read For

[Algorithms](track-analysis.md), [persistent audio identity](analysis-cache.md),
[library gestures](playlist-interactions.md) or [MIX](dj-mix.md).

## Current Behavior

The Library footer offers **Analyze all**, including entries hidden by search.
It snapshots the current library (at most 1,024 occurrences), queues full original
spectrogram/tempo analysis and shows completed, failed and pending counts. While
work remains, the button becomes **Cancel analysis**. New additions are not silently
included; removed entries are discarded. An individual failure does not stop the
batch. Right-click **Analyze** uses the same typed scheduler without selecting,
seeking or playing the row. It works while playback is stopped or on another track.

Analyze all revalidates audio hashes and skips decoding for valid full cached results,
including after restart. Missing/corrupt results are computed. Individual Analyze
forces recomputation and replaces the stored result by audio hash. A full save atomically removes
older prefix/whole rhythm rows; the session invalidates older in-memory rhythm for
that occurrence. Failed/cancelled attempts preserve the previous valid cached result.
Opening artwork diagnostics can reuse a completed result; its Analyze button forces
a fresh pass. Repeated clicks on an already fresh queued/running job coalesce.

Clicking a row's analysis badge opens that occurrence's spectrogram/BPM window,
reusing its saved report or computing it if unavailable. It never selects, reorders
or plays the row; filtered views and duplicate titles still use stable occurrence IDs.
The view owns a separate inspection result from current-playback analysis, so another
track cannot supply the player's BPM or codec facts. Its retry targets the inspected
row, including after Stop, and removed/unknown IDs are rejected. Pending inspection
can resume after playback navigation; closing cancels only its foreground work.
The artwork chart button also remains available while the displayed track is stopped.

A separate badge model leaves search, scroll, selection and open row menus intact.
The chart icon is green/cyan for complete, muted for queued/running, and has
a red X for failure. Its three bars grow and shrink independently while running;
queued and finished states are static. Hidden/offscreen rows, minimized windows,
disabled controls and reduced motion stop the animation. Accessible labels distinguish
these states. Inconclusive BPM is a successful measurement, not a decoder failure.

At startup and on import, an owned background worker progressively restores badges
from valid full cached reports without starting playback or analysis. Unchanged file
facts reuse a bounded content-identity index; older caches and changed/new files are
hashed once without decoding. Every lookup rechecks read access and regular-file facts.
Rhythm-only, missing, changed-audio, corrupt and evicted results remain unmarked.
Fresh session attempts take precedence over late restoration; removed IDs are ignored.
Only small status observations reach the session, never a library of retained plots.

One scheduler owns full analysis, with only one decoder active. Background batches
yield between files to automatic current/next rhythm preparation. Playback navigation
cancels/reaps the batch job and resumes it after current-track work. Current rhythm
(excerpt then full timeline) has highest analysis priority. Starting playback
automatically queues a full report check for the current occurrence; enabling
MIX also queues the actual next occurrence. Cached completed reports are reused.
Each playback generation revalidates the retained source before reuse, so a
replaced recording at the same queue occurrence cannot keep an old result.
Current full analysis runs before next full analysis, and both overtake unrelated
Library batches after rhythm preparation. Full jobs yield while two streams
overlap, then resume after handover. A requested current plot retains its
foreground priority. Transport stays independent. Stop cancels pending
analysis; closing the diagnostics window cancels only its own explicit request.
Batch work continues if Library is hidden. Shutdown cancels and reaps owned workers.

Only current playback and the explicitly inspected plot remain in memory, sharing
one report when they match. Other rows retain small statuses; reports use the bounded cache. Off-track files
open read-only/nonblocking inside the job, require regular-file identity, and use
the default native audio stream. The playing track uses its retained selected stream.
Existing decode limits, input revalidation and redacted diagnostics still apply.
Unknown/stale IDs cannot authorize arbitrary paths. Music files are never changed.

## Key Files

- `crates/resolibre-audio/src/session/analysis.rs`: queue, cancellation and observations.
- `crates/resolibre-audio/src/session/worker.rs`: typed actions and rhythm scheduling.
- `crates/resolibre-audio/src/track_analysis.rs`: retained/default source and forced pass.
- `crates/resolibre-storage/src/lib.rs`: atomic report replacement.
- `crates/resolibre-storage/src/track_sources.rs`: bounded badge identity hints.
- `crates/resolibre-audio/src/analysis_cache/status.rs`: background badge restoration.
- `apps/desktop/src/{presentation,playlist_window,analysis_window}.rs`: shared actions.
- `apps/desktop/ui/components/{queue-view,queue-row,playlist-panel}.slint`: menu, badges/footer.
- `apps/desktop/ui/components/analysis-indicator.slint`: state labels and visible running bars.
- `apps/desktop/ui/assets/analysis.svg`: Lucide `chart-no-axes-column`, existing Lucide license.

## Tests

Native generated-file tests cover idle full-library work, missing-source failure,
continuation/retry, unknown IDs, removal/cancellation during playback and original-file
preservation. A seeded stale cache proves explicit Analyze decodes again and replaces
full/rhythm results; a restarted batch preserves seeded full results. Compiled UI pointer tests cover off-track menu identity, independent
badge updates, filtered Analyze all and cancellation. Native Library captures cover
palettes, Unicode and scale. Run workspace tests, Clippy and the Library preview cases
with the documented native environment. Workspace verification: 194 tests passed
(36 opt-in cases excluded), plus 13 Python probe checks; Clippy, formatting and docs
checks passed. Five native checks passed: Library on X11 at 1x/2x and Wayland,
plus diagnostics journeys at 1x/2x. Software-rendered diagnostics peaked at
364/383 MiB, above the existing 200 MiB target; this is not resource acceptance.
Performance/device/platform gates in the
[analysis plan](../../plans/track-analysis-plan.md) remain separate.

Regression tests also cover automatic badge restoration before Analyze, duplicate
occurrences, older unindexed reports, missing/rhythm-only sources, corrupt/cleared
reports and profile isolation. Compiled UI checks advance the frame clock to verify
bar growth/shrinkage and static reduced-motion, minimized, queued and finished states.

Stop/inspection regressions cover idle analysis, independent library results, retry
identity and removed IDs. Production UI wiring and pointer tests verify stopped
artwork access, badge clicks with duplicate titles/filtered rows, and unchanged
selection, playback and source bytes. Repeated inspection keeps forced retries fresh. A MIX-handover regression checks
that reopening the previous badge reads its saved report without decoding again.

Badge verification (2026-10-23): targeted regressions, storage tests, Clippy and three
native Library previews (X11 1x/2x, Wayland) passed. The full workspace run still fails
three unchanged session-format migration tests: their byte offsets assume the old
17-byte header, while `ReSolibre-Session` uses 19 bytes. This is not a fully passing
workspace baseline; native preview checks used an isolated profile and display.

Stop/inspection verification (2026-10-23): workspace tests passed with those three
known migration failures excluded. Targeted regressions, Clippy, scoped formatting,
docs validation and five isolated native checks passed (Library X11 1x/2x and
Wayland; analysis journeys at 1x/2x). Artwork now renders the chart icon alone.

## Edge Cases

A source can become unavailable or change during analysis; no partial result is used.
Cache failure permits a successful in-memory measurement without durable reuse; the
badge means analysis succeeded, not a guarantee that storage succeeded. No automatic
retry storm or scan occurs. Failed badges retain no private native error payload.
Linux execution does not establish Windows/Android/iOS adapter support. A durable
catalog/job history is not implemented; badge restoration depends on the bounded cache.

## Related Docs

[Analysis target](../target/analysis-and-radio.md), [cache](analysis-cache.md),
[test plan](../../testing/test-plan.md), [Library interactions](playlist-interactions.md).
