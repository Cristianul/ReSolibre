# Core map

[Rules](../../crates/AGENTS.md).

| Path | Purpose |
| --- | --- |
| [Queue](../../crates/resolibre-core/src/queue.rs) | Edits/[replacement](features/local-file-import.md); [order](features/queue-playback-order.md) |
| [Session](../../crates/resolibre-audio/src/session/mod.rs) | [TRIM/FADE](features/playback-edges.md) |
| [Audio](../../crates/resolibre-audio/src/lib.rs) | libmpv; [MIX](features/dj-mix.md) |
| [FFI](../../crates/resolibre-audio/src/ffi.rs) | API |
| [Platform I/O](../../crates/resolibre-audio/src/platform.rs) | [Windows](../development/windows-native.md) retained files/pipes |
| [Analysis](features/track-analysis.md) | [Live](features/live-spectrum.md)/[cache](features/analysis-cache.md)/[batch](features/library-analysis.md) |
| [Metadata](features/track-metadata.md) | Tags/covers |
| [local-play](../../crates/resolibre-audio/examples/local-play.rs) | CLI/memory |
| [Tests](../../crates/resolibre-audio/tests/local_playback.rs) | Playback |
| [CLI](../../tools/check_local_playback.py) | Isolation |
| [PCM check](../../tools/check_pcm_conversion.py) | [Signals](../../tools/pcm_signal.py) |

[Playback](features/local-playback.md).
[Desktop](features/local-desktop-player.md); [restore](features/session-persistence.md).
[Controls](../testing/playback-responsiveness-validation.md).
[Plan](../plans/local-desktop-player-plan.md); [SMB](../plans/smb-playback-plan.md) planned.

| Target | Read for |
| --- | --- |
| [Core/platforms](target/core-and-platforms.md) | Contracts |
| [Audio/EQ](target/audio-and-equalizer.md) | Output/controls |
| [Sources](target/library-and-sources.md) | SMB/access |
| [Library/playlists](target/library-and-playlists.md) | Catalog/storage |
| [Processing](target/audio-processing.md) | Gain/transitions |
| [Analysis/radio](target/analysis-and-radio.md) | Jobs/spectra/radio |
| [Metadata](target/metadata-editing.md) | Confirmed tag/art writes |
| [Security](target/security-and-privacy.md) | Trust/inputs |
| [Remote](target/remote-control.md) | Paired control |
| [Tests](../testing/test-plan.md) | Verification |
