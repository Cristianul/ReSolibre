# Local playback responsiveness validation

## Boundary and reproduction

Linux x86_64 GNU reference. [Native setup](../development/native-environment.md):

```sh
python3 -m unittest discover -s tools -p 'test_responsiveness*.py' -v
python3 tools/check_playback_responsiveness.py --check-reference-budgets
python3 tools/check_playback_responsiveness.py --trials 3 --allocator-diagnostic
```

Default: 100 fresh-player trials per codec over three private sessions (34/33/33),
60 same-process cycles per session, no retries; failure reports its attempt.
`--trials 3 --skip-resources` is only smoke. Diagnostics use one WAV session,
60 cycles and test-only trim. `--system-allocator` compares library defaults.

Approved [startup policy](../../crates/resolibre-audio/examples/support/process_memory.rs):
Linux/glibc CLI calls `mallopt` before threads to fix the mmap threshold at
128 KiB; failure exits safely. Other libc implementations skip it. `LocalPlayer`,
audio buffers, ownership and system settings are unchanged. No production trim,
arena limit or allocator dependency. The [single-threaded regression](../../crates/resolibre-audio/tests/process_memory.rs)
checks repeated allocation/free behavior; omitting the policy reproduced failure.

The harness applies equivalent glibc settings only at worker exec: `mallopt` is
MT-Unsafe and libtest starts threads. Both threshold spellings are set, replacing
worker `GLIBC_TUNABLES` to prevent overrides. Server/monitor/harness retain their
environment. Raw library callers retain system allocation policy.

## Signal and timing protocol

[Worker](../../crates/resolibre-audio/src/responsiveness_tests.rs),
[harness](../../tools/check_playback_responsiveness.py),
[probe](../../tools/responsiveness_probe.py),
[analyzer](../../tools/responsiveness_signal.py).

Generated 112 s, 48 kHz stereo PCM16 encodes absolute frame identity in two
channels; generated FLAC preserves it. Decoder position is not evidence.
Every 480-frame block must decode to consecutive source frames within one PCM
unit of quantization; silence is at most two units. Two adjacent blocks confirm
a result. First target-block offset and resume continuation error must be at
most 960 frames (20 ms). A full-block silence cannot be a valid marker.

CLOCK_MONOTONIC correlates submission/native entry/return and pipe receipt.
Response times are **monitor receipt upper bounds**, including buffering and
20 ms confirmation. Calibration checks continuity/delivery gaps/span error,
not absolute lag; nothing is subtracted. Startup identifies source within its
first second, rejecting stale tails. Play includes private stream linking
(recorded separately). First/subsequent streams are separated; files are warm.
No cold-cache or physical-latency claim.

Each trial checks pause/resume, seeks to 103/4/0/111 s, paused seek to 60 s,
then eight seeks (8/90/2/103/20/4/100/70 s) submitted every 5 ms without waiting
for native return. Record actual lateness and every acknowledgement; no test
coalescing occurs. Confirm the final target and 120 ms of subsequent continuity.
Check EOF/commands after EOF and loss while seek output is pending separately.

## Resources and isolation

Sample player/server/Pulse/monitor/harness CPU and RSS every 100 ms during
5 s playback and 5 s controls (seek every 200 ms); one core = 100%. Each of
60 lifecycles samples RSS after open/seek and Drop, plus glibc counters.
Samples do not establish an instantaneous peak or an indefinite memory bound.

Use the [private-server safeguards](output-failure-validation.md): owned 0700
directories, sockets, process groups and verified graph; hostile configs stay
isolated. Generated sources are read-only, <=32 MiB, with incremental hash and
mtime/mode checks. FFmpeg streams bounded FLAC. Analysis retains 120 blocks and
one partial block, chunks <=64 KiB; logs <=2 MiB, no growing capture files.
Datagrams <=64 bytes, batches <=64 commands; invalid/replayed commands fail.
Readiness/trial/resource deadlines: 5/15/90 s; owned children are always reaped.
Negatives cover malformed input/commands, absent/lost output, dead monitor,
stale/corrupt/truncated markers, bounds, backpressure and timeout cleanup.

## Results and budget review

