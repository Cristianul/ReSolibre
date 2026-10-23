# Expanded feature acceptance plan

Planned checks: read the affected target and [general plan](test-plan.md).
Not needed for unchanged prototype checks. Use owned fixtures, never users' music.
Assess trust/access/isolation, untrusted input, private logs/exports/caches,
dependencies and abuse/replay in every group.

## Audio processing

- Volume: silence/unity, invalid/NaN values, click-free changes; UI/media/remote
  commands leave OS/global and other-app volume unchanged. Reject stale device gains.
- Normalize all tracks: loud/quiet/silent/near-silent and different masters; invalid,
  missing or stale/transcoded measurements. Measure loudness/peaks, dynamics and
  combined EQ/mix headroom. Playback never waits or applies late analysis abruptly.
- Crossfade: same/different albums, CUE when supported, short inputs, different
  rates/channels; pause/seek/skip/queue edits, cancellation and next-source stalls.
  Measure overlap/continuity/clipping; disabled crossfade preserves compatible gapless.
- Beat sync: known beats and licensed music, half/double/changing tempo, absent/slow
  rhythm, introductions/low confidence. Verify phase, percentage limits, pitch/tempo
  restoration, resource bounds and ordinary-crossfade fallback without lost intros.
- Diagnostics: compare graph/formats/backend with reality; stale/unknown data and
  PCM width never imply original bit depth, provenance or bit-perfect output.

## Library and playlists

- Native lists: order/repeats, restart/crash, disk-full, interrupted transactions/
  migrations; preserve recoverable lists and IDs.
- Missing/moved/offline sources preserve entries; credential rotation creates no
  duplicates and forgetting revokes access. Later matching covers retags, identical
  bytes, re-encodes/remasters/live/remixes/edits: uncertainty never auto-merges/deletes.
- M3U/M3U8: Winamp/relative/absolute paths, Unicode/legacy encodings, EXTINF, repeated/
  missing entries, unsupported dialects. Bound size/count; reject unsafe protocols,
  recursive fetches and token exports. Imports never write music or grant network
  access. Test explicit export replacement, failures and non-portable references.
- Services later: origin/read-only/native copies leave remote lists unchanged.
  Compare adds/removes/reorders/repeats, permissions, missing sources and concurrent
  edits. Test stale snapshots, retries/partial writes and unsupported revisions or
  occurrence IDs; no lossless-sync claim beyond API guarantees.

## Analysis and radio

- Spectra: signals, silence/noise, spikes, lossy-to-lossless fixtures; preserve
  unknowns and never certify provenance from bandwidth.
- Under scans/analysis measure audio/controls, CPU/RAM, I/O, queue/cache bounds;
  slow SMB, battery throttling, cancellation, long/corrupt input, full disk/cache,
  source removal and stale completion. Drop visual frames, never block audio;
  record observed underruns. Test compatible cache hits, upgrade invalidation,
  clearing/interrupted rebuilds and later incompatible algorithm/model choices.
- Track Radio: disabled network, incomplete tags, uncertain genres/identities,
  unavailable/repeated songs and optional feedback. Assess musical usefulness
  separately. Online opt-in never auto-downloads/substitutes or leaks history/secrets.
  Review runtime/model/data licenses before distribution.
- Radio: direct stations without directory access, unsafe redirects/protocols,
  malformed metadata, stalls/bounded retries. Later timeshift enforces duration
  and byte limits, disk-full handling, LIVE return and retention; tuner support
  needs actual compatible hardware evidence.

## Writes and connected clients

- Tags/artwork: individual/mixed bulk fields, unchanged/cleared values, unknown tags,
  image bounds and overrides versus embedded writes. Cancel writes nothing; Save
  rechecks permissions/identity. Test conflicts, rename collision/traversal,
  partial batches/interruption, backup bounds/recovery and promised audio preservation.
- Remote: target library/queue IDs never become controller paths; test app-only
  volume, bounded paging, pairing/replay/revocation, server outages and direct/relay
  authorization equivalence. Browser origin/transport needs real-browser checks.
- Handoff: authorized destination/position and missing audio. Transfers preview
  files/space/existing content, verify integrity/access and never silently
  overwrite/delete or synchronize a whole library.
