# Product roadmap

Status: active planning; features and dependencies are not implemented by this
roadmap. SMB scope is unchanged. Archive only completed execution plans.

## Product intent and status vocabulary

Personal, free/open-source music player under the existing GPL-3.0-only license.
Commercial success, monetization and community adoption are not requirements.
Offline-first, useful without ReSolibre accounts, internet or a companion server;
online sources may require their own credentials. No mandatory telemetry or
external requests for local functionality. Optional networking never changes
local ownership or the original-file protection rules.

Implemented means present in code with documented evidence and limits. Planned
means desired but unimplemented. Experimental means quality/feasibility still
needs proof; optional integrations are not prerequisites. Future targets are
not a promise of first-release delivery or a new platform support claim.

## Requirement and ownership map

| Requirement group | Current status / sequencing | Canonical specification |
| --- | --- | --- |
| Local playback, pause/seek, formats | Linux adapter and local desktop implemented; evidence has limits | [Current feature](../architecture/features/local-playback.md) |
| Shared playback/domain logic | Portable values exist; reusable behavior grows incrementally | [Core/platforms](../architecture/target/core-and-platforms.md) |
| Usable local desktop player | Controls/queue/session restore implemented; acceptance in progress | [Execution plan](local-desktop-player-plan.md) |
| Direct SMB2/3 | Deferred until local acceptance and Jellyfin; no implementation | [Execution plan](smb-playback-plan.md) |
| Device adaptation, gapless, ten-band EQ/presets | Adaptation/local EQ implemented; gapless planned | [Audio/EQ](../architecture/target/audio-and-equalizer.md) |
| App-only volume, global normalization, crossfade, signal path | App volume implemented; other processing planned | [Processing](../architecture/target/audio-processing.md) |
| Beat-aligned crossfade with bounded tempo adjustment | Desired later enhancement; detector/stretching experimental | [Processing](../architecture/target/audio-processing.md) |
| Large catalog, native playlists, M3U/M3U8 | Catalog first; playlists/M3U reevaluated after Jellyfin, deferred unless needed | [Library/playlists](../architecture/target/library-and-playlists.md) |
| Stable IDs, missing references | Minimal contracts before persistence; advanced matching deferred | [Library/playlists](../architecture/target/library-and-playlists.md) |
| Duplicate candidates, relinking, library health | Desired enhancements; detailed cross-source identity design later | [Library/playlists](../architecture/target/library-and-playlists.md) |
| Jellyfin then Navidrome; external playlist comparison/sync | Jellyfin before SMB; Navidrome after SMB; read/import before bidirectional writes | [Sources](../architecture/target/library-and-sources.md), [playlists](../architecture/target/library-and-playlists.md) |
| Individual/bulk tags, artwork, filename transforms | Desired enhancements after read-only baseline | [Metadata editing](../architecture/target/metadata-editing.md) |
| Track details, spectrogram and live spectrum | Requested next; [focused plan](track-analysis-plan.md), PCM feasibility first | [Analysis/radio](../architecture/target/analysis-and-radio.md) |
| Offline Track Radio, detailed genres, optional recommendations | Medium/future; models and quality experimental | [Analysis/radio](../architecture/target/analysis-and-radio.md) |
| Smart playlists, playlist history/undo | Smart playlists later; history/undo lower priority | [Library/playlists](../architecture/target/library-and-playlists.md) |
| Internet radio; hardware radio and timeshift | Internet radio medium/future; hardware/buffer features lower priority | [Analysis/radio](../architecture/target/analysis-and-radio.md) |
| Compact/library UI, themes, classic skins | Layouts first; skin packages/classic compatibility near the end | [Interface/themes](../architecture/target/interface-and-themes.md) |
| Android, secure LAN remote, target-device library browsing | Future client phase; no mandatory account/server | [Remote](../architecture/target/remote-control.md) |
| Handoff, file transfer, companion server, web/cloud | Later optional capabilities; separate from remote control | [Remote](../architecture/target/remote-control.md) |
| Windows, possible macOS/iOS, browser constraints | Linux first, Windows next; others need platform evidence | [Delivery](../architecture/target/delivery-and-licensing.md) |
| Spotify / YouTube Music | Exploratory only, subject to authorized APIs and terms | [Sources](../architecture/target/library-and-sources.md) |

## Immediate work