The [archived investigation](../plans/implemented/playback-responsiveness-plan.md#baseline-investigation)
records harness defects, interrupted attempts, allocator comparisons and why
application-startup ownership was selected. Current acceptance is below.

Reference run: 2026-10-21, Fedora 45 prerelease, Linux 7.3.0-rc7, Ryzen AI MAX+
392 (24 logical CPUs), about 62 GiB RAM, glibc 2.44, Rust 1.99 debug build,
libmpv 0.41.0, FFmpeg 9.0.2 and PipeWire 1.6.9. Local generated files, default
virtual sink volume, no EQ or scan; normal workstation background load.

The corrected full run passed 200/200 trials, six server sessions, protocol
negatives, missing/lost output, malformed input and dead-monitor rejection.
Response upper bounds below are milliseconds; percentiles are nearest-rank.

The complete historical percentile table is retained in the
[execution plan](../plans/implemented/playback-responsiveness-plan.md#baseline-percentiles).

Untuned baseline: target error <=9.98 ms, resume error <=14 ms; native control
p95 <1 ms, dispatch p95 61.83–63.29 ms (includes the 100 ms event wait).
Fresh RSS <=72.18 MiB; ten-cycle RSS maxima 131.67–162.98 MiB. CPU ranges:
1.4–4.0% playback, 2.0–4.2% controls. Whole-response timing is not native-call time.

## Application-policy acceptance

Final worker-only run: 200/200 fresh journeys and 360 lifecycles across six
sessions passed, including all native negatives and the budget gate. No trim.
Milliseconds at monitor receipt; same environment and nearest-rank summaries.

| Codec / operation | n | p50 | p95 | p99 | Max |
| --- | ---: | ---: | ---: | ---: | ---: |
| WAV play | 100 | 110.93 | 143.55 | 148.02 | 158.79 |
| WAV pause | 200 | 74.37 | 106.06 | 107.75 | 127.23 |
| WAV resume | 100 | 55.46 | 84.01 | 86.15 | 86.19 |
| WAV seek | 400 | 73.88 | 115.90 | 127.22 | 127.88 |
| WAV paused seek/resume | 100 | 101.48 | 112.14 | 112.94 | 113.08 |
| WAV burst final | 100 | 60.18 | 81.41 | 92.15 | 92.18 |
| FLAC play | 100 | 84.60 | 97.55 | 101.59 | 102.18 |
| FLAC pause | 200 | 74.85 | 107.21 | 127.24 | 127.58 |
| FLAC resume | 100 | 54.20 | 86.14 | 86.32 | 86.35 |
| FLAC seek | 400 | 74.16 | 106.32 | 127.28 | 128.18 |
| FLAC paused seek/resume | 100 | 101.54 | 112.18 | 112.77 | 115.71 |
| FLAC burst final | 100 | 60.41 | 81.91 | 92.30 | 92.79 |

RSS maxima: fresh 71.68 MiB, active lifecycle 73.92 MiB, post-Drop 73.32 MiB;
all below 100 MiB. Final post-Drop readings ranged 70.48–72.54 MiB across
sessions. CPU averaged 1.0–2.4% playback and 2.0–4.6% controls (one core).
Target/resume errors remained <=9.98/14 ms. Native-control p95 was 0.52–0.81 ms;
dispatch p95 63.20–64.81 ms. These runs meet comparison limits; they do not
establish statistical latency/CPU improvements or bounds for other workloads.

Approved memory objective: <=100 MiB fresh, active and after repeated cycles;
the old proposed 256 MiB ceiling is withdrawn. Comparison guardrails: startup
p95 <=200 ms, other controls <=150 ms; 5 s CPU averages <=5% playback and <=10%
controls. These are reference-workload checks, not product-wide promises.
`--check-reference-budgets` rejects incomplete, non-finite or over-limit evidence.

Regressions: 13 Rust tests plus the standalone allocator test (also with an
inherited 4 MiB threshold), core-only without native exports, 22 Python tests,
CLI privacy/FIFO/network checks, rustfmt and Clippy passed. Nine PCM captures/two negative cases and three output-failure journeys plus
malformed input passed under the fixed threshold. Physical-device/listening and
other media/platforms remain unmeasured in this slice.
