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

A separate badge model leaves search, scroll, selection and open row menus intact.
The leading chart icon is green/cyan for complete, muted for queued/running, and has
a red X for failure. Accessible labels distinguish these states. Inconclusive BPM
is a successful measurement, not a decoder failure. Badges describe attempts observed
in this session; results persist, but opening Library does not hash every source to
restore badges. Restarted rows remain unmarked until a full result is observed.

One scheduler owns full analysis, with only one decoder active. Background batches
yield between files to automatic current/next rhythm preparation. Playback navigation
cancels/reaps the batch job and resumes it after current-track work. Current rhythm
(excerpt then full timeline) has highest analysis priority, followed by a requested
current plot; it overtakes other queued/running Library plots. Transport stays independent. Stop cancels pending
analysis; closing the diagnostics window cancels only its own explicit request.
Batch work continues if Library is hidden. Shutdown cancels and reaps owned workers.

Only the current generation's plot remains in memory. Other rows retain a small
status; reports are stored separately in the bounded profile cache. Off-track files
open read-only/nonblocking inside the job, require regular-file identity, and use
the default native audio stream. The playing track uses its retained selected stream.
Existing decode limits, input revalidation and redacted diagnostics still apply.
Unknown/stale IDs cannot authorize arbitrary paths. Music files are never changed.

## Key Files

- `crates/resolibre-audio/src/session/analysis.rs`: queue, cancellation and observations.
- `crates/resolibre-audio/src/session/worker.rs`: typed actions and rhythm scheduling.
- `crates/resolibre-audio/src/track_analysis.rs`: retained/default source and forced pass.
- `crates/resolibre-storage/src/lib.rs`: atomic report replacement.
- `apps/desktop/src/{presentation,playlist_window,analysis_window}.rs`: shared actions.
- `apps/desktop/ui/components/{queue-view,queue-row,playlist-panel}.slint`: menu, badges/footer.
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

## Edge Cases

A source can become unavailable or change during analysis; no partial result is used.
Cache failure permits a successful in-memory measurement without durable reuse; the
badge means analysis succeeded, not a guarantee that storage succeeded. No automatic
retry storm or scan occurs. Failed badges retain no private native error payload.
Linux execution does not establish Windows/Android/iOS adapter support. Persistent
badge restoration and a durable catalog/job history are not implemented.

## Related Docs

[Analysis target](../target/analysis-and-radio.md), [cache](analysis-cache.md),
[test plan](../../testing/test-plan.md), [Library interactions](playlist-interactions.md).
