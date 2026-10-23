# PCM conversion validation

## Scope and reproduction

Follow the core map/rules, local feature, audio target, test plan and
[status](../plans/implemented/pcm-conversion-plan.md). Reference: 2026-10-20,
Linux x86_64, extracted libmpv 0.41.0; dependencies resolve.

```sh
export RUSTFLAGS="-L native=$PWD/.local/native/usr/lib64"
export LD_LIBRARY_PATH="$PWD/.local/native/usr/lib64"
python3 -m unittest discover -s tools -p 'test_pcm_*.py' -v
python3 tools/check_pcm_conversion.py
```

Cargo JSON identifies the offline-built Rust test worker. Workspace tests skip
`pcm_capture_tests::capture_fixture`; this harness drives it explicitly.
`#[cfg(test)]` excludes capture options from production API/CLI.

The worker selects `ao=pcm`, explicit `capture.wav`, no append, and starts paused
to observe source/output formats before fast EOF. Conversion/control cases set
`audio-samplerate=48000`; isolation sets no rate override. The
[versioned option](https://github.com/mpv-player/mpv/blob/v0.41.0/DOCS/man/options.rst)
requests conversion, not automatic unsupported-device fallback. The
[PCM sink](https://github.com/mpv-player/mpv/blob/v0.41.0/audio/out/ao_pcm.c)
is untimed and finalizes WAV on destruction. Parse after EOF, Drop and exit;
rendering speed does not measure physical latency.

## Results

Stereo PCM16: 3 s, 440 Hz left at 0.5–1.3 s, 880 Hz right at 1.7–2.5 s, otherwise
silence, 20 ms fades, 8% peak. All cases passed three consecutive runs; format
reports match the finalized extensible PCM WAV.

| Case | Source/output Hz | Frames | Duration |
| --- | --- | --- | --- |
| Control | 48,000 / 48,000 | 144,000 | 3 s |
| Conversion | 44,100 / 48,000 | 144,000 | 3 s |
| Fresh isolation | 44,100 / 44,100 | 132,300 | 3 s |

Conversion measurements: 439.99994/879.99994 Hz, normalized RMS
0.05656822/0.05656857, burst timing errors 0.146/0.104 ms, silent-window RMS zero.
No clipping or tested local dropouts. These are captured-sample measurements.

Unchanged thresholds: duration/burst timing within 2 ms; pitch within 2 Hz;
steady RMS within 5% of 0.08/sqrt(2); silence/inactive-channel RMS below 0.0001;
no full-scale samples. Ignore 50 ms around transitions for RMS/pitch. Rising
zero crossings estimate pitch without a second resampler. Timing uses first/last
samples above 0.0002 full scale; 20 ms steady windows check local level/dropouts.

## Failure checks and limits

Seven Python tests cover valid tones, silence/swaps/wrong pitch/rate, truncation,
clipping/dropouts, malformed/oversized WAV and extensible metadata, real process
timeout/reaping and kernel write limits. Native malformed-media and uncreatable-
destination cases fail without format/finalization success or private diagnostics.

The parent owns 0700 directories, read-only generated inputs and fresh fixed
destinations. Existing files/symlinks are rejected; the negative case permits a
directory. Hash/mtime/permissions, hostile config/script isolation and cleanup
pass. Music/system settings are untouched; no registry dependencies were added.

Each native child has a ten-second deadline including open/Drop and a 2 MiB
`RLIMIT_FSIZE` per file, including captured logs; core dumps are disabled. Reads
are bounded before PCM allocation. Cargo build has a separate 120-second limit.
These test bounds do not establish production memory/latency guarantees.

Regressions pass: twelve Rust tests, core without native exports, Clippy, rustfmt,
CLI privacy/network/FIFO checks and device silence. Listening was not repeated.
These signal checks establish no fidelity, physical continuity, DAC negotiation,
device-loss recovery or bit-perfect claim.
