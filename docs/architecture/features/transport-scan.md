# Hold-to-scan transport

## Read When

Changing Next/Previous gestures, temporary speed/direction, or scan cancellation.
Read both area rules/maps and the [audio target](../target/audio-and-equalizer.md).

## Do Not Read For

Queue traversal uses [playback order](queue-playback-order.md); live FFT/full-file
analysis remains in the [analysis plan](../../plans/track-analysis-plan.md).

## Current Behavior

Short clicks use [shared Back/Next policy](queue-playback-order.md). A 350 ms hold
scans the pressed track at 3× forward/backward with native pitch correction.
Backward audio is reversed, with pitch retained. Keyboard/accessibility activation
uses the same policy. Seeking must be available; a lone/first/last track can still
be scanned even when its corresponding navigation action is unavailable.

Release (including outside the control), focus loss or disabling ends the hold.
The session restores forward 1× playback and the original pause intent. Scanning
from pause temporarily plays audio. Each edge holds on the current track while
pressed; release resumes normal EOF handling, or stays paused if initially paused.
The status strip reports scan direction and speed. Scan state is never persisted.

The shared button owns the gesture; desktop captures the generation on pointer-down
and defers release outside snapshot refresh borrows. The worker validates that
generation and owns pause/scan state. Track replacement, removal, Stop, failure and
shutdown discard scanning; seeks and Play/Pause restore normal transport first.

The adapter explicitly enables pitch correction, uses native `speed=3` and
`play-direction`, and seeks to the observed position after changes to flush audio
queued at the old speed. Live EQ intent is committed before graph reconstruction.
A temporary `keep-open` holds boundaries; ordinary resume restores normal EOF.
Reversal buffering is capped at 4 MiB, backward demux steps at one second. Existing
8 MiB demux bound, disabled disk cache/config/scripts/network references remain.

## Key Files

- `crates/resolibre-core/src/session.rs`: portable `ScanDirection`.
- `crates/resolibre-audio/src/controls.rs`, `equalizer.rs`: native scan/EQ reconstruction.
- `crates/resolibre-audio/src/session/`: typed commands, worker and bounded release.
- `apps/desktop/src/transport_scan.rs`: generation-scoped gesture bindings.
- `apps/desktop/ui/components/controls.slint`, `ui/player.slint`: shared hold input.

## Tests

`cargo test --workspace --all-targets --locked`: native session speed/restoration,
both edges, paused scans, stale generations; mailbox flood/release; compiled UI
short/long press, outside release, focus loss, disabling and actual binding flow.
`python3 tools/check_transport_scan.py`: six isolated captures through the production
adapter, generated WAV/FLAC/MP3 in both directions. A 12-second two-tone signal takes
about four seconds; 440/880 Hz remain unchanged and reverse order is verified.
The untimed PCM sink cannot retract pre-scan output on reset; the checker removes
only whole 65,536-frame priming buffers before measuring the scan (see
[upstream sink](https://github.com/mpv-player/mpv/blob/v0.41.0/audio/out/ao_pcm.c)).
The parent bounds child lifetime/output and verifies read-only source hashes/mode/
mtime, hostile external-config isolation and private diagnostics. Requires FFmpeg.

Executed: 88 workspace tests, 14 isolated native UI cases, six scan captures and
nine conversion captures plus negative cases. Clippy, build, rustfmt and docs pass.
`tools/check_window_snap.py` covers connected UI behavior.
Captures prove digital behavior, not physical device latency or listening quality.

## Edge Cases

32 ordinary pending commands plus one reserved scan-release slot. A current-generation
release removes pending scans for that/older generations despite control flooding; stale
releases receive no priority and cannot cancel a newer gesture. Stop clears all.
All rates/directions are typed constants, not user-supplied native filter expressions.
No dependencies, media writes or external access are introduced.

Reverse decoding depends on container/codec seekability. WAV/FLAC/MP3 have measured
coverage; other formats and physical devices need further validation. Existing native
call cancellation limits apply; pending format negotiation retains its ten-second
worker timeout. Pitch preservation is time stretching, not bit-perfect playback.

## Related Docs

[Player](local-desktop-player.md), [EQ](local-equalizer.md),
[security](../target/security-and-privacy.md), [test plan](../../testing/test-plan.md).
