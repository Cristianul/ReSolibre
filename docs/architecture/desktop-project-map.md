# Desktop map

[Rules](../../apps/desktop/AGENTS.md).

| Path | Purpose |
| --- | --- |
| [Entry](../../apps/desktop/src/main.rs) | Startup/close |
| [Reset](features/profile-reset.md) | Menu/profile reset |
| [EQ presets](features/named-eq-presets.md) | Presets |
| [Bindings](../../apps/desktop/src/presentation.rs) | [TRIM/FADE](features/playback-edges.md) |
| [Window hosts](features/detachable-playlist.md) | Library/snap/focus |
| [Import](features/local-file-import.md) | Drops/folders/chooser |
| [UI](../../apps/desktop/ui/player.slint) | [Analysis](features/track-analysis.md)/[batch](features/library-analysis.md) |
| Controls | [Gestures](features/playlist-interactions.md)/[search](features/library-search.md) |
| [Style](features/player-appearance.md) | [Tags](features/track-metadata.md); [info](features/track-information.md) |
| [Tests](../../apps/desktop/src/ui_tests.rs) | UI journeys |

[Feature](features/local-desktop-player.md), [setup](../development/desktop-environment.md),
[design](../design/compact-player.md), [validation](../testing/local-desktop-validation.md),
[plan](../plans/local-desktop-player-plan.md), [storage](features/session-persistence.md).

| Target | Read for |
| --- | --- |
| [Interface](target/interface-and-themes.md) | Skins/accessibility |
| [Audio/EQ](target/audio-and-equalizer.md) | Audio controls |
| [Library](target/library-and-playlists.md) | Catalog/storage/Info |
| [Processing](target/audio-processing.md) | Gain/normalization/crossfade |
| [Delivery](target/delivery-and-licensing.md) | Packaging/licenses |
| [Security](target/security-and-privacy.md) | Inputs/isolation |
| [Tests](../testing/test-plan.md) | Acceptance |

[Core targets](core-project-map.md).
