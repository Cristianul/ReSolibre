# Current architecture

Rust: portable domain (`resolibre-core`), analysis cache (`resolibre-storage`), Linux
libmpv/session (`resolibre-audio`), Slint (`resolibre-desktop`). Core has no GUI/native
dependencies; CLI plays local audio.

Desktop: import, transport/seek/gain, queue, Shuffle/Repeat and
[EQ/presets](features/local-equalizer.md). [Restoration](features/session-persistence.md)
never autoplays. One worker owns each read-only descriptor/engine.
Catalog, named playlists and SMB remain planned.

[Core map](core-project-map.md), [desktop map](desktop-project-map.md),
[adapter](features/local-playback.md), [desktop](features/local-desktop-player.md).
[Validation](../testing/local-desktop-validation.md): monitor evidence excludes
DAC latency; native cleanup has no hard cancellation bound.
