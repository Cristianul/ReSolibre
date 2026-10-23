# Target architecture summary

## Product and priorities

A personal GPL-3.0-only player; monetization is not required. Local use needs no
ReSolibre account, cloud, companion server, telemetry or external requests.
Sources may need credentials. Users own their music and playlists.

Linux first (Fedora, Debian, Ubuntu), with a native Windows x64 prototype. Android follows; macOS/iOS
where feasible and an offline-capable web client are later targets. Source order
is local features/library first, then Jellyfin, SMB and Navidrome. Spotify/YouTube
Music are exploratory only, subject to authorized APIs and terms.
The [roadmap](../plans/product-roadmap.md) separates current, planned and experimental
work; desired does not mean required at first release.

## Shared implementation

Use a local modular monolith with shared Rust behavior and explicit source/audio/
storage/platform capabilities. Implement common playback, queue and application
rules once; reuse DSP/analysis and native engine integration wherever feasible.
Separate only demonstrated platform differences. Shared code can span crates:
portable core builds stay independent of native libraries and GUI toolkits.

Keep libmpv, libsmb2 and SQLite. Slint needs accessibility, skin and performance
evidence. Native clients embed engines; browsers may need different adapters.
Do not create unused future-client scaffolding.
See [core/platforms](target/core-and-platforms.md).

## Playback and processing

Adapt to supported output with practical fidelity; do not reject playable music
for purity or claim bit-perfect output without evidence. EQ remains in each
client's first release: ten bands, preamp, bypass, presets and explicit headroom.
[Audio/EQ](target/audio-and-equalizer.md) owns controls, output and feasibility.

[Processing](target/audio-processing.md) adds app-only Master Volume (100% unity;
never OS/global volume), one normalization toggle applying independently to every
track, configurable crossfade regardless of album, and an actual Audio Signal
Path view. Compatible gapless remains available. Later optional beat sync aligns
timing/tempo within a user percentage limit and falls back to ordinary crossfade
when unsuitable. App volume works; other processing remains planned.

## Library, tools and performance

[Sources](target/library-and-sources.md) stay modular and independently usable.
[Library/playlists](target/library-and-playlists.md) owns native local persistence,
M3U and service playlists, reevaluated after Jellyfin. Establish stable IDs,
ordered repeated occurrences and missing-reference retention before persistence.
Advanced recording/version matching, duplicates and relinking are deferred;
paths must not become track identities in the meantime.

Target hundreds of thousands of entries, with one million as a stress scenario.
Use incremental scans, indexed pages, virtualized UI and bounded caches.
Prioritize audio/controls over analysis; measure seek output separately from UI
feedback. Never load the whole catalog/artwork into RAM.
[Analysis/radio](target/analysis-and-radio.md) covers background scheduling, useful
spectrograms, cautious provenance estimates, experimental offline Track Radio,
optional recommendations, internet radio and later hardware/timeshift. Keep caches
simple; retaining selectable older algorithms is an open future possibility.

## Interface and trust

Compact Winamp-inspired and library layouts share session/queue/EQ. Keep folder/
artist/album browsing, search, drag/drop, shuffle/repeat and playlists. Themes/layouts
preserve playback. ReSolibre themes support both; classic .wsz targets compact mode.
MAKI/Modern execution stays deferred. Use Lucide for ReSolibre controls; imported art
retains its rights. Keep advanced tools contextual.
See [interface/themes](target/interface-and-themes.md).

Ordinary use never writes, retags, renames or deletes original music. Keep app data
separate. [Metadata editing](target/metadata-editing.md) plans individual/bulk tags,
artwork and filename transforms through explicit preview/confirmed writes with
recovery, after the read-only baseline. Deletion is a separate confirmed workflow.
Validate untrusted media, tags, playlists, skins, models and stored data; bound
resources, isolate sources/profiles, protect credentials and private diagnostics.
See [security/privacy](target/security-and-privacy.md).

## Connected clients and delivery

[Remote](target/remote-control.md): account-free paired LAN control comes with
mobile/web work, internet later. The target device owns its library, queue, output
and app volume. Handoff and file transfer are distinct later capabilities. An
optional companion/web/cloud must never become necessary for local playback or
direct LAN control.

Skins move near the end. Local UI/catalog, then Jellyfin and SMB: [delivery](target/delivery-and-licensing.md),
[decisions](target/open-decisions.md), [tests](../testing/test-plan.md).
Current docs describe implementation; complete reading flows stay within 32 KiB.
