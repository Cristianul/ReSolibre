# Security and privacy target

## Trust and source boundaries

ReSolibre has no product login/tenant hierarchy. Boundaries are OS user/profile,
each source and its credentials, imported content, native/FFI code and future
paired devices. LAN discovery is not authorization. Source credentials grant
access only to that source; controllers never receive them.

Normal operation reads original music only. Application catalogs, playlists,
preferences, skins and caches live separately. Future writes need explicit
capabilities, confirmation and revalidation of source permissions/file identity;
Info is not consent. Define conflict/partial-write recovery before release.

Use local SQLite transactions for related state and bounded writer work. A
failed scan or disconnected share is not an empty source or a deletion request.

## Secrets and privacy

Remember credentials only on request through the OS secret store; catalog
rows hold opaque references. Store failure offers session-only use or an error,
never plaintext fallback. Future device credentials are individually revocable.
Secrets cannot enter themes, playlists, metadata exports or diagnostics.

Paths, share names, URLs, tags, artwork and listening history may be private.
Log allowlisted categories/opaque IDs; redact native diagnostic callbacks.
Initial scope includes no telemetry or automatic remote crash upload.

## Untrusted content and dependencies

Bound probes, tag lengths, image sizes, archive expansion, playlists, network
responses and concurrent work before allocation/native calls where practical.
Reject unsafe archive paths/symlinks and unsupported executable skin content.
Test malformed/truncated input and resource exhaustion.

The controlled native player must not inherit external scripts/configuration,
launch URL helpers or follow arbitrary content-supplied protocols. Adapter-
controlled source access must not become arbitrary network/filesystem access.
Review maintained dependency versions/features/licenses and relevant advisories.
Document unsafe interop; safe Rust does not make native dependencies safe.
Use established cryptography; fuzz relevant parsers/boundaries.

## Caches, retention and future audit

Scope cache/catalog IDs by source/profile and invalidate access on credential
or source changes. Do not put private responses in shared/public caches or
treat cached content as authorization. Bound memory/disk storage with eviction
and clear-cache actions. Exact byte limits/history retention remain open.

Forgetting a source removes credential access; define whether catalog references
are retained without deleting original music. No infinite retention/full audio
cache is assumed. Related local changes are transactional. If future sensitive
writes/pairing need durable audit records, define privacy, retention and
atomicity; best-effort logs are not atomic audit evidence.

Future remote transport needs encryption/authentication, narrow commands,
revocation, stale/replay rejection, size/rate limits and reconnect handling.
See [remote target](remote-control.md).

Every change assesses trust, access, isolation, inputs, secrets/private data,
caches, dependencies and abuse. Add relevant negative cases and report gaps
using the [test plan](../../testing/test-plan.md).
