# Built-in player appearance

## Read When

Changing composition, palettes, chrome, controls or visual tests.
See [desktop map](../desktop-project-map.md), scoped rules and
[reference](../../design/compact-player.md).

## Do Not Read For

Audio DSP, session persistence, external skin loading, artwork decoding or window
positioning. See their owning modules.

## Current Behavior

Titles [scroll](playlist-interactions.md), flush left without hover fill or padding.
Seek spans transport/volume. SHFL/REP/TRIM/FADE align left; framed EQ/LIB align right.
TRIM is yellow.
A brush opens Themes. EQ overlaps beneath the player without rails or a gap;
Library stays separate. Both default visible; Green/Spectrum share behavior.
Shared frameless chrome drags from the title button and passive chassis, readout, cover
and EQ gaps, with 6 px resize borders. Main has minimize/maximize/Close; Library
drags from its thin top inset and passive margins/footer, with no title/X.
Analysis and Themes also drag from passive content and margins.
LIB hides it. Main Close saves/stops.
Short title clicks still reveal Library; other buttons, sliders, menus, row gestures
and resize edges keep their actions.
[Covers](track-metadata.md): 206² centered fill.
Hosts share 6 px transparent corners, a fine lip and shallow brushed faceplate.
Library uses recessed 36 px rows, no outer rim and a shaded footer. Buttons have
no focus rings; keyboard, hover/press and active states remain.
Dropdowns: dark inset, fine rim, white chevron; toolkit input.

Header menu: Theme → Green/Spectrum; shutdown rejects edits.
Green is default; theme/panel intent persists. Elapsed seek uses pink–violet–blue–cyan
in Spectrum, distinct from yellow TRIM overlays. Themes preserve audio/queue/EQ;
external skins are unsupported.

The shared dial increases on right/up, decreases on left/down with equal sensitivity;
diagonals add. Arrows increment, Home/End reach bounds, double-click resets to 50%.
Press never jumps gain. The session owns gain; local drag preview ends on release/
cancel. Seek/EQ reuse bounded controls. EQ supports painting across bands and
double-click to zero; faders show gain/frequency. Preamp remains manual.

EQ follows the Spectrum reference: dark inset, long filled rails, 13 ticks and
18×26 px metal caps. Values replace frequency labels on hover/drag/focus; preamp
has a ±2 dB scale. On/Off sits left, presets/Save right; no Reset/heading/footer. Caps
use thin sockets; circles are antialiased. Plates retain 64 faint lines.
Volume matches Play's 76 px diameter, without a caption; pointer and Spectrum segments remain.
Forest: 1254² PNG (~6 MiB), not metadata. Brand master: `docs/design/icon.png`;
256² runtime copy: `ui/assets/resolibre.png` (256 KiB). `branding.slint` shares header/
native icons; XDG ID: `resolibre`. Install checkout launcher: `python3 tools/install_desktop.py`.

EQ slides behind the stationary case in 300 ms with cubic-bezier(0.4, 0, 0.2, 1).
Reversals continue from current progress. Native size expands before opening and
shrinks after closing; one timer cancels obsolete resizes. Closing disables EQ
input without bypassing DSP. `RESOUL_REDUCED_MOTION=1` removes the transition.

## Key Files

- `ui/player.slint`, `src/presentation.rs`: layout and deferred EQ sizing.
- `src/themes.rs`, `ui/themes-window.slint`: chooser lifecycle, selection and previews.
- `ui/components/window-header.slint`: header, chassis and pull-out drawer.
- `ui/components/theme.slint`: semantic palette and one frequency color mapping.
- `ui/components/controls.slint`: PanelKey legends/LEDs, buttons and shared input.
- `ui/components/instruments.slint`: passive keys, readouts, metal, dial and artwork.
- `ui/components/icon.slint`: shared Lucide wrapper, re-exported by controls.
- `ui/components/equalizer-panel.slint`, `queue-row.slint`: reusable views.
- `src/{visual_tests,ui_tests,drawer_tests}.rs`: interaction/native coverage.

## Tests

[Validation and decisions](../../testing/player-appearance-validation.md).
Compiled controls check chooser reuse/close/shutdown, selection isolation, keyboard,
defaults, palette isolation, seek generation and EQ sweeps. Isolated native renders
cover both palettes, 760/800 widths, 1x/2x, Unicode/long titles, empty/error states
and detached hosts. Native silent playback exercises panel/palette changes and EQ.
Pointer tests cover passive drag routing, control clicks and disabled movement.

## Edge Cases

Rows are virtualized; shapes/images are bounded. Filenames are sanitized text;
no network or source writes. Covers use bounded worker thumbnails.
Full assistive-technology, mixed-monitor DPI and physical listening acceptance
remain separate from compiled/native rendering evidence.

## Related Docs

[Local player](local-desktop-player.md), [detachment](detachable-playlist.md),
[interface target](../target/interface-and-themes.md),
[execution record](../../plans/implemented/player-visual-polish-plan.md).
