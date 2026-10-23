# Local desktop validation

Current follow-up record, 2026-10-22. Read with the
[feature](../architecture/features/local-desktop-player.md). Reference environment:
Linux x86_64, debug Rust 1.99, Slint 1.18.1, libmpv 0.41.
[Commands](../development/desktop-environment.md).

## Scrub-to-EOF correction

Two separate failures were reproduced: a late seek reused the original ten-second
load deadline while audio formats disappeared; repeated endpoint seeks were sent
after native teardown but before the queued EOF event was consumed (`seek`, -12).
The first fix alone did not cover the second case.

Each continuous initialization wait now has its own deadline. Native events are
serviced between commands. Seek/EQ command rejections yield to pending terminal
events only when the loaded engine confirms teardown; real errors still propagate.
Pointer gestures retain their original track generation until release/cancel, so
holding at the endpoint cannot scrub the next occurrence. A fresh gesture works.

Regression coverage includes three-track repeated seeks, a compiled slider gesture
with native silent playback, late seeks after ten seconds, final EOF and a direct
adapter check that delivered EOF still makes subsequent seeks fail. Removing the
gesture guard makes the UI regression fail. Source bytes/modification metadata are
checked for supplied-codec runs. These checks do not prove physical audibility.

Checks for this correction passed:

- Workspace/all-target Rust: 53 passed, 12 opt-in tests excluded. Desktop build,
  rustfmt, all-target Clippy `-D warnings`, docs/whitespace checks passed.
- Five supplied MP3/FLAC compiled-UI gesture journeys passed on silent native output;
  hashes, sizes, mtimes and modes unchanged. Generated three-track regression passed.
- Three private-output baseline/absent/loss/recovery journeys plus malformed input
  passed; genuine failures were not reported as successful EOF.
- Production-session virtual-marker smoke: three WAV and three FLAC trials passed,
  including protocol/output-loss, malformed input and dead-monitor negatives.
  This is not a repeat of the baseline 100-trial/resource gates.

## Session persistence

The [restart feature](../architecture/features/session-persistence.md) now saves
queue/order/selection, volume, modes and active EQ without autoplay. Workspace tests:
59 passed, 12 opt-in tests excluded. Native restoration and production controller
restart/close tests pass, including pending changes, mute and empty queues.
Storage checks cover autosave, bounds, corruption, non-UTF8/missing/duplicate paths,
profile locks/isolation, special files and failed-write preservation. Clippy,
format/build and docs checks pass. No new dependencies, networking or music writes;
full resource/device and power-loss testing were not repeated for this slice.

## Earlier evidence and remaining gates

The [initial baseline](local-desktop-baseline.md) preserves full volume captures,
100-trial WAV/FLAC marker results, resource measurements, dependency audit snapshot
and earlier failures. Read it with profile `local-desktop-baseline`; it is historical,
not a repeated full performance/dependency audit. Later records:
[queue modes](../plans/implemented/shuffle-repeat-plan.md),
[EQ](../plans/implemented/equalizer-plan.md).

Approved reference budgets remain 200 MiB desktop / 100 MiB native audio and UI
acknowledgement p95 <=50 ms. Native/kernel calls have no universal cancellation
bound; close remains responsive while waiting for owned cleanup. Full physical
listening/device, keyboard/focus/DPI/accessibility, portal cancel/close, startup/idle
and distribution acceptance remain open. Catalog/named playlists and everyday local use precede SMB. The [local plan](../plans/local-desktop-player-plan.md) stays active.
