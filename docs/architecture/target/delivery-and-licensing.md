# Delivery and licensing target

## Platforms and tools

| Area | Direction |
| --- | --- |
| First desktop targets | Fedora, Debian and Ubuntu Linux |
| Next desktop target | Windows |
| Future clients | Android; macOS/iOS where feasible; offline-capable web |
| Contributor editors | VSCodium and IntelliJ; editor-independent project |
| Native language/build | Rustup/Cargo; native dependency tooling only as needed |
| Documentation | English Markdown, Mermaid, plain tables/trees |

No supported distribution versions, hardware architectures or runtime/toolchain
versions are pinned yet. Choose a reference machine and representative Linux
test matrix before publishing performance/support claims. Linux development
on Fedora does not imply Fedora-specific product code.

Compare Linux distribution approaches against startup/resource needs, audio
access, SMB, filesystem portals and secret-store integration. Flatpak is a
candidate, not an approved packaging result. Native packages/portable bundles
remain alternatives. Windows packaging and future mobile store compatibility
need separate verification.

## License and third-party material

Project code is GPL-3.0-only, matching the selected GPLv3 direction. The full
unmodified license is in the repository root `LICENSE`. Official releases are
intended to be free of charge. Contributors retain their copyright; do not
invent ownership assignments or require a CLA without a project decision.

Record license and build-option evidence for libmpv/FFmpeg, libsmb2, SQLite,
the chosen UI toolkit, Lucide, fonts and bundled artwork before distribution.
Do not assume a library's default build has the same obligations as every
optional feature. Preserve required notices and provide corresponding source
and build information as required by the actual distributed combination. Review
analysis model weights, training-data terms and runtime libraries separately;
"free download" or personal/noncommercial use does not settle redistribution.

Imported Winamp skins keep their own rights; compatibility does not authorize
bundling a museum of third-party artwork. Use redistributable test fixtures.
Mobile distribution-channel/license compatibility remains a future review
item before selecting a release mechanism.

## Repository and generated files

Keep the implemented Cargo workspace reproducible. Commit source, manifests,
application lockfiles, migrations, useful build configuration and tests.
Keep build output, runtime profiles, logs, secret fixtures and editor state
out of Git. The root `/target/` ignore must not hide architecture target docs.

Document actual commands when they exist; do not publish nonfunctional setup
commands for uncreated manifests. Avoid adding a web package manager/runtime
before a web stack is selected. Do not require an IDE-specific build path.

## First implementation slices

1. Build [local controls and a queue](../../plans/local-desktop-player-plan.md).
2. Extend local audio: shuffle/repeat, EQ/presets, gapless, output recovery and
   measured latency/memory; separately prove crossfade feasibility.
3. UI: compact/library surfaces, accessibility/scaling and lists.
4. Local catalog: scans, browse/search and scale validation.
5. Accept integrated everyday local use, including playback during scans, queue
   restoration, device/file recovery and original safety. Minimal controls alone do not
   complete this gate; see the [roadmap](../../plans/product-roadmap.md).
6. Add Jellyfin browse/play, preserving local behavior
   and validating source isolation, recovery and bounded network work.
7. Then prove direct SMB reads/seek/cancellation and integrate the remote source,
   preserving local behavior. Neither remote source is prerequisite to local gates.
8. Packaging/test matrix and reproducible release evidence.

Reevaluate playlists after Jellyfin; implement only if needed, otherwise defer.
Skin packages/classic compatibility move near the end and do not gate sources.

These slices reflect the revised priorities. The core must remain reusable throughout.
Navidrome, tag editing/deletion, mobile/web remote control and
internet access follow their explicitly scoped phases.

The additive [product roadmap](../../plans/product-roadmap.md) sequences new
processing, analysis and connected features. Desired does not mean required in
the first release. Preserve these gates and the deferred SMB plan's bounded scope.

## Documentation lifecycle

Keep actual state in current/feature docs, decisions in focused target docs,
and navigation in small maps. Update all affected documents in the same task.
Every complete routine reading flow includes applicable guidance and stays
within 32 KiB. Run `python3 tools/check_docs.py` after documentation changes.
Full license texts and upstream specifications are reference materials,
not default architecture-reading context.
