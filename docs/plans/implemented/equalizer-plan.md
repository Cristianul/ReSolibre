# Local equalizer implementation record

Status: implemented, scoped checks passed, 2026-10-22. User approved the local EQ
slice and chose manual preamp only. No automatic compensation or hidden limiter.

## Delivered contract

Portable ten-band values/actions, native double-precision filters and bounded live
updates; manual preamp, bypass/reset, built-in curves and three explicitly saved
preset slots. The exclusive panel reuses shared sliders; storage runs on an owned
background worker outside music. [Feature contract](../../architecture/features/local-equalizer.md)
owns ranges, source-rate policy, file format, errors and security boundaries.
No normalization, crossfade, catalog, spectrum animation, skins or SMB was added.

## Evidence

- Workspace/all-target tests: 46 passed, 11 opt-in tests excluded from the default
  run. Core-only build, desktop build, rustfmt and all-target Clippy `-D warnings`
  passed. Python regressions: 23 passed. Documentation/whitespace checks passed.
- `tools/check_equalizer.py`: 18 real native captures. Centers/preamp/bypass and
  combined gains within 0.10 dB; manual overload/recovery and 8 kHz unavailable
  bands passed. Impulse/sweep maximum reference error 0.0000153 full scale.
  Four live ramps and paused seek passed private-monitor checks; the 1 kHz
  discontinuity residual stayed below 0.005. Observation allows 1.3 s for ramp,
  buffering and a settled window; this is not a measured EQ latency guarantee.
- Explicit FLAC/MP3 with EQ passed silent-output journeys. SHA-256, size, mtime
  and mode unchanged. Preset tests reject malformed/oversized/versioned data,
  non-finite values, symlinks/FIFO and failed replacement; tiny values round-trip.
- Native Save/Load/panel journey and compact/EQ screenshots passed; selector
  bounds corrected after inspection. Vertical keyboard/focus tests include 2x
  scaling at 760x710. Twelve native session cycles/1,080 panel toggles: 188.926 MiB
  peak RSS; callback-group p95 16.239 ms, max 19.019 ms (<50 ms gate).
- EQ-enabled 60 open/control/seek/Stop cycles: 85.230 MiB peak RSS. Private-monitor
  live EQ: ~73 MiB, ~2% one-core CPU. Existing session smoke: three WAV/three FLAC
  marker journeys plus protocol/output-loss negatives; three 60-cycle resource
  runs per codec, maxima 78.36/79.66 MiB. Volume 0/50/100% captures passed.
  This does not repeat the older 100-trial latency acceptance gate.

Reference: Linux x86_64, debug/libmpv 0.41/Slint Winit-FemtoVG. Process RSS sampled
at 20 ms for UI/EQ cycles; external portal/GPU services excluded. UI harness uses
glibc mmap threshold 65536; audio 131072 before threads. Budgets: 200/100 MiB.
No new dependencies; bounded numeric settings never become untrusted native syntax.

## Failures corrected and limits

Immediate commands to a newly inserted paused filter failed before initialization;
configure the fixed graph before decoding and wait for source/output negotiation.
Signal capture found runtime filter edits lost at seek: persist the latest curve
into graph configuration before reset. An untimed short 8 kHz capture could finish
while paused during setup; static captures now start already configured, without
that pause. Twenty repeated low-rate captures preserved all frames; frame counts
remain asserted rather than accepting empty/partial evidence. Early live windows
included buffered transition output; the settled-window contract is now explicit.

Manual listening, physical devices, full accessibility and broader native/kernel
stall bounds remain acceptance work in the [base plan](../local-desktop-player-plan.md).
No universal inaudibility, clipping prevention, bit-perfect or release-support
claim. Saved slots do not restore active EQ/playback automatically. Reading profile:
`equalizer`. Temporary measurements/screenshots stay ignored under `.local`.
