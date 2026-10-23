# Persistent analysis cache

## Read When

Changing content identity, durable BPM/beat/spectrogram results or profile storage.
Use the `analysis-cache` reading profile. This is local original-audio caching.

## Do Not Read For

[Session intent](session-persistence.md), [analysis algorithms](track-analysis.md),
[live visualization](live-spectrum.md), library indexing or remote sources.

## Current Behavior

Desktop enables a separate SQLite database at the current profile's
`analysis/analysis.sqlite3`. BPM, observed beat times, confidence/inconclusive
results and full spectrogram measurements survive restart. Prefix rhythm,
whole-track rhythm and full analysis have distinct coverage; a prefix cannot
satisfy a whole-track request. Whole/full results can serve automatic current BPM.
Automatic next-track rhythm requires MIX. Opening diagnostics can reuse a plot;
[Analyze all](library-analysis.md) reuses valid full results; individual Analyze
recomputes and replaces full/older rhythm results. Failed attempts preserve prior data. No music is modified.

`resolibre-storage` owns portable content hashing, SQLite and cached-result validation.
Symphonia demuxes audio packets without constructing a decoder. SHA-256 covers the
complete encoded packet sequence, timing/trim and technical codec setup. MP4 audio
edit lists/movie timescale and gapless trim tags also contribute, because they affect
beat timing; descriptive metadata does not. Text tags,
artwork, paths and stream IDs do not enter the digest. Retagging, renaming or copying
unchanged encoded audio can reuse results. Different encodings may have different
keys even when they sound identical: this is not an acoustic fingerprint or proof
of lossless provenance. Artist/title are bounded descriptive fields, refreshed on
reuse; they do not authorize a lookup. A bounded RIFF INFO fallback covers a
Symphonia 0.6.1 WAV metadata-log omission.

Every job re-reads the audio identity; timestamps alone are never an integrity proof.
The Linux adapter uses an independent cursor on the retained original descriptor,
checks file identity before/after hashing and decoding, and revalidates before
publication. Session generation checks remain authoritative. No database work or
hashing runs on the UI, transport owner or audio callback. Default library/test
sessions have no cache; desktop explicitly supplies a profile location. Future
remote sources must supply a separately scoped storage/access adapter.

Hashing has a two-second cooperative deadline, 512 MiB input/read budget,
16 MiB per probe/packet read phase and no whole-file buffer. Malformed/unsupported or multi-audio-stream containers bypass
persistence and retain ordinary native analysis. Native and portable stream IDs
are not assumed interchangeable: only a single audio stream is eligible. Cached
lookahead still resolves the actual native selected stream before publication.
Local filesystem stalls and allocations internal to demuxers are not hard realtime
bounded. Results describe original audio, independent of EQ, output volume or MIX.

SQLite transactions use schema 3, defensive mode, untrusted-schema protection,
50 ms lock waits, a 2 MiB page cache, no mmap and bounded result reads. Unknown
schemas are rejected; incompatible future analysis/hash changes must explicitly
invalidate the cache in a schema migration, not silently reinterpret old rows.
Schemas 1–2 are atomically invalidated and recomputed on demand for activity edges
and contextual tempo recovery. Schema 3 validates activity bounds before use.
Media and session preferences are untouched. There is no per-algorithm version registry. Corrupt result rows become misses;
corrupt/unwritable/locked storage falls back to fresh analysis and logs one redacted
warning per session. Playback does not depend on storage success.

Retention: at most 1,000 result rows and 192 MiB payload, LRU eviction, 4 MiB per
result and a 256 MiB main-database ceiling. SQLite rollback journaling can add
transaction storage temporarily. Unix directories/files are private (0700/0600);
links, hard links, special files, shared permissions and oversized database files
are rejected. The platform must supply a trusted private profile root. `clear()`
removes analyses and vacuums this database; metadata remains; no desktop clear button yet.
Deleting the dedicated analysis directory while closed also rebuilds the cache.

## Key Files

- `crates/resolibre-storage/`: portable demux/hash, SQLite and validation/tests.
- `crates/resolibre-core/src/analysis/`: serializable typed measurements.
- `crates/resolibre-audio/src/analysis_cache.rs`: retained-file checks and redacted fallback.
- `rhythm_analysis.rs`, `track_analysis.rs` in that crate: worker-only load/save.
- `session/analysis.rs`, `session/rhythm.rs`: explicit profile configuration.
- `apps/desktop/src/main.rs`: current-profile composition.

## Tests

Storage checks restart, tags versus audio changes, inconclusive reports, coverage,
invalid activity/beat/geometry/JSON, schema-1/2 invalidation, future/corrupt schemas, LRU bounds, profile isolation,
links and permissions. Native jobs prove a retagged FLAC cache hit leaves decoded
progress at zero and returns the same measurements; changed audio requires decoding.
A real session restart restores automatic rhythm and the requested spectrogram.

Run `cargo test --workspace`; opt-in container checks:
`cargo test -p resolibre-storage real_formats_retag_without_reanalysis -- --ignored`.
These generate WAV/FLAC/MP3/AAC/Vorbis/Opus fixtures, change tags/covers/MP4 edit
timing, reject multiple audio streams and preserve input originals.
The opt-in `persistent_cache_song_length_timing` uses a generated four-minute FLAC;
one local run measured 2.15 s cold analysis versus 0.12 s for a reopened cache hit,
including content hashing (3.9 MB compressed fixture; not a general benchmark).
`check_playback_responsiveness.py --session-worker --analysis-cache --trials 3`
checks controls/resources with persistence; `check_mix.py` uses a private cache.
Local evidence: 188 workspace tests passed (36 opt-in tests excluded); eight storage
checks including generated-container cases passed. Three MIX captures with caching
passed (maximum beat offset 27 ms, peak 98.12 MiB). Six WAV/FLAC control/resource
runs passed 360 lifecycle cycles, peaking at 94.25 MiB. Playback CPU ranged
5.60–16.19% of one core, above the existing 5% target; the first control run overlapped
the MIX workload, so this is regression/stress evidence, not an isolated CPU baseline.
The 100-trial, physical-device and mobile gates remain open.
Use the documented native environment for adapter checks/builds.

## Edge Cases

Artist/title may be absent; labels are not a catalog or listening history. Cached
technical measurements never replace current container facts such as codec or bit
depth. Storage failure does not erase music or saved session intent. Cancellation
never stores partial analysis. Persistence is bounded, so evicted or unsupported
inputs can be analyzed again. Windows/Android/iOS storage permissions, SQLite
packaging and native decoder lifecycles remain unvalidated; portable Rust does not
establish a working application port.

Dependencies: pinned [Symphonia](https://docs.rs/symphonia/0.6.1/) (MPL-2.0),
[rusqlite](https://docs.rs/rusqlite/0.40.2/) with bundled SQLite (MIT/public domain),
[SHA-2](https://docs.rs/sha2/0.11.1/) and Serde (MIT/Apache-2.0).
`cargo audit` is unavailable in this environment; no full advisory audit is claimed.

## Related Docs

[Analysis target](../target/analysis-and-radio.md), [MIX](dj-mix.md),
[active plan](../../plans/track-analysis-plan.md), [test plan](../../testing/test-plan.md).
