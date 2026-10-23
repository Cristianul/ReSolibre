# Initial local desktop validation

Historical baseline, 2026-10-22, before later EQ and scrub fixes. Measurements below
are not a claim that every gate was rerun after those changes. Current evidence:
[local desktop validation](local-desktop-validation.md). Reading profile:
`local-desktop-baseline`.
Reference: Linux x86_64 workstation, Rust 1.99, Slint 1.18.1 Winit/FemtoVG,
libmpv 0.41, debug builds, glibc mmap threshold 65536 before threads.
[Setup/commands](../development/desktop-environment.md).

## Checks

- Workspace: 30 passed, seven ignored harness/native tests exercised separately.
  Core-only check, rustfmt, all-target Clippy with warnings denied: passed.
  Python: 23 passed.
- Session tests: EOF/queue, stale seeks, paused navigation, failure/recovery,
  bounds, repeated cleanup and durationless FLAC. UI tests: accessible actions,
  pointer/keyboard controls, 2x scale, long Unicode text and exclusive panels.
  Native compact/Playlist/EQ screenshots visually inspected.
- Supplied FLAC/MP3 journeys passed on silent and default-device outputs.
  SHA-256/size/mtime/mode and OS sink volume/mute unchanged. No listening claim.
- CLI privacy/FIFO/network-reference/config isolation and default-device silence
  passed. Nine PCM captures plus malformed/destination negatives passed. Three
  private output-loss/recovery journeys and malformed input passed.
- Captured volume 0/50/100%: amplitude 0/0.25/1; generated-tone RMS
  0.000000 / 0.014142 / 0.056567.
- Cargo audit 0.22.2: zero known vulnerabilities; maintenance warnings remain for
  bincode (RUSTSEC-2025-0141), ttf-parser (2026-0192), zbus_names (2026-0371),
  zvariant (2026-0370), zvariant_derive (2026-0372). Recheck before packaging.

Queue follow-up: shared policy/actions/row. Native/UI tests pass duplicate/stale
IDs, paused edits, remove-current stop and empty-queue recovery. Six marker
journeys plus protocol/output-loss negatives passed. Twelve UI cycles: 184.20 MiB
peak, callback-group p95 7.831 ms, max 9.157 ms. Row screenshots checked.

Follow-ups: [queue modes](../plans/implemented/shuffle-repeat-plan.md),
[EQ](../plans/implemented/equalizer-plan.md).

## Budgets and measurements

User approved 200 MiB desktop RSS; native audio remains 100 MiB. Native UI:
12 pause/seek/next/resume/stop/replay cycles, 1,080 panel toggles, 183.91 MiB peak
RSS, 5.90 s elapsed. Three-panel-callback groups: p95 5.705 ms, max 7.640 ms;
p95 gate <=50 ms. Earlier single cycle: 173.54 MiB. Sampled process RSS every
20 ms; excludes portal/GPU services. No compositor-latency or catalog-scale claim.

Production-session marker gate: 100 WAV and 100 FLAC journeys across three
sessions per codec, with 60-cycle resource checks in each; zero failed trials.
Protocol/output-loss, absent output, malformed input and dead-monitor negatives
passed. Original comparison gates retained.

| Measurement | WAV | FLAC |
| --- | ---: | ---: |
| Play to monitor p95 | 109.239 ms | 93.083 ms |
| Seek to marker p95 | 53.504 ms | 53.417 ms |
| Largest settled RSS | 71.836 MiB | 72.207 MiB |
| Playback CPU, one core | 1.2–1.6% | 1.4–1.6% |
| Controls CPU, one core | 1.6–2.2% | 2.2% |

Monitor receipt is an upper bound, not physical latency. Session-mode legacy
`native_ms`/`open_native_ms` fields measure mailbox submission, not native work;
markers establish output. Stop acknowledgement waits for release. Local cleanup
passed five-second deadlines. Raw logs/samples are ignored under `.local`.

## Failures corrected

- Volume capture found double perceptual curves. The adapter inverts mpv's cubic
  gain before applying core amplitude; captures verify the correction.
  [mpv source](https://github.com/mpv-player/mpv/blob/v0.41.0/player/audio.c).
- FLAC estimated duration clamped a 103-second seek near the start, timing out the
  third smoke run. Capability is now separate from duration; typed absolute seeks
  are not clamped. Corrected smoke/full gates passed. The regression fixture is
  generated silence (130 s, mono 8 kHz), ffmpeg stdout FLAC without final total
  samples; no music. Native property assertions allow 0.5 s buffered delay;
  markers verify precise targets.
- UI introspection needed compiler debug metadata, now enabled in debug builds.
  Probe EOF semantics/error logging initially differed from production. It now
  permits exactly one allowlisted diagnostic per failure, with privacy negatives.

## Gaps

Native/kernel stalls cannot be forcibly cancelled; close visibly waits without
detaching. Resolve universal cleanup bounds before broader sources. User listening,
portal cancel/close, full native keyboard/focus/DPI/accessibility, startup/idle
sampling and distribution notices remain acceptance/release work. Artwork,
library/persistence are later slices. The plan stays active for manual acceptance
and native-stall limits; SMB follows everyday local features.