Finish metadata persistence, cached batch analysis and startup/resource checks first.
Next: reproduce MIX tempo-transition artifacts reported on 2026-10-23: short cracks
at reduced BPM and voice/timbre changes while BPM rises. Diagnose before changing DSP.

## Incremental delivery

1. Build the [local desktop milestone](local-desktop-player-plan.md): file selection,
   play/pause/stop, time/seek, app volume and a temporary queue. Validate the supplied
   MP3/FLAC and generated fixtures. This starts the local phase; it does not unlock SMB.
2. Next/previous, queue editing, Shuffle/Repeat and local ten-band EQ/presets exist.
   [Session restoration](../architecture/features/session-persistence.md) now exists. Add compatible gapless. Prove ordinary crossfade separately and define
   normalization/diagnostic boundaries before fixing the processing pipeline.
3. Build compact/library layouts, keyboard/accessibility and scaling. Add local
   folder import, bounded scanning and browse/search; validate catalog scale.
   Keep stable track IDs and missing references. Skin packages/classic compatibility
   move near the end; saved playlists and M3U are not prerequisites.
4. Accept the local player for everyday use: open music, browse/search,
   edit the queue, control playback/EQ and recover from file/output failures.
   Verify both layouts, playback during scans, resource bounds and unchanged music.
   Record passing integrated journeys and remaining limits. Keep local use independent
   of networking; completing only the minimal window is insufficient for this gate.
5. Add Jellyfin browse/play. Validate source/credential isolation, bounded network
   work, failure recovery and local regressions. Then reevaluate playlist needs:
   implement only what is needed; otherwise defer native/service playlists, copies
   and M3U further. Existing queue/session restoration remains available.
6. Then resume [direct SMB feasibility](smb-playback-plan.md): bounded reads,
   cancellation, seek, authentication, recovery and measured resources. Keep its
   scope unchanged, then integrate SMB into the working player with local regressions.
   Preserve packaging/licensing and reproducible Linux release gates; EQ remains
   required in each client's first release. Confirm remaining release scope there.
7. Add signal-path detail, normalization/crossfade beyond feasibility, bounded analysis,
   spectral tools and metadata/artwork workflows incrementally. Advanced matching,
   relinking and duplicates remain later. These enhancements need not all precede SMB.
8. Add Navidrome browse/play. Playlist features follow the post-Jellyfin needs
   decision; external changes/manual synchronization have their own phases.
9. Add Android and secure LAN control when client work starts. Explore offline Track
   Radio, genres, smart playlists and internet radio as independent slices.
10. Add skin packages and classic Winamp compatibility near the end, with their
   archive/input safety and UI regression checks.
11. Evaluate beat sync, automatic playlist sync, handoff, file transfer, companion
   hosting and broader internet access only in scoped later plans.

This ordering updates the [delivery gates](../architecture/target/delivery-and-licensing.md).
It is not a deadline or a claim that one developer should build every subsystem
simultaneously. Each implementation slice needs its own acceptance evidence.

## Smaller future conveniences and exclusions

Retain lower-priority CUE sheets/album-long files with track markers; plain and
synchronized LRC lyrics where feasible; a sleep timer with configurable fade-out;
bookmarks within long tracks/DJ sets; and audio output device
selection. Local session restoration exists without autoplay; remote restoration
remains deferred. Output selection is distinct from required output compatibility/recovery.

Do not introduce ABX comparisons, decorative 3D/MilkDrop visualizers or multiple
independent ambient-mixer sessions. Useful live spectrum remains in scope.
Classic Winamp Modern/MAKI execution stays deferred; existing explicit future
deletion workflows are preserved, not expanded into automatic cleanup.

## Decisions and evidence

[Open decisions](../architecture/target/open-decisions.md) tracks unresolved choices.
Same-album crossfade exemption and album-based normalization are not open options:
the accepted policy applies regardless of album. Reusing common implementation is
the default; divergence needs a demonstrated platform/build/lifecycle constraint.
Keeping selected older analysis algorithms is possible future work, not a promise
to maintain every algorithm or introduce a versioning framework now.

[General tests](../testing/test-plan.md) and
[expanded acceptance cases](../testing/expanded-features-test-plan.md) are obligations,
not implemented tests or benchmark results. New docs must preserve the complete
32 KiB reading-flow budget and current-state/target distinction.
