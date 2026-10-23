# Cross-area feature index

Implemented: [audio](local-playback.md), [desktop](local-desktop-player.md),
[order](queue-playback-order.md), [queue gestures](playlist-interactions.md), [search](library-search.md), [hold scanning](transport-scan.md), [EQ](local-equalizer.md), [TRIM/FADE](playback-edges.md), [MIX](dj-mix.md). Targets:

| Planned flow | Target |
| --- | --- |
| UI → session → source → output, including EQ | [Audio](../target/audio-and-equalizer.md) |
| UI → source → catalog, including future confirmed edits | [Library](../target/library-and-sources.md) |
| Skin import/preferences → layouts → shared session | [Interface](../target/interface-and-themes.md) |
| Controller → pairing/transport → player | [Remote](../target/remote-control.md) |

Update leaves with code. Register complete reading
flows in `tools/check_docs.py` (32 KiB including guidance).

[Session persistence](session-persistence.md): saved intent.
[Profile reset](profile-reset.md): menu/reset.

[Detachable Library](detachable-playlist.md): queue/snapping.

[File drops](local-file-import.md): folders/Add/Replace.

[Track metadata](track-metadata.md): labels/covers/durations.

[Facts](track-information.md); [live spectrum](live-spectrum.md); [spectrogram/BPM](track-analysis.md); [persistent cache](analysis-cache.md); [Library analysis](library-analysis.md).
