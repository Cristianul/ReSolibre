# Library and playlists target

Status: catalog/named playlists planned. [Local session restore](../features/session-persistence.md) exists.
Reevaluate native/service playlists and M3U after Jellyfin browse/play; implement
only if needed then, otherwise defer further. They do not gate Jellyfin or SMB.
Cross-source matching and service synchronization remain later work.
[Sources](library-and-sources.md) owns access and original-file protection.

## Local catalog and scale

Use SQLite in application storage, never beside music or on an SMB share.
Target hundreds of thousands of entries and a million-entry synthetic stress
catalog; a million real audio files are not required for this test.

- Index artist/album/title browsing; FTS5 is the search candidate. Define Unicode,
  prefix and tokenization behavior through tests.
- Use stable sorting with a unique tie-breaker and indexed keyset/cursor pagination
  where appropriate; avoid deep OFFSET scans and whole-library UI loading.
- Bound result pages, metadata parsing, thumbnails and artwork memory. Load art
  on demand, not eagerly for every search result.
- Scan incrementally with checkpoints, transactional batches and bounded writer
  work. Define page/snapshot consistency and keep queries responsive during scans.
- Failed enumeration is not an empty source. Preserve unavailable entries and
  recover after interruption without duplicating them on reconnect.
- Keep caches/indexes rebuildable; migrations preserve playlists/preferences and
  protect the previous catalog on failed or interrupted upgrades.

Support folder and artist/album browsing, search, queue editing, drag-and-drop,
shuffle/repeat and read-only Info. Layout changes preserve source and queue.
[Local queue gestures](../features/playlist-interactions.md): click selects, double
click plays, drag reorders with Escape cancellation, context Delete removes only
the occurrence. [Local search](../features/library-search.md) filters session tracks;
catalog indexing remains planned. The player title reveals current in Library.

## Minimal identities now; matching later

Use stable internal IDs independent of path, title and content hash. Keep source
identifiers scoped by source/profile; credential rotation does not create new
tracks. A playlist occurrence has its own ID, separate from the track it refers to.
Store locations and availability separately so a missing file remains referenced.
This foundation does not require fingerprinting or a universal recording database.

Later identity design must distinguish musical recording, version/mastering,
local or remote representation, location, device availability and metadata.
Several representations may relate to one recording without being interchangeable.
Keep original version/source preference; do not silently replace live, remixed,
edited or remastered audio. Final schema and automatic matching rules are deferred.

Moved-file recovery may combine source IDs, hashes, acoustic fingerprints, metadata,
duration and filename/directory hints. Uncertain matches remain missing and offer
manual relinking; they never delete playlist entries. Tag edits can change a file
hash without changing the music. A fingerprint or MusicBrainz ID alone does not
prove identical mastering. Revalidate availability and access at playback time.

Library health later includes missing/broken references, inconsistent/missing tags,
missing covers, corrupt files and optional integrity verification. Distinguish
byte-identical duplicates from acoustic or metadata candidates. These are read-only
reports; never auto-delete, merge, overwrite or retag originals.

## Local queue imports

[Drop policy](local-file-import.md) covers local Add/Replace and folder expansion.
Catalog scans and saved named playlists remain separate.

## Native playlists and M3U

Native playlists are local persistent application data, usable without servers or
accounts. Preserve ordering, repeated track occurrences and unresolved references
across restarts. Users may explicitly remove missing entries. Queue state and a
saved playlist are separate concepts. Changes
are transactional; source disconnection does not erase entries.

Investigate a small M3U/M3U8 import/export slice alongside native playlists, including
classic Winamp paths and EXTINF metadata. This is a desired interoperability feature,
not a reason to delay native persistence for obscure dialects. Prefer UTF-8 M3U8;
define legacy encoding/path handling and report unsupported entries without silent
loss. Resolve relative paths against the imported playlist under the source access
policy. Import is not permission to fetch arbitrary URLs or follow nested playlists.

Export is an explicit user-chosen write, separate from scans/import and never an
implicit sidecar beside music. Confirm replacement of an existing export. Export
only usable references, never credentials or authentication tokens. Report entries
that cannot be represented portably; another device need not have the same paths.
Mixed-source references are a future capability, not a promise that every external
player can resolve them. Smart playlists and native history/undo follow.

## Service playlists, if needed after Jellyfin

When service playlist support is needed, display Jellyfin/Navidrome existing
playlists with origin labels, without conversion for browsing/playback. Import as
Native creates an independent local copy retaining origin references; it never
overwrites the remote list or implicitly downloads audio. Local playlist ownership
does not make unavailable remote music playable offline.

| Mode | Ownership and behavior |
| --- | --- |
| Linked / read-only | Service owns the list; ReSolibre presents its state |
| Linked / manual sync | Keep base snapshot and local changes; compare and preview chosen writes |
| Linked / automatic sync | Future opt-in bidirectional sync with conflict handling |
| Native copy | Independent local list with informational origin references |

Detect additions, removals, reorders and relevant metadata/source changes by
comparing external snapshots. Consider three-way merge only in this later phase.
Do not assume stable remote occurrence IDs, atomic writes, revision preconditions,
push notifications or identical permissions across services. Repeated items and
concurrent edits may require manual resolution. Unsupported local-source entries
cannot silently disappear when syncing to a service. Expose per-adapter capabilities;
do not promise lossless automatic sync when the API cannot protect concurrent edits.

## Evidence

[General tests](../../testing/test-plan.md) cover scale and migrations;
[playlist cases](../../testing/expanded-features-test-plan.md) add planned identity,
interoperability and conflict checks. References:
[SQLite FTS5](https://sqlite.org/fts5.html),
[pagination](https://sqlite.org/rowvalue.html#scrolling_window_queries),
[MusicBrainz recording/mastering](https://musicbrainz.org/doc/Style/Recording),
[OpenSubsonic updates](https://opensubsonic.netlify.app/docs/endpoints/updateplaylist/).
