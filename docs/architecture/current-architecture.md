# Current architecture

Rust workspace: portable core, local SQLite cache, libmpv/session adapter, Slint UI.
Core excludes GUI/native dependencies. Each worker owns its read-only file/engine.
Local transport, queue, EQ/presets and restoration work; restore never autoplays.
Catalog, named playlists and SMB remain planned. Native cleanup has no hard deadline;
device latency is unproven.

[Playback](features/local-playback.md), [UI](features/local-desktop-player.md),
[Windows x64](../development/windows-native.md); Live Spectrum is Linux-only.
