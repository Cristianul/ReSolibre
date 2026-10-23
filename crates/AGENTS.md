# Core and adapter rules

Follow [root rules](../AGENTS.md) and the
[core map](../docs/architecture/core-project-map.md). The local audio prototype is implemented.
This area owns portable domain behavior and distinct source/audio/storage
adapters. Widgets belong to desktop; domain tests require no GUI/network.

## Boundaries

- Keep native dependencies outside the portable core's compile graph. Model
  source/platform capabilities explicitly; do not assume seek or write support.
- Organize reusable feature policy behind narrow APIs; adapters implement platform
  capabilities, not independent copies of the feature.
- Expose typed commands/events, not libmpv strings, handles or filter syntax.
  Assign one authoritative owner per state; reject stale generations/revisions.
- Isolate unsafe FFI with documented ownership, thread and lifetime contracts,
  RAII cleanup and preserved errors. Avoid recoverable-path panic/unwrap.
- Use Cargo/rustfmt conventions and idiomatic names. Do not fight the formatter.
- Explain unfamiliar ownership/borrowing, lifetime and concurrency choices at
  the point where they affect behavior. Prefer named steps and explicit error
  handling over dense combinators; introduce traits/macros only for a real
  boundary. Keep Rust idiomatic without imitating Java class hierarchies.

## Responsiveness and data

- Never block an audio callback on locks, network, database, images or unbounded
  allocation. Bound workers/channels/read-ahead and prioritize controls/audio.
- Cancel/coalesce obsolete seeks and reads; verify the final requested position,
  not only cursor movement. Discard stale source/search/EQ/device completions.
- Negotiate supported output, preserve source format where practical, and
  report actual conversion. EQ uses validated presets and explicit headroom.
- Read originals only in normal flows. Future writes require a separate
  confirmed capability and rechecked permissions/file identity.
- Keep SQLite local and separate from music/SMB. Use migrations, transactions,
  indexed stable pagination and bounded scan work; failed scans are not deletes.
- Scope IDs, cache and credentials by source/profile. Use bounded SMB2/3 reads,
  timeouts and OS secret storage; never fall back silently to plaintext.
- Validate/bound tags, media, art and playlists before native calls. Redact
  third-party diagnostics. Follow the focused security target for trust rules.

## Evidence and maintenance

Use the [test plan](../docs/testing/test-plan.md): direct behavior, real adapter
boundaries, connected regressions, original-file preservation and negative/
security cases. Update implemented feature docs and area navigation in the
same task; record decisions in focused targets. Explicitly track new relevant
files. Run `python3 tools/check_docs.py`; include root and these rules in the
complete 32 KiB flow budget.
