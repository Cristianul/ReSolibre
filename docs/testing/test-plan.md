# Test and performance plan

## Evidence and maintenance

[Prototype](../architecture/features/local-playback.md): current evidence/limits.
Acceptance follows [root rules](../../AGENTS.md): regressions, security
and source preservation. Test boundaries; report checks and gaps.

Docs checks (Python 3.9+):

```sh
python3 tools/check_docs.py
git diff --check
git diff --cached --check
```

Check UTF-8, links, placeholders, summary and profiles (32 KiB including rules/maps).
[Expanded acceptance](expanded-features-test-plan.md): new processing, playlists,
analysis, writes and connected work; not unchanged prototype checks.

## Acceptance matrix

| Area | Required evidence |
| --- | --- |
| Audio | Play/pause, exact seek, queue/shuffle/repeat, output fallback, compatible gapless; malformed media, device loss, cancellation/stale buffers |
| EQ | Response/preamp/bypass/presets/live changes; combined boosts, clipping, invalid/NaN gains, low-rate bands, clicks/CPU |
| Sources | Local/authenticated SMB read/seek/reconnect, remembering; denial, bad credentials, timeouts, short reads/source replacement |
| Catalog | Scan/search/stable pages/playlists/migrations; offline/interrupted scans, moves/duplicates, corrupt DB/disk-full |
| UI/themes | Both layouts preserve session; keyboard/focus, resize/DPI/Unicode/IME, drag/drop; archive traversal/bombs, huge art/stale work |
| Originals | Bytes/modification metadata and write-call observations before/after normal/error journeys; read-only mounts/shares |
| Privacy | Source/profile cache and secret isolation; store failure, log/export leaks, cross-source access, dependency/input/abuse review |
| Future remote | Pair/reconnect/revoke; unauthorized/replayed/stale commands, expired invitations, concurrent controllers, oversized traffic |
| Future writes | Draft/preview/confirmation; cancel, conflicts, lost permissions, interrupted/partial writes and recovery |

Use owned/redistributable fixtures. Signals cannot prove OS/DAC output. EQ needs tones/impulses/sweeps/peaks plus listening/device checks; gapless
needs format-change and remote-startup checks.

## Performance protocol

Use deterministic 100k/500k/1M catalogs: varied albums/artists, duplicate titles,
Unicode, query selectivity; real media for audio. Record
hardware/OS/filesystem, device/format, engine build/config, source, network RTT/
throughput, volume, caches and background work. Report repeated cold/warm p50/p95/p99.

Measure startup-to-controls (first-ever/cold/warm), UI acknowledgement, play/pause
to output/cessation, seek-to-target audio/error, search-to-first/stable pages, idle/play/
scan/scroll peak/settled RAM (cache/GPU separate), CPU, UI stalls and underruns.
Start latency clocks at command submission, separate from acknowledgement. Define
CPU accounting for idle/play/EQ/scan. 01:43 means 103-second audio, not a cursor. Exercise SSD/SMB, rapid seeks,
scans and slow/offline networks within bounds. Provisional UI acknowledgement:
p95 <= 50 ms on reference hardware. Freeze budgets with workload evidence and user review.

## Gates

1. Local audio: controls, EQ, gapless, output recovery and latency/resources.
2. UI: layouts, accessibility/scaling, idle/startup.
3. Local use: scans/search, scale, controls during scans.
4. Jellyfin: browse/play; isolation, bad/oversized responses, timeouts,
   cancellation, recovery and local regressions.
5. Then SMB: bounded reads, seek, cancellation, recovery and local regressions.
   Crossfade keeps its separate gate.
6. Release: Linux devices/packages, preservation/privacy, reproducible builds/licenses.
   Web needs browser journeys/component tests; mobile needs real lifecycle,
   interruption/background checks beyond emulation.

Test playlists if needed after Jellyfin; skins near the end.
Retain baselines; cover shared boundaries and report gaps.
