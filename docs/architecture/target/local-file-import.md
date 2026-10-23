# Local file import target

User-requested scope: files and folders dropped on Player/Playlist offer Add or
Replace; an empty queue adds directly. Keep one reusable import/action owner.

Add preserves playback; Replace stops and selects the first new occurrence.
Neither operation autoplays. Recursively expand folders in deterministic order
off UI/audio threads. Import audio/video candidates for audio-only playback;
silently ignore unrelated files/links and empty results. Failed/oversized batches
preserve the queue.
Keep the existing 1024-file/1 MiB queue boundary; cancellation and errors remain
observable. Do not write music, decode metadata or expand URLs/playlists.

[Implementation and bounds](../features/local-file-import.md) describe the current
X11/XWayland path. Native Wayland delivery, catalog indexing and M3U are separate work.
