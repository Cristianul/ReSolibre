# Audio and equalizer target

## Playback policy

Preserve supported rate/layout/precision; otherwise convert compatibly, never
reject playable music for purity. Report processing: EQ, gain, normalization,
mixing and OS/device processing alter samples even at matching rates.
Exclusive output remains planned.

App volume works; [processing](audio-processing.md) defines normalization,
album-independent crossfade, beat sync and signal-path work.

[Queue modes](../features/queue-playback-order.md) share core policy.

## Engine and source boundary

Libmpv plays local audio/video sound. Disable video, scripts, user configuration,
URL helpers and implicit network resolution.

Prove local seek, gapless, EQ, output and resources.
SMB random access/cancellation follows local feature acceptance; pipes cannot seek.
Prove crossfade before fixing session design. Failed requirements trigger a
decision on FFmpeg/direct output.

## Control and buffering

[Hold Next/Previous](../features/transport-scan.md): 3× pitch-preserving scan,
release restores forward 1× and prior pause; short clicks navigate.

- Separate UI feedback, engine acknowledgement and actual play/pause/seek output.
- Seek precisely where supported: `01:43` means 103 seconds within agreed tolerance.
  Disclose unsupported/non-seekable cases.
- Coalesce seeks, cancel old reads and invalidate stale decoded buffers; last
  accepted request wins.
- Bound read-ahead, queues/disk cache independently of scans; never download a
  whole library for latency. Measure source wait, codec indexing and hardware
  buffering separately. Budget prefetch; gapless needs compatible formats and
  cannot guarantee arbitrary device changes or network failures.

Approved Linux/glibc policy: application startup fixes the allocator mmap
threshold at 128 KiB; libraries never apply it implicitly. Reference objective:
<=100 MiB during/after 60 open/control/Drop cycles. Recheck CPU and controls;
this is no hard cap or guarantee for other media/platforms.

## Equalizer contract

Each client ships with EQ:

| Control | Behavior |
| --- | --- |
| Ten bands | Validated shared frequency/gain definitions |
| Preamp | Manual -2..+2 dB; zero centered |
| Bypass / reset | Preserve preset when bypassed; reset to a defined flat state |
| Presets | Named portable files; saved-first selector, confirmed replacement |
| Headroom | Explicit policy to prevent avoidable overload |

The [local EQ contract](../features/local-equalizer.md) implements the first slice:
user-selected manual preamp only (2026-10-22), no automatic compensation/limiter.
Versioned core values are independent of UI/filter syntax. Combined boost/peak,
low-rate, live-change and resource checks remain required. DSP is not bit-perfect;
other clients need measured response tolerances, not merely matching labels.

## Lifecycle and evidence

Handle unplug/replug, default-device changes, unsupported formats, short reads,
network interruption and EOF. Renegotiate compatible output or report a
recoverable error when no device is usable. Seeking/probing never writes media.

The [test plan](../../testing/test-plan.md) covers latency/precision, gapless,
EQ response/headroom and source preservation. Upstream references:
[mpv manual](https://mpv.io/manual/stable/),
[libmpv API](https://github.com/mpv-player/mpv/blob/master/include/mpv/client.h),
[Web Audio](https://www.w3.org/TR/webaudio/).
