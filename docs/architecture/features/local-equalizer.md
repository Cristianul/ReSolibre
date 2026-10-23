# Local equalizer

## Read When

Changing shared EQ policy, native filters, live controls or named presets.
Follow the [core](../core-project-map.md)/[desktop](../desktop-project-map.md) rules.

## Do Not Read For

Normalization, spectrum visualization, crossfade or full library storage. See
[audio processing](../target/audio-processing.md) and the [local player](local-desktop-player.md).

## Current Behavior

Core owns one validated ten-band curve: 31.25, 62.5, 125, 250, 500 Hz and 1, 2, 4,
8, 16 kHz; peaking Q=1.4, gains -12..+12 dB, manual preamp -2..+2 dB (zero centered). Defaults
are flat/bypassed. Reset flattens gains and preamp while preserving enable state.
Bypass preserves the curve. Manual preamp: no automatic attenuation/limiter.
Positive gains can clip; preamp compensation remains manual.

The native session initializes a fixed double-precision libavfilter graph before
loading: preamp, ten peaks, then existing app volume/output adaptation. Bypass uses
unity gains in that graph, not a bit-perfect claim. At/above source Nyquist, bands
are unavailable, retain saved gains and receive no live commands. FFmpeg bypasses
out-of-range centers; the exact-Nyquist identity case is included in signal tests.

Live gains move toward confirmed intent at up to 60 dB/s without graph replacement.
Snapshots show targets, not sample-accurate convergence. A seek stores the
latest curve in the graph configuration before reset; runtime-only commands would
otherwise be lost. Audio reconfiguration invalidates observed rate/gains. Failure
stops with a safe error. Output/clipping evidence covers tested fixtures only.

Sliders support keyboard/accessibility and double-click reset to 0 dB.
Holding left drag across the EQ edits the hovered band at the pointer height;
gaps, unavailable bands and preamp are excluded. Release/cancel ends the gesture.
Folding, navigation and Stop/replay preserve EQ.
Flat/Bass/Voice/Bright use zero preamp. [Named presets](named-eq-presets.md) replace
fixed slots: saved-first selector, adjacent Save/name editor, confirmed Replace
and portable files. On/Off sits left; selector/Save right; no Reset/footer/title.
[Active EQ restores](session-persistence.md) without playing. Valid v1 preamps
outside -2..+2 clamp to the nearest limit; other session/preset data stays intact.

## Key Files

- `crates/resolibre-core/src/equalizer.rs`: values, actions, ranges, built-in curves.
- `crates/resolibre-audio/src/equalizer.rs`, `session/worker.rs`: native translation/lifecycle.
- `apps/desktop/src/equalizer.rs`, `preset_store.rs`: bindings and OS-user storage.
- `apps/desktop/ui/components/equalizer-panel.slint`, `controls.slint`: shared view/sliders.

## Tests

Core/session tests cover invalid values, bypass/reset, low rates, paused controls
and navigation. Preset tests cover bounded parsing, naming, migration, transfers and failed writes. Compiled UI tests cover vertical keyboard control
and focus, centered preamp/endpoints, legacy migration, reset and drag bounds; `native_equalizer_journey` exercises named Save/Load/panels.
`python3 tools/check_equalizer.py` captures 18 native response/peak/impulse/sweep cases and private
monitor live changes/paused seek. See the [execution record](../../plans/implemented/equalizer-plan.md)
for measurements, failures corrected and outstanding acceptance limits.

## Edge Cases

Non-finite/out-of-range gains and invalid indices are rejected before native work.
Adjacent edits coalesce only for the same control; different bands/actions preserve
order. Preset reads are <=4096 bytes and reject unknown versions, extra fields,
invalid UTF-8, symlinks and nonregular files. Scientific numeric serialization stays
bounded even for tiny finite values. No input becomes a filter expression or path.
No new dependencies, network, media writes, hidden compensation or telemetry.

## Related Docs

[Audio target](../target/audio-and-equalizer.md), [plan](../../plans/implemented/equalizer-plan.md),
[test plan](../../testing/test-plan.md), [desktop setup](../../development/desktop-environment.md).
Native semantics: [mpv filter commands](https://mpv.io/manual/stable/#filter-commands),
[FFmpeg equalizer](https://ffmpeg.org/ffmpeg-filters.html#equalizer).
