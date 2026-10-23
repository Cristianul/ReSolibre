# Private audio-output failure validation

## Boundary and reproduction

This is Linux audio-server failure evidence, not physical unplugging or DAC
validation. Use the [native setup](../development/native-environment.md), then:

```sh
python3 -m unittest discover -s tools -p 'test_output_*.py' -v
python3 tools/check_output_failure.py
```

Requires PipeWire, pipewire-pulse, pactl, parec and pw-link. The 2026-10-20 run
used PipeWire 1.6.9, Pulse clients 17.0 and extracted libmpv 0.41.0 (ABI 2.5).

## Test routing and safeguards

[Configurations](../../tools/fixtures/audio-output/server.conf) create a private
0700 runtime, native server, Pulse bridge and virtual stereo sink. No hardware
discovery, session manager, DBus or TCP listener is configured. Explicit private
addresses and configuration replace inherited Pulse/PipeWire overrides.
The graph has exactly one sink; control queries verify its identity and the
worker PID/target attachment. Explicit links connect playback and sink monitor.

The ignored [Rust worker](../../crates/resolibre-audio/src/output_failure_tests.rs)
uses `ao=pulse`, a private `pulse-host` and `audio-device=pulse/resolibre-test`.
Test-only options; production selection is unchanged.
The fixture disables Pulse autospawn and shared memory: its default memory pool
exceeds the 2 MiB file limit. Audio travels over the owned local socket.

The [harness](../../tools/check_output_failure.py) generates six-second read-only
48 kHz stereo PCM16 tones. A monitor block with at least 128 samples above 100
in S16 amplitude proves non-silent virtual activity before loss. It does not
measure fidelity, acoustic output or latency. Formats/position are not failure evidence.

[Process ownership](../../tools/private_audio.py) bounds readiness to 5 s,
worker lifetime including open/Drop to 12 s, and each log/capture to 2 MiB.
Cleanup terminates owned process groups, escalates and reaps direct children
before temporary files/sockets are removed. Timeouts fail the harness; they
never substitute for adapter errors. Diagnostics expose only allowlisted worker
reports. Source hashes, mtime and permissions must match after every case;
hostile external mpv configuration/scripts must stay inactive.

## Recorded evidence

Three complete journeys passed:

| Case | Observed result |
| --- | --- |
| Baseline | Monitor audio, EOF, empty formats, completed Drop |
| Absent endpoint | Error -14 in 195–205 ms; no EOF |
| Lost Pulse connection | Monitor audio, then error -14 in 8–11 ms; no EOF |
| Fresh recovery | Old worker destroyed/reaped before restart; new monitor audio and EOF |

Failures cleared formats and completed destruction. Loss was injected
within one second of monitor confirmation and within the first three seconds
of the six-second fixture. Failure response must be within 3 s; measurements
include parent observation/control overhead and are regression evidence only.

Malformed media failed safely. Four helper tests passed: routing isolation,
failed startup, file bounds and timeout cleanup of a TERM-resistant descendant. Twelve ordinary Rust tests,
core-only checks without native exports, nine PCM captures/two PCM failures,
seven PCM helper tests, CLI/device silence checks, rustfmt, Clippy and docs
validation passed. No production fix or new recovery policy was needed.

Physical devices, automatic reconnect/switching, timing and resource benchmarks
remain unvalidated. The [completed plan](../plans/implemented/output-failure-plan.md)
records scope and completion; [local playback](../architecture/features/local-playback.md)
describes the current API.
