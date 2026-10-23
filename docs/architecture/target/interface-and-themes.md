# Interface and themes target

Local desktop RSS budget: 200 MiB on the reference machine; audio-only 100 MiB
(user approved 2026-10-22). Startup target: under one second with restored
theme, EQ and Library ready at first appearance (2026-10-23).

## Initial local milestone

Build [local controls and a temporary queue](../../plans/local-desktop-player-plan.md)
first, then local layouts and catalog before Jellyfin/SMB. Closing stops playback;
tray is later. [Approved design](../../design/compact-player.md): sculpted hardware, Green/Spectrum,
a sliding EQ drawer, separate Library, square cover art, 6 px corners and
controls, native drag/resize; Library has only Close. English UI; localization open.

[Detached Playlist](../features/detachable-playlist.md): same session, 5 px snap to
owned windows. Main carries snapped peers; Playlist drag detaches. X11/XWayland.

## Two layouts, one session

Compact mode is Winamp-inspired: transport, track/time, playlist and EQ.
Modern library mode provides source navigation, artist/album views, search,
playlists and queue. Both are required desktop directions. Switching layout or
theme preserves track, position, queue, shuffle/repeat, volume and EQ; it never
creates a second engine. Theme and EQ/Library visibility also survive restarts.

Later views expose Edit Tags, Analyze Audio
and Audio Signal Path; playback settings expose EQ, normalization and crossfade.
Advanced tools must not crowd the default layout. Crossfade may use two internal streams within
the same logical session; changing layouts still never creates a second session.

Slint is the initial candidate. Prove bitmap/region rendering, scaling, Unicode,
large lists, accessibility and desktop integration; consider Qt if requirements
fail. Embedding a browser for skins needs an explicit lightweight-design decision.

## ReSolibre themes

Skin packages and classic compatibility move near the end of the roadmap; they
do not gate Jellyfin/SMB. Existing built-in appearance remains available.

Use versioned declarative packages: semantic colors, typography, spacing,
geometry and licensed assets. Approved layouts provide compact/spacious variants;
arbitrary executable UI is out of scope. Validate before switching and retain
a built-in fallback. Shared controls own interactions/accessibility.

Lucide supplies icons through one primitive. The built-in hardware skin permits
material shading. Buttons omit focus rings but retain keyboard actions. Keep state
indicators solid, without edge glows; preserve artwork and reduced motion.

## Classic compatibility

Initially render Winamp 2 `.wsz` assets/metadata in compact mode; extracting
colors into a generic theme is insufficient. Test main controls, time/text, EQ,
playlist, optional assets, scaling and region/shape behavior in a representative
corpus. Document unsupported cases instead of claiming universal compatibility.

Classic skins do not define library mode. Use the selected ReSolibre theme/fallback
there, retaining the chosen classic skin for return to compact mode. Defer
Winamp Modern `.wal`/XML/MAKI and its dynamic scripting; never silently execute it.

Bound archive entries/expanded sizes and image decoding. Reject unsafe paths,
symlinks and duplicate/case collisions. Do not fetch remote resources, execute
scripts or extract outside application-owned storage. Rejection preserves
usable UI and audio. Bundle only appropriately licensed artwork/fixtures;
format compatibility grants no skin-redistribution rights.

## Accessibility and future clients

Provide keyboard controls, named buttons, slider increments and accessible
state independent of bitmap text. Verify long/Unicode filenames, IME input,
focus restoration and DPI/scaling. Keep classic controls usable at supported
zoom while preserving their sprite artwork.

Externalize strings. Validate future touch/browser clients separately; apply the
React/TypeScript ruleset if selected for web.

References: [classic rendering](https://docs.webamp.org/docs/features/skins/),
[Modern structure](https://github.com/WACUP/Winamp-Skinning-Archive/blob/master/Modern%20Skins/README.md).
Design references; acceptance follows the [test plan](../../testing/test-plan.md).
