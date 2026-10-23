# Player appearance validation

2026-10-22, Linux x86_64, Rust debug, Slint 1.18.1/FemtoVG.
Read with the [feature](../architecture/features/player-appearance.md).

## Frameless follow-up

Native title dragging/6 px borders, square art at 760/860/1200 widths, reachable
controls, window actions, cancellation/shutdown and Playlist scroll passed.
X11/Wayland captures omit decorations; mixed-frame snap passes at 1x/2x. Missing
KWin extents require zero extents for frameless clients; decorated peers use live
bounded extents. No global input hooks.

Earlier opaque debug: 194.06 MiB peak/13.67 s, sampled every 20 ms, 12 silent
playback cycles; callback p95 32.29/max 39.79 ms. A prior transparent variant
failed 200 MiB at 202.41 MiB. Logs: `.local/chrome-real-playback-memory.json`,
`.local/chrome-real-playback.log`.

## Compact surface follow-up

The outer frame and footer were removed (then 264/508/608 px); secondary controls share 26 px targets with 14 px icons. Both hosts
use transparent backgrounds with 6 px corners. Native captures assert zero alpha
at the outer corner on X11/Wayland; minimum-width and 2x captures remain readable.

Earlier single-window debug: idle Wayland 178.40/X11 180.97 MiB; 12-cycle playback
202.70 MiB peak (over the 200 MiB gate), p95 38.08/max 43.79 ms, 20 ms sampling over
13.23 seconds with production mmap threshold. Logs: `.local/compact-real-*.json`
and `.local/compact-real-playback.log`. Memory optimization remains open.

## Separate Playlist and fixed EQ drawer

Both start visible; PL/EQ independently hide/show them. Main heights: 264/516 px;
Playlist: 420×516 px. Native checks cover initial right docking and moved-position
retention through hide/show on X11, plus separate-window lifecycle on Wayland.
The playback journey now uses X11/XWayland, matching normal startup.

Earlier two-window idle, private empty profiles, 6 s/250 ms sampling: X11 188.35,
Wayland 182.70 MiB. Wayland playback: 208.01 MiB/13.37 s, p95 24.05/max 30.20 ms.
Pre-chooser X11: 209.19 MiB/12.81 s, 12 cycles/20 ms sampling, p95 22.29/max 31.75 ms.
Both exceed 200 MiB; timing passes 50 ms. mmap threshold: 128 KiB. Short workloads;
optimization remains open. Logs: `.local/drawer-real-idle-memory.json`,
`.local/drawer-real-playback*.json`, `.local/drawer-wayland-playback-memory.json`.

## Executed checks

- Earlier frontal-cap follow-up: 39 desktop passed, ten opt-in excluded; 14 native
  cases covered snap/focus/docking, X11/Wayland hosts, palettes at 760/800 widths,
  1x/2x, Unicode/empty/error states, playback, edits, modes, EQ, Themes and drops.
- Inspected native captures: [Compact](../design/player-compact.png), [Green](../design/player-green.png),
  [Spectrum](../design/player-spectrum.png), [Playlist](../design/player-playlist.png), [Themes](../design/player-themes.png).
  Frontal caps, fine rims and antialiased circles inspected at 1x/2x. No new RSS run.
- Earlier material baseline: idle Wayland 177.00 MiB/X11 177.15 MiB; native silent
  playback 192.96 MiB peak over 12.03 seconds, 20 ms sampling, 12 cycles; callback
  p95 22.25 ms/max 29.39 ms. Production mmap threshold: 128 KiB. Short workloads
  only, not sustained playback/scan acceptance.
- Themes: lazy creation/reuse, immediate selection, focus return, shutdown guards;
  native preview and palette changes during playback pass.
- Native size assertions caught stale minimum heights; resize waits for constraints.
- Desktop build/Clippy, formatter and docs checks pass.

## Sliding drawer

46 desktop/18 native cases passed; build, Clippy, format and docs pass.

EQ precedes PL; the narrower case overlaps behind the player, without feet/gap.
Compiled tests cover 300 ms progress, reversal continuity, disabled closing input,
stationary transport, reduced motion and unchanged EQ/time. Native 1x/2x checks
capture opening/closing, assert opaque seam pixels, delayed collapse and stale
resize cancellation. Static palette captures use reduced motion. No new source
access, dependency, storage or DSP behavior; no new RSS/physical latency claim.

## Earlier baseline

Before the material/illustration follow-up: real desktop production idle 169–170
MiB, connected playback test 191.66 MiB. Virtual KWin with forced software rendering
measured 207–209 MiB idle, exceeding 200 MiB. That failure remains recorded; current idle figures use hardware rendering. Full scan/device/resource acceptance remains open.

## Reproduction

Use the [desktop environment](../development/desktop-environment.md):

```sh
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo build -p resolibre-desktop --locked
python3 tools/check_window_snap.py
python3 tools/check_docs.py
```

The native harness owns a private compositor/profile and writes ignored logs/PNGs
under `.local/window-snap`. Optional `RESOUL_VISUAL_APP` measures software-rendered
idle RSS separately. Material baseline logs:
`.local/material-real-memory.json`, `.local/material-real-playback-memory.json`
and `.local/material-real-playback.log`. Tests never inject global input or change
the user's saved session.

## Trust and limits

Icons retain the bundled Lucide license. One fixed embedded PNG
uses the existing toolkit decoder (about 6 MiB decoded);
shapes and queue virtualization remain bounded. No new dependencies, downloads,
settings parser, remote artwork or music writes. Fixtures remain preserved.
Physical drag/edge-resize, full screen-reader/IME, mixed-monitor DPI, sustained
resource/device and physical listening checks are not claimed. Palette selection now persists; real track
cover decoding remains future work.
