# Metadata editing target

Status: desired later enhancement, outside the initial read-only release.
No metadata writer exists. Aim for practical Mp3tag-like individual and bulk
editing without promising every format, field or transformation at once.

## Drafts and scope

Expose Edit Tags contextually. Support title, artist, album artist, album,
track/disc numbers, date/year, genre, comments, compatible extended metadata
and album artwork according to each format's capabilities. Unknown/unsupported
fields must not be silently discarded during a round trip.

Distinguish local library overrides from embedded file metadata. Show exactly
which store and files will change. Multi-selection displays mixed values;
only explicitly selected fields change. Untouched differences survive. Clearing
a field is distinct from leaving it unchanged. Cancel/close performs no write.

Filename-to-tags, tags-to-filename and artwork import/replacement are desired
extensions. Filename-to-tags first creates drafts; tags-to-filename is an explicit
rename workflow. Preview paths, collisions, invalid/reserved names and resulting
library references. Never infer permission to rename from normal scanning.
MusicBrainz or similar lookups are optional and user-enabled; preview matches
and network disclosure before applying anything to files.

## Confirmed writes and recovery

Before Save show proposed changes and affected files, obtain confirmation and
recheck source write capability, permissions and file identity/version. Do not
overwrite an external edit. Source support for reads does not imply writes.
Keep initial playback/scanning strictly read-only; a future write capability is
separate and unavailable to generic remote playback commands.

Define safe replacement, backups and recovery per format/source before enabling
writes. Do not promise atomicity across a bulk batch or all SMB servers. Report
per-file success/failure and partial completion, protect originals on failure,
bound backup storage and provide a recoverable path after interruption. Preserve
unrelated tags and audio content where the format supports it. Reconcile local
references and invalidate affected caches only after confirmed outcomes.

Validate image dimensions/decoded size, metadata lengths and untrusted filenames;
never follow tag-derived paths outside the chosen authorized scope. Artwork must
not trigger hidden downloads. File deletion remains its own explicit confirmed
workflow, preserved from the earlier plan; it is not duplicate cleanup by default.

## Evidence and related targets

[Sources](library-and-sources.md), [library identities](library-and-playlists.md),
[security](security-and-privacy.md) and the
[planned write tests](../../testing/expanded-features-test-plan.md) define access,
conflict, preservation and failure obligations. The writer library, supported
formats and backup policy remain open until implementation evidence is available.
