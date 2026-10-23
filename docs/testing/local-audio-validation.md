# Local audio validation

## Scope and reproduction

Read the [local playback feature](../architecture/features/local-playback.md)
and [native environment guide](../development/native-environment.md) first.
This is the manual device/listening flow, not a new output policy.

After exporting the documented native library paths, run from the repository:

```sh
cargo build -p resolibre-audio --example local-play --offline
python3 tools/check_local_playback.py --device
python3 tools/check_audible_playback.py
```

The listening script creates disposable, read-only WAV PCM16 fixtures outside
music directories: three seconds each, mono at 44,100 Hz and stereo at 48,000 Hz.
Each has 440 Hz then 880 Hz bursts, separated by silence, with 20 ms fades and
8% peak amplitude (about -22 dBFS). Stereo sends the low tone only left and the
high tone only right. System volume is unchanged. Each process has a ten-second
deadline; temporary files are removed when the check exits.

Record heard/missing/distorted sound and channel placement independently of
script success. The script checks decoder/output reporting, EOF, SHA-256,
modification time and read-only permissions; it cannot judge listening.

## Reference evidence — 2026-10-20

Environment: the recorded Fedora x86_64 setup, extracted libmpv 0.41.0, default
desktop output and the user's speakers. `wpctl` reported volume 0.44 without a
mute marker before the run. No system audio settings were changed.

| Generated source | Decoder report | Audio API output report |
| --- | --- | --- |
| WAV PCM16, 44,100 Hz, mono | 44,100 Hz, 1 channel, Signed16 | 44,100 Hz, 1 channel, Signed16 |
| WAV PCM16, 48,000 Hz, stereo | 48,000 Hz, 2 channels, Signed16 | 48,000 Hz, 2 channels, Signed16 |

Both complete automated journeys passed. The user confirmed both mono tones
from both speakers, followed by the low stereo tone on the left and the high
tone on the right. This establishes audible local playback and the expected
channel routing on this setup. It is not a fidelity or distortion measurement.

The decoder property became unavailable while audio output was draining before
EOF. CLI checks originally mistook this for missing evidence; they now require
a valid report during playback and allow later unavailability. Current values
and previously observed evidence remain distinct.

Ten Rust tests, core-only tests without native exports, CLI isolation/device
checks, rustfmt, Clippy and documentation validation are the verification set
for this change. See the feature for coverage and the native guide for commands.

## Measurement boundaries and remaining work

`position()` reads engine `time-pos`; the seek test's 20 ms tolerance applies
only to that property. Neither position, loaded/EOF events nor format reports
measure when samples reach speakers. No play/pause/seek latency was measured.

The source report describes decoded PCM, not the original codec or effective
bit depth. `Signed16` identifies signed 16-bit sample storage. Output describes
the audio API boundary; OS mixing, conversion and physical DAC format were not
measured. Matching rates/representations prove neither bit-perfect playback nor
absence of downstream conversion. A later [simulated-output check](output-adaptation-validation.md)
validates channel/sample adaptation separately. [Captured rate conversion](pcm-conversion-validation.md)
is also checked. Device fallback/loss and wider codec/device coverage remain pending.
