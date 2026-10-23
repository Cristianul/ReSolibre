# Live PCM tap investigation

Status: Linux proof and integrated reference runtime, 2026-10-22. Read with
root/core rules, maps, security and tests (`live-tap-validation` profile).

## Decision and boundary

The user chose to retain libmpv and proceed with a filter, rather than replace the
playback engine. Shared analysis stays in core; platform adapters own PCM access
and packaging. This approves the direction, not a claim that all platforms work.

The public mpv client API has no decoded-PCM callback; its render API is video.
The experiment splits the audio graph with `asplit`: one unchanged output branch
and one Float32 LADSPA tap ending in `anullsink`. No system audio capture, second
decoder or patched mpv is involved. It tests the library supplied by the harness;
this historical harness filter is separate from the application's bundled runtime.

## Reproduce

Use the [native environment](../development/native-environment.md), a C11 compiler
and an explicitly provided [LADSPA SDK](https://www.ladspa.org/ladspa_sdk/ladspa.h.txt)
include directory. The SDK header is not vendored. The installed FFmpeg must
enable its [LADSPA filter](https://ffmpeg.org/ffmpeg-filters.html#ladspa).

```sh
python3 -m unittest discover -s tools -p 'test_live_tap.py' -v
python3 tools/check_live_tap.py --ladspa-include /path/to/sdk --trials 3
```

`tools/fixtures/live-tap/tap.c` exposes a test descriptor and bounded SPSC queues:
16 channel slots, four blocks per slot, at most 2,048 Float32 samples per block.
C11 lock-free index atomics are required at compile time. The callback copies
samples and drops analysis data when full, without waiting, allocating or I/O.
Only the harness reader consumes each queue. Release/acquire indices transfer
buffer ownership; library lifetime extends beyond engine teardown.

`tools/check_live_tap.py` compiles into a private temporary directory and starts a
fresh process for each run. Generated read-only WAVs are opened through validated
regular-file descriptors; scripts, references, network protocols and cache are
disabled. Output cannot overwrite an existing file and requires a private owned
directory. Native output is suppressed, child time/file size and fixture size
are bounded, and source bytes/modification metadata/mode are checked. Explicit
SDK/plugin arguments are trusted developer code, never media-supplied paths.

## Executed evidence

On the reference Linux x86_64/libmpv 0.41.0 environment, baseline, consuming tap
and stalled tap produced identical captured WAV bytes for mono 16-bit, stereo
16-bit and six-channel 24-bit generated input. Opposite-phase channels retained
their independent power. A stalled reader overflowed the queues while playback
still reached EOF. Four negative tests passed for read-only access, symlinks,
FIFO/directory/empty/oversized input and capture overwrite/directory isolation.

Three two-second timed-null trials per mode produced:

| Mode | Highest process RSS (MiB) | Mean process CPU (s) | Longest pause command call (ms) |
| --- | ---: | ---: | ---: |
| Baseline | 93.578 | 0.1169 | 0.149 |
| Consuming tap | 93.656 | 0.1280 | 0.137 |
| Stalled tap | 93.730 | 0.1182 | 0.141 |

These are short harness measurements including Python, not isolated filter cost,
physical audio latency, percentile acceptance or a desktop memory baseline.
Small differences are noisy. The existing 200 MiB desktop/100 MiB audio resource
gates remain unchanged and are not satisfied by this experiment alone.

## Historical proof limits

The proof deliberately does not recycle channel slots or support multiple graphs
in one process. Large callbacks truncate analysis blocks; they do not provide a
continuous STFT input. It has no source timestamps, seek generations, track
replacement, playback-clock alignment or EQ/scan integration. Production must
solve these before consumers use its samples. Filter absence must remain an
observable optional-analysis failure, never break otherwise playable music.

| Platform | Packaging investigation; not execution evidence |
| --- | --- |
| Linux | Shared library and FFmpeg LADSPA work in the isolated harness. |
| Windows | FFmpeg's wrapper uses `dlfcn.h`/`dlopen`; a compatible build and loader bridge, DLL exports and path handling need verification. |
| Android | Package native libraries per ABI and enable LADSPA in the owned FFmpeg build; upstream mpv-android's current build script does not request it. NDK/device lifecycle tests remain. |
| iOS | Investigate bundled signed library/framework loading and FFmpeg configuration; no executable downloads or writable plugin search path. Device tests remain. |

No ordinary Linux monitor becomes the portable contract. This repository still
has a Linux-only playback adapter; portable shared results are not a completed
Windows/mobile port. Platform builds and physical-device synchronization remain
open. The integrated graph lifecycle and bounded transport are described below.

## Integrated runtime

The [live spectrum](../architecture/features/live-spectrum.md) now connects the
owned multichannel filter to the shared FFT/bands and desktop motion. It loads
bundled bytes through sealed anonymous memory. Source counters, graph-instance
barriers and per-player contexts replace the historical proof's unrecycled slots
and silently truncated channel blocks. Queue overflow is still allowed, but gaps
reset analysis and never create an apparently continuous input.

Executed: 18 native open/seek/pause/close cycles across mono, stereo and six channels
distinguished 250 Hz from 1,500 Hz after seek, including opposite-phase channels.
The expanded PCM harness passed 15 captures, including byte-identical
baseline/consuming/stalled bundled-filter output, native negatives and preservation.
Native static and real-audio motion journeys passed at 1x/2x; the latter exercises
pause, paused seek, resume and stop. A queued callback after pause initially moved
bars; checking confirmed playing state fixed the regression. An untimed capture
sink's unavailable audio clock now correctly falls back to the engine timeline.

The first three-WAV/three-FLAC session resource run stayed below 84 MiB over 60
open/close cycles per resource trial, but exceeded proposed CPU limits (7–9% play,
12–13.4% controls, one core). Live measurement cadence was then bounded to 20/s
without reducing presentation cadence; RustFFT now selects supported SIMD with a
scalar fallback. The follow-up three-WAV/three-FLAC resource run measured 3.8–4.8%
playback CPU, 7.99–9.79% control CPU and at most 85.55 MiB across repeated cycles.
These samples fit the proposed CPU/memory thresholds, but three trials do not
replace the full 100-trial percentile protocol. Harness/monitor costs are separate.

Windows/mobile execution, physical audio/visual latency, full percentile runs
and desktop GPU/resource acceptance remain open. The architecture keeps the
loader/build platform-specific and shared facts/FFT/bands/motion in portable core.

## Related evidence

[Active plan](../plans/track-analysis-plan.md),
[shared STFT](../architecture/features/spectral-analysis.md),
[FFmpeg wrapper](https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/af_ladspa.c),
[mpv-android FFmpeg build](https://github.com/mpv-android/mpv-android/blob/master/buildscripts/scripts/ffmpeg.sh),
[Android native libraries](https://developer.android.com/ndk/guides/libs),
[Apple framework packaging](https://developer.apple.com/library/archive/technotes/tn2435/_index.html).
