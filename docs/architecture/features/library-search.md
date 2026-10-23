# Library search

## Read When

Changing Library filtering or its embedded editor. Use root/core/desktop rules,
both area maps, [metadata](track-metadata.md), security and the test plan.

## Do Not Read For

SQLite catalogs, saved playlists, tag editing or network search.

## Current Behavior

The footer's magnifier/Search button toggles an inline editor directly above it.
Opening focuses the editor. Typing filters all session entries live by filename,
artist, title, album, album artist, date/year and disc/discnumber. Metadata arrives
through the existing background reader; results refresh when tags become available.
Missing/unreadable tags still allow filename/title matches.

Search uses Unicode lowercase and literal substrings, with all whitespace-separated
terms required across those fields. It is accent-sensitive, with no regex, fuzzy
matching or query operators. Queries are limited to 256 Unicode characters.
The footer shows matching/total tracks plus unfiltered duration; empty results say “No matching tracks”.
Clear restores all rows; closing Search or Escape also clears the query. Search
state survives hiding the Library, but is not persisted across application restarts.

Filtering changes only presentation: queue order, playback traversal, shuffle,
repeat and persisted entries remain intact. Original row numbers and stable IDs
survive filtering; playing/deleting/reordering use the existing occurrence actions.
Reordering places the source beside the visible target in the full queue. Current
markers use the filtered index. The player's title clears search before revealing
and selecting current. Query/model changes cancel pending row gestures.

## Key Files

- `apps/desktop/src/library_search.rs`: bounded matching and stable-ID lookup.
- `apps/desktop/ui/playlist-window.slint`: filtered model/current index.
- `apps/desktop/ui/components/playlist-panel.slint`: footer/editor composition.
- `apps/desktop/ui/components/text-field.slint`: shared text input with native IME.
- `crates/resolibre-audio/src/{metadata,session/metadata}.rs`: bounded searchable tags.

## Tests

`library_search::tests`: literal/Unicode matching, field combinations, duplicate
titles, large IDs, query limits, real input, playback mapping, late tags and reveal.
Metadata tests probe generated tagged MP3/FLAC and reject unrelated/private fields.
Existing row gestures, queue, persistence and native Library previews are regressions.
The X11/Wayland journey covers clearing a filter before current-track reveal;
delayed property notifications must not reset its scroll. [Preview](../../design/library-search.png).

## Edge Cases

At most 1,024 session entries are filtered in memory. No additional probing per
keystroke, file writes, network access, dependencies or disk cache. Only allowlisted,
sanitized, bounded tags are retained; queries/tags/paths are not logged. Full paths
are not searchable. Canonical Unicode normalization and catalog-scale indexing are
future work; no new whole-app memory or physical IME validation is claimed.

## Related Docs

[Gestures](playlist-interactions.md), [hosts](detachable-playlist.md),
[catalog target](../target/library-and-playlists.md).
