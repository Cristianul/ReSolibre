# SMB playback feasibility plan

Status: deferred until the [roadmap](product-roadmap.md) everyday-local-use gate
and the subsequent Jellyfin phase. Local playback features/EQ, layouts and catalog
must be validated together. Skins and saved playlists do not gate SMB. The minimal desktop window
alone does not unlock SMB. No implementation or SMB measurements yet;
scope/acceptance unchanged. Jellyfin moved ahead of SMB on 2026-10-23.

Objective: play and seek through ReSolibre-owned SMB2/3 reads; stop within a
bounded time when the source stalls. Preserve local playback and originals.
The local session now owns a command worker and fresh per-track engines.
Source workers and SMB remain unimplemented.

## Scope and user involvement

Use one source/track, generated WAV/FLAC and the private audio monitor. UI,
scanning, playlists, gapless, EQ, downloads, saved credentials and automatic
reconnect are outside scope. Recovery means a fresh open after prior cleanup.

| Stage | Access needed from the user |
| --- | --- |
| Local source bridge and simulated failures | None |
| Direct SMB adapter and isolated server tests | None if a private test server can be provisioned |
| Validation against the user's server | Reachable server/share, selected files and permitted authentication |

For real validation: an address such as `smb://server/share/folder`, guest or
authenticated access, username/domain if needed, VPN requirements and two or
three selected tracks (prefer WAV/FLAC, one over 103 s). A mounted local path
does not validate direct SMB. Do not recursively scan the library.

Use a local hidden-input password prompt or an explicitly selected supported
secret-store entry. No passwords in chat, URLs, arguments or tracked files; no
private endpoints/usernames/filenames in reports. The user's share is read-only.
Inject faults only in owned fixtures; never stop or alter the NAS or write there.

## 1 Verify the source boundary locally

Verify libmpv/libsmb2 ABI, cancellation, threading, dependencies, advisories and
licenses; pin tested versions. Keep native code outside resolibre-core.

Prototype a read-only custom stream alongside the retained-file descriptor
path. Define typed bounded reads, absolute byte seeks, size/capabilities,
cancellation and safe errors. libmpv translates seconds into byte seeks.

Callbacks must not call back into their libmpv instance or unwind across FFI.
Keep stream state alive until close and registrations until engine destruction.
Cancellation must wake blocked work without waiting for that work's lock.
Distinguish session cancellation from superseded read/seek requests; do not
assume libmpv invokes its cancel callback for each seek. Reject stale completions
using source/session/request identity before making their bytes observable.

Define bounded command/I/O ownership: commands remain responsive during I/O;
open, stop and Drop cancel and join owned work. Killing the test process is not
application cleanup. No detached unbounded workers or implicit cross-track engine
reuse. Resolve ownership/API changes in the owning target before integration;
preserve existing local behavior.

Exit: marked WAV/FLAC matches the descriptor baseline. Tests cover short reads,
real EOF, injected errors, delayed reads, a blocked read, cancellation during open
and shutdown, malformed offsets/sizes, stale requests and repeated destruction.
A temporary lack of data must not become EOF. No use-after-close or deadlock.

## 2 Implement and verify direct SMB

Use asynchronous libsmb2 with one context owner and finite DNS/connect/auth/read
deadlines. Bridge completions to stream callbacks without blocking commands.
Prove per-operation cancellation versus context closure; late callbacks must
not access released memory.

Open only selected regular files with read access. Keep authentication and SMB
addresses in the source adapter; libmpv receives only an opaque registered source
identifier. Preserve the native configuration/protocol restrictions. Reject
cross-source identifiers, path escapes and content-triggered connections. Define
signing/encryption requirements for the chosen server; no silent security downgrade
or SMB1 fallback. Credentials stay session-scoped with safe diagnostic categories.

Use an owned loopback SMB server, unprivileged port, generated read-only files
and ephemeral credentials; leave host services unchanged. Missing tooling is
a recorded prerequisite, not a reason to stop independent local work. Simulated
reads do not establish SMB integration.

Test authenticated and permitted guest reads, wrong credentials, access denied,
missing files, unsupported negotiation, source isolation, short/truncated data,
disconnect during open/read/seek, and a live connection that stops returning data.
Verify explicit fresh recovery and repeated cleanup. Use wrong-password and
disconnection tests on the isolated server, avoiding lockouts on real accounts.

Exit: a remote generated WAV/FLAC plays and seeks through actual SMB traffic;
no media writes, file creation, renaming or deletion occur. Check original hashes
and modification metadata and observe client/server operations in the lab.

## 3 Measure controls and bounded resources

Reuse the existing marker/monitor protocol: output identity and continuity,
pause/resume, seek to 103 s and reversals, paused seek, bursts with a distinct
final target, EOF and safe errors. Reuse its source/log/capture limits and cleanup.
Run 100 normal-condition journeys per codec over three sessions and 60 lifecycle
cycles per session. Repeat faults three times; retain every attempt/failure.

Before acceptance runs, freeze the network conditions, buffer/in-flight byte and
request caps, open/read deadlines and stop/cleanup limits in the validation record.
Measure from command submission, not after a blocked read finishes. Separate
command acceptance, source wait, monitor receipt and full resource release.
Include slow throughput, latency and a stalled source beyond the configured
deadline; log actual injected conditions. A surviving process or stale output
fails cleanup even if a controller has already acknowledged stop.

Preserve the local responsiveness comparison gates. Start with the 100 MiB
memory objective and measure the additional SMB working set.
Report p50/p95/p99/max, CPU, active/post-close RSS, descriptors, threads, queued
bytes and bytes transferred. Bound read-ahead; never fetch the whole file merely
to pass seek tests. Network budgets require a recorded workload and user review;
do not relax a failing bound after seeing results without explaining the cause.

Exit: local regressions pass; remote output, cancellation, cleanup and resources
meet the frozen limits. Unresolved native cancellation/ownership stops broader
integration until a concrete alternative is evaluated.

## 4 Validate the user's source and close

On the supplied share, test selected files: playback, pause/resume, seeks and
repeated opens. Record server/dialect, codec and network conditions anonymously.
Use generated markers for precision claims; ordinary music alone cannot prove
sample identity. Report virtual monitor and physical listening evidence separately.

Run source/FFI, core-only/workspace, Python, CLI privacy, PCM and output-failure
checks, rustfmt and Clippy. Verify config/source/credential isolation on failure.
Create `docs/testing/smb-playback-validation.md` with evidence; update feature/
maps/targets/profiles, check docs/whitespace and stage new files without committing.

Archive after lab and real-server acceptance. If real access is unavailable,
keep this plan active with completed stages and exact missing input.

## Reading routes and references

Profile: `smb-playback-plan`. Implementation routes:
`core-and-platform-contracts`, `playback-local-and-smb`, `native-environment`,
`output-failure-validation` and `playback-responsiveness-validation`.
Select by stage; complete flows stay within 32 KiB.

[Source target](../architecture/target/library-and-sources.md),
[libmpv stream API](https://github.com/mpv-player/mpv/blob/v0.41.0/include/mpv/stream_cb.h),
[libsmb2](https://github.com/sahlberg/libsmb2).
