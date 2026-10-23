# Player visual polish execution plan

2026-10-22. User requested analysis, a plan and implementation against the saved
Green/Spectrum concepts, and explicitly delegated design decisions while away.

## Analysis and scope

Working controls lacked the references' recessed display, circular transport,
volume dial and calibrated EQ. Preserve compact/Playlist/EQ geometry, exclusive
lower panels, detachment and existing playback/queue actions.

## Executed steps

1. Refine shared tokens/buttons/sliders; add passive dial and record faces using
   bounded toolkit shapes, without dependencies, gradients or idle animation.
2. Recompose the player; reuse slider input for rotary volume, preserving keyboard,
   accessibility, double-click, seek capture and EQ sweep gestures.
3. Add Green/Spectrum accents, fader scales and quieter shared playlist rows.
4. Verify input regressions, native renders at minimum/default sizes and 2x,
   playback/EQ/detachment, memory, build/Clippy/format and docs.
5. Update feature/design docs, record decisions and archive this completed plan.

## Delegated decisions

- Green is the default; Spectrum colors EQ groups and volume segments. It is a
  built-in palette selector, not an external skin loader or audio effect.
- Palette selection is per-run for this slice; the existing audio/session storage
  format is unchanged. A future UI preferences owner can persist visual settings.
- Use a static record placeholder: no invented artwork, artist, format or meters.
- Rotary volume uses vertical dragging, arrows/Home/End and double-click to 50%;
  its center displays the value. It controls app gain, never the system mixer.
- Keep native window decoration and the existing optional --x11 launch behavior.
- No added library/favorite/menu controls until their actions exist.
- Retain 366/610/710 logical pixel heights; EQ needs space for presets/status.
  Keep the record area at 172 px to avoid cyclic window/layout size dependencies.

## Status

Implemented and verified. Workspace: 65 tests pass; desktop follow-up: 23 pass.
Eight opt-in native runs cover snap/hosts, 16 layout/palette captures and connected
playback/EQ. Build, all-target Clippy with warnings denied, formatting, whitespace
and complete documentation reading flows pass.

Real desktop: 169–170 MiB idle production RSS; 191.66 MiB peak in the compiled
12-cycle playback journey. The isolated software-rendered idle run exceeds 200 MiB
(~207–209 MiB); it is recorded separately, not called a budget pass. Full resource
and assistive-technology acceptance remains open beyond this visual slice.

Corrected native-render findings: palette caption clipping, transport/icon centering,
EQ heading disappearance at minimum width and a cyclic placeholder-size binding.
See [current feature](../../architecture/features/player-appearance.md) and
[validation/captures](../../testing/player-appearance-validation.md).

The subsequent material/illustration refinement supersedes the flat placeholder
decision; see the current feature and validation above for its scope and evidence.
