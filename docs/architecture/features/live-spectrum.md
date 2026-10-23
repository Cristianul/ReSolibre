# Live spectrum

## Read When

Changing live spectrum transport or motion. Use root/core/desktop rules, maps,
security and tests (`live-spectrum` profile). FFT mathematics has a separate
[feature](spectral-analysis.md).

## Do Not Read For

Full-track spectrograms, provenance, tempo or mobile playback ports. See the
[active plan](../../plans/track-analysis-plan.md).

## Current Behavior

The always-present area between Stop and the volume dial contains 24 continuous
bars, rendered as restrained gradient light directly on the chassis. Both palettes
share one frameless widget. The session provides measured targets; the UI owns
35 ms attack/180 ms release smoothing on a 16 ms timer. Pause freezes displayed
values; stop/EOF/failure clears them. Seek/track generations discard stale data.
Hidden/minimized views stop animation; reduced motion directly applies targets.
Closing stops the timer. Scan clears levels until normal playback resumes.

Core integrates bin power into logarithmic 40 Hz–16 kHz bands with fractional-bin
overlap and a fixed -70..-10 dB height range. Frequencies above Nyquist stay dark.
Live processing takes complete 2,048-frame windows at no more than 20 measurements/s,
independently of presentation. Measurements precede EQ, fades and software volume;
they are not output/DAC readings or audio-quality judgments.

The Linux adapter embeds an owned C11 LADSPA filter built by `cc`. Its bytes load
from a sealed anonymous `memfd`, never media paths or `LADSPA_PATH`. The library
and descriptor remain alive for the process lifetime. Validated channel count
selects a matching multichannel descriptor. An `asplit` branch converts to Float32
and terminates in `anullsink`; the main branch retains its format. No second
decoder or system audio capture runs for live analysis.

Each player has a unique context token; each graph has a non-reused instance ID.
Before seek, the adapter records an identity barrier and rejects old instances.
The callback interleaves channels into eight fixed 1,024-frame blocks, using
release/acquire indices. It never allocates, waits, logs or computes FFT. Full
queues or a contended producer drop analysis input. Source counters expose gaps;
the worker resets history instead of joining separated samples. It processes at
most eight blocks per tick and retains at most 128 reduced results. Contexts are
freed only after native destruction stops all callbacks.

Sample counters plus seek offset are matched to mpv's estimated audio clock, with
a timeline fallback when that clock is unavailable. Unsolicited reconstruction
reanchors the estimate. LADSPA carries no native timestamps: exact synchronization
across arbitrary formats/devices is not proven. Missing filter support is an
optional-analysis warning, logged once per track; fake activity is never shown.

## Key Files

- `crates/resolibre-core/src/spectrum/live.rs`, `live/consumer.rs`: bands, motion, cadence and stale-input policy.
- `crates/resolibre-audio/native/tap.c`, `build.rs`: callback transport/build.
- `crates/resolibre-audio/src/live_spectrum/`: loading, lifetime and native clock/input adaptation.
- `crates/resolibre-audio/src/session/{mod,worker}.rs`: observation and optional failure.
- `apps/desktop/src/live_spectrum.rs`, `ui/components/live-spectrum.slint`: display.

## Tests

Core tests cover frequency placement, Nyquist/silence, invalid values and refresh-
independent motion. Native tests cover sealed code, context exhaustion/retirement,
18 engine replacements, frequency-changing seek and pause. The PCM harness compares
baseline bytes against consuming/stalled bundled taps in three trials. Existing
EQ, TRIM/FADE, scan, queue and preservation tests remain connected.

The compiled view test checks pause, generation reset, hidden/reduced-motion timers
and clearing. The isolated `live_spectrum::tests::native_motion` journey uses real
generated PCM, checks changing frames, pause/seek/resume/stop and saves 1x/2x images.
Static previews cover minimum width and both palettes. A queued timer callback
after pause was caught by this journey and now checks confirmed playing state.
See [validation](../../testing/live-tap-validation.md) for executed checks and limits.
A timer interval or snapshots alone do not prove display frame pacing.

## Edge Cases

At most 16 contexts coexist; exhaustion disables optional analysis. Tokens/graph
IDs fail closed on exhaustion. Buffers support 32 channels and do not grow with
track duration. Large callbacks may drop input after eight blocks. A late format
or clock may temporarily hide data. There is no persistent analysis cache.

C transport and core algorithms are platform-neutral; the loader/build currently
targets Linux. The native Windows player explicitly reports Live Spectrum unavailable.
Windows/Android tap packaging, iOS signing, physical-device alignment
and full desktop CPU/GPU budgets still require validation. This is not a completed
Windows/mobile application port.

## Related Docs

[Shared STFT](spectral-analysis.md), [information](track-information.md),
[analysis target](../target/analysis-and-radio.md), [tests](../../testing/test-plan.md).
