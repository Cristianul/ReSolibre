# Local playback responsiveness plan

Status: implemented and verified on 2026-10-21. Memory reduction before SMB
and application-startup policy were approved; reference acceptance passed.

## Scope and decisions

Measure actual play/pause/resume/seek output, CPU and memory on the private
PipeWire/Pulse monitor. Physical latency, SMB, EQ, gapless, UI and automatic
recovery remain outside scope. No production worker/coalescing API is introduced.

User approved <=100 MiB as the initial repeated-cycle memory objective, replacing
the unaccepted 256 MiB proposal. User then approved application-startup glibc
policy: fix the mmap threshold at 128 KiB, never inside `LocalPlayer::open`.
Preserve engine/file ownership and audio buffer sizes. Check latency/CPU against
the recorded comparison guardrails; these are not product-wide guarantees.

## Execution and evidence

1. **Probe implemented.** Ignored typed Rust worker, bounded private datagrams,
   shared monotonic clock and streaming position-marked WAV/FLAC analyzer.
   Receipt times are upper bounds; calibration jitter is never subtracted.
2. **Baseline complete.** 100 fresh-player journeys per codec over three sessions:
   pause/resume, 103/4/0/111-second seeks, paused seek, independent 5 ms bursts,
   settled final output, EOF and output loss. Preserve every failure, no retries.
3. **Retention traced.** Sixty same-process open/control/Drop cycles reached
   ~182 MiB with mostly free glibc heap. Trim barely helped; a fixed threshold
   reduced diagnostics to ~68 MiB. Limiting arenas to two reached ~88 MiB.
4. **Policy implemented.** Single-threaded CLI startup owns the setting; a
   standalone regression checks actual allocation/free behavior. Worker tests
   apply the equivalent setting at exec, before libtest creates its thread.
5. **Final acceptance passed.** 200/200 journeys, six sessions with 60 cycles
   each and all native negatives. Maximum active/post-Drop RSS: 73.92/73.32 MiB;
   fresh: 71.68 MiB. CPU: 1.0–2.4% playback, 2.0–4.6% controls. The <=100 MiB
   objective and latency/CPU comparison gate passed without retries or trim.

The [validation record](../../testing/playback-responsiveness-validation.md) defines
marker thresholds, uncertainty, attempts, metrics, isolation and exact commands.
It also records the stale-startup harness failure and buffered-tail regression;
neither established a production playback defect.

## Safety and closure

Generated media only; preserve hash/mtime/mode. Private 0700 directories, owned
sockets/process groups, explicit endpoint/PID checks and hostile-config isolation.
Sources <=32 MiB; logs <=2 MiB; bounded streaming capture, 64-command batches;
readiness/control/resource deadlines 5/15/90 s. Reject missing/corrupt/stale audio,
malformed/replayed commands, absent/lost output and dead monitors. Reap failures.

Rust/core-only tests, 22 Python checks, CLI privacy/FIFO/network checks,
PCM/output-failure suites, rustfmt, Clippy and docs/whitespace checks passed.
Physical-device checks remain separate. Archived after final acceptance; new
files tracked without committing. Reading profile:
`playback-responsiveness-history`; focused validation/native profiles supply details.

## Baseline investigation

The first full attempt caught a harness defect: startup accepted a previous
player's tail, then failed pause continuity. A first-second identity requirement,
regression and 18 smoke journeys passed before restarting. No production control
defect was established.

Review also found that continuity checks could skip a buffered tail already read
with confirmation. A regression now rejects stale/audio-while-paused tails. An
application run was interrupted for this fix; six smoke journeys passed before
restarting acceptance. Measurement timestamps/thresholds were unchanged.

The untuned 60-cycle WAV diagnostic settled at 182.18 MiB (flat after about 30
cycles). glibc allocated bytes grew 0.56→0.60 MiB, free bytes 15.43→114.69 MiB:
retention explains most growth. Test-only trim left 180.71 MiB. This is not a
whole-process leak proof; see the
[glibc definitions](https://sourceware.org/glibc/manual/latest/html_node/Statistics-of-Malloc.html).

Isolated 60-cycle WAV comparisons (same debug build):

| Allocator policy | Fresh RSS MiB | Maximum post-Drop RSS MiB |
| --- | ---: | ---: |
| Default, dynamic threshold | 71.31 | 182.64 |
| `MALLOC_ARENA_MAX=2` | 67.38 | 87.89 |
| `MALLOC_MMAP_THRESHOLD_=131072` | 66.43 | 67.72 |

The two tuned diagnostics ran concurrently; timings are not acceptance evidence.
Settings were inherited by all child processes. Static threshold disables
[glibc's dynamic adjustment](https://sourceware.org/glibc/manual/latest/html_node/Malloc-Tunable-Parameters.html).
Application-startup ownership was approved; worker-only acceptance is in the validation record.

## Baseline percentiles

Untuned 2026-10-21 reference, milliseconds at monitor receipt; full protocol and
limits in the [validation record](../../testing/playback-responsiveness-validation.md).

| Codec / operation | n | p50 | p95 | p99 | Max |
| --- | ---: | ---: | ---: | ---: | ---: |
| WAV play | 100 | 111.11 | 143.09 | 150.97 | 161.76 |
| WAV pause | 200 | 74.63 | 106.54 | 117.14 | 126.96 |
| WAV resume | 100 | 54.22 | 75.45 | 85.80 | 86.30 |
| WAV seek | 400 | 73.77 | 106.47 | 126.81 | 128.86 |
| WAV paused seek/resume | 100 | 101.57 | 112.13 | 113.01 | 113.09 |
| WAV burst final | 100 | 60.26 | 81.43 | 81.82 | 92.68 |
| FLAC play | 100 | 109.96 | 123.27 | 124.71 | 129.16 |
| FLAC pause | 200 | 74.22 | 106.31 | 117.57 | 127.12 |
| FLAC resume | 100 | 54.62 | 85.76 | 86.27 | 86.72 |
| FLAC seek | 400 | 73.53 | 106.49 | 126.53 | 128.10 |
| FLAC paused seek/resume | 100 | 100.27 | 112.79 | 117.53 | 117.81 |
| FLAC burst final | 100 | 69.58 | 82.23 | 92.15 | 92.18 |
