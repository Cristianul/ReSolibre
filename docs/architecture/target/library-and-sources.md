# Library and sources target

## Scope and source capabilities

Local playback and catalog first. After everyday local use passes,
add Jellyfin, then direct SMB2/3, then Navidrome/OpenSubsonic. Sources work independently
or together, without a companion server. Remote files use progressive reads.

| Capability | Local / SMB initial policy |
| --- | --- |
| Browse and scan | Incremental bounded background work |
| Read and seek | Bounded random access where supported by source/format |
| Authentication | Per-source credentials; OS secret store when remembered |
| Modify or delete | Unavailable in the initial release |
| Persistent audio download | Explicit later feature, not implicit caching |

Adapters expose capabilities and typed errors, including non-seekable/live input,
listing, original versus transcoded audio and later playlist/write support.
Keep SMB authentication/protocol handling in libsmb2; no SMB1 fallback. Test
authenticated/permitted guest reads, disconnects and denials without repeated
login loops or credential leakage. A failed scan is not an empty source.

Target practical MP3, FLAC, WAV, AAC, ALAC, Opus and Ogg Vorbis support according
to actual decoder/container/platform capabilities; artwork and embedded metadata
are untrusted. The prototype's narrower allowlist is not a complete codec claim.
Adding M4A/MP4 containers or other formats requires explicit validation, not
silently broadening native protocols. Source IDs stay stable across reconnects.

## Data and original-file boundary

Keep SQLite, playlists, caches and art in separate local application storage.
[Catalog/playlists](library-and-playlists.md) owns scale, native persistence,
path-independent identities and later relinking/service synchronization. Read it
for those tasks; the deferred SMB feasibility slice does not implement a catalog.

Ordinary import, scans, cover lookup, playback and preferences never write, retag,
rename or delete music. Do not create implicit sidecars/playlists beside originals.
Use read-only source access where supported. Explicit playlist export chooses its
own destination and confirms replacement; it is not permission to modify music.
OS-managed access timestamps are distinct from application writes: check bytes,
modification metadata and write calls without promising frozen access accounting.

Info is read-only initially. [Metadata editing](metadata-editing.md) owns future
individual/bulk drafts, previews, confirmed writes, backups/conflicts and renames.
Deletion remains a separate explicit confirmed workflow. Cancel/close writes
nothing; recheck permissions and file identity before any future modification.

Credentials stay outside catalog rows, behind opaque OS secret-store references.
Optional remembering fails safely with session-only use or an error; no plaintext
fallback. Forgetting a source removes access, not original media.

## Optional services

Spotify and YouTube Music remain exploratory, outside the core roadmap. Evaluate
only provider-authorized APIs, licensing and platform terms; do not assume ordinary
PCM access, unrestricted playback or downloadable files. No source adapter may
make local playback depend on an external service. Internet/hardware radio and
bounded timeshift are separate [future targets](analysis-and-radio.md).

## References and verification

[libsmb2](https://github.com/sahlberg/libsmb2),
[security/privacy](security-and-privacy.md) and the
[test plan](../../testing/test-plan.md) cover source isolation, bounded reads,
recovery and unchanged originals. Playlist needs are reevaluated after Jellyfin browse/play; defer unless needed.
