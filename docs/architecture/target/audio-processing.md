# Audio processing target

App volume and [TRIM/FADE](../features/playback-edges.md) work: TRIM -55 dBFS,
0.5 s minimum, 50 ms margin; FADE 1 s. Playback respects TRIM; analyses bypass it.
[MIX](../features/dj-mix.md) adds Linux overlap and bounded beat matching; normalization remains planned.
Read with [audio/EQ](audio-and-equalizer.md).

## Application-only Master Volume

UI, media integration and remote commands control only ReSolibre's generated signal,
never OS/global or other-app volume. 0% is silence; 100% is unity gain (0 dB) at
this stage, with explicitly enabled DSP still applying. OS volume is downstream.
The local slider uses squared percentage; default 50% is 25% amplitude.
The adapter inverts mpv's cubic curve with a 120 ms/full-scale slew guard.
Short dial clicks request a two-second smoothstep glide; manual changes cancel it.
The worker owns timing and persists the destination; stationary holds >=1 s do nothing.

With libmpv use software `volume`, not backend-dependent `ao-volume`/`ao-mute`.
Validate ranges/non-finite values and smooth changes. Per-device persistence is
desired; decide playback-host versus output-endpoint keys. Remote volume belongs
to the target session, never the controller's OS mixer.

## Normalize Volume: ON / OFF

Apply one track-level policy to every song, irrespective of album, genre, edition
or source. No album mode/per-track UI controls. Measure perceived loudness with
BS.1770/EBU R128 or compatible ReplayGain principles; peak amplitude or spectral
extent alone is insufficient. Loudness target and Master Volume are distinct.

Measure each actual representation, not a shared recording ID. Cache compatible
measurements outside music; compute missing values in bounded background jobs.
Without results, play uncorrected and expose pending/unavailable diagnostics. Apply
late results on later playback; explicit toggles may smoothly change processing.

Prefer fixed track gain to preserve dynamics. Limit positive gain against overload;
reaching the target must not force compression/distortion. Bound silence/near-silence
amplification. Numerical target, peak margin and combined EQ/mix headroom need
validation before implementation; no LUFS target or limiter is selected yet.

## MIX and gapless

MIX follows FADE and prepares the actual next queue occurrence. Current BPM stays automatic with MIX off. Keep libmpv, portable policy
and OS-specific native scheduling. [Current behavior/tests](../features/dj-mix.md)
define the Linux prototype; other platforms and physical output remain unvalidated.

MIX overlaps about 10% of the shorter track with opposing gain ramps. Additional
bounded beat-free cuts affect only the joining edges; internal pauses stay intact.
User-approved speed adjustments are bounded to ±20% per stream. Prefer 1:1;
otherwise use 1:2, 1:3, 2:3 or inverses with the smallest rate change and align
shared cycles. Never clamp speed off-grid. Bring both tracks gradually to the
geometric BPM midpoint over up to six source seconds, then hold for one second
before overlap; prepare the muted incoming track at that midpoint. Keep matching limits.
Start the six-second return one second before handover; continue afterward.
Tune approach/lead/duration in core; preserve the shared ratio during overlap
and freeze on pause. Validate sustained pitch and <=35 ms captured attack alignment;
the user rejected widening that limit. Prime native processing and retain its phase
through unity. Normal playback and source files stay unchanged.
TEMPO means rates without verified phase; SYNC or the ratio means confirmed phase.
Missing evidence/unsupported ratios retain crossfade and supported MIX cuts.

Keep one authoritative queue/session, bounded preparation, pitch preservation,
explicit controls/cancellation, source isolation and correlated-peak headroom.
Validate half/double ambiguity, variable tempo, short tracks, next-source failure,
seek/pause/skip, queue edits and changes during overlap. Common physical timing and
listening quality must be demonstrated; two handles alone prove neither.

Gapless remains a separate contract with MIX off. Do not exempt same-album or CUE
transitions silently when they become supported. Never claim unavailable audio is
ready or wait without bounds. No replacement engine is authorized.

## Audio Signal Path

Show the actual graph: decoder, track gain, overlap/stretching, EQ, conversion,
volume and output where active, in implemented order. Show source codec/container,
encoded rate/bit depth when known, decoded/output API formats, backend, resampling,
DSP and known server transcoding. Distinguish server reports from observation;
unknown/stale values stay explicit. PCM storage width or matching rates do not
prove original bit depth, DAC format, provenance or bit-perfect output. Keep this
contextual view outside the main controls.

## Evidence

[Acceptance cases](../../testing/expanded-features-test-plan.md) are planned;
[analysis](analysis-and-radio.md) owns background jobs. Technical references,
not adopted dependencies or measurements: [mpv](https://mpv.io/manual/stable/),
[FFmpeg](https://ffmpeg.org/ffmpeg-filters.html),
[beat tracking](https://librosa.org/doc-playground/latest/generated/librosa.beat.beat_track.html),
[time stretching](https://www.breakfastquay.com/rubberband/integration.html).
