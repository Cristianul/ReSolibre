# Current audio information

## Read When

Changing the current stream's technical facts or compact player information line.
Use root/core/desktop rules, area maps, security and the test plan.

## Do Not Read For

Tags/covers, output negotiation, live spectrum or full-track analysis. See
[metadata](track-metadata.md), [playback](local-playback.md) and the
[active analysis plan](../../plans/track-analysis-plan.md).

## Current Behavior

The worker queries the selected libmpv audio codec and decoded source rate/channel
count at most twice per second while loaded. A typed, fixed-size `StreamInfo`
travels with the current snapshot. No extra engine, probe, decode or cache is
created. Missing properties remain unknown; optional query failure clears the
facts and logs one sanitized warning per opened track without stopping playback.

During Play/Pause, the line below the title shows e.g.
`Lossless · 44.1 kHz · 16-bit · Stereo · ≈ 120.0 BPM`. Mono/stereo are named; other counts are displayed as
`N channels`, without guessing speaker layout. Rates retain whole-Hz precision.
Errors/import notices take precedence; loading, scanning and unavailable/pending
TRIM/FADE retain their state messages. Stop/EOF/failure/replacement clear facts;
new generations cannot inherit prior facts. Automatic [rhythm detection](track-analysis.md)
starts a quick excerpt, then a full current-track timeline even with MIX off.
Both take priority over other analysis. Variable recordings show measured local BPM
at the playhead (including seek); unmeasured sections show `BPM unavailable`.
During preparation the line says `BPM analyzing…`. Values remain estimates; no
number is fabricated for silence or uncertain rhythm.

Codec names come from an exact allowlist of native identifiers, never filenames
or native descriptive text. Known lossless encodings use a compact green/cyan
**Lossless** badge with a Lucide infinity glyph; other encodings retain their codec
name (MP3, AAC, etc.). Hybrid WavPack/DTS stay named without a lossless claim.
The badge describes encoding, not original provenance. Unknown identifiers stay `Unknown codec`. Decoder
working precision is not encoded bit depth. A bounded native-FLAC STREAMINFO read
supplies bit depth only when source rate/channels match; other formats stay unknown. No lossless-origin,
authenticity, spectral bandwidth or listening-quality verdict is produced.

## Key Files

- `crates/resolibre-core/src/track_info.rs`: typed facts and fixed codec labels.
- `crates/resolibre-audio/src/formats.rs`: native property reads and codec mapping.
- `crates/resolibre-audio/src/session/{mod,worker}.rs`: lifecycle and bounded cadence.
- `apps/desktop/src/track_info.rs`, `ui/player.slint`: description and notice priority.
- `apps/desktop/ui/components/lossless-badge.slint`, `ui/assets/lossless.svg`: encoding badge (Lucide infinity, existing license).
- `crates/resolibre-core/src/analysis/tempo.rs`: portable playhead-to-measured-BPM lookup.

## Tests

Native `local_playback` tests cover generated PCM, FLAC/MP3 with misleading
extensions, unloaded/EOF states and original-file preservation. Session tests
cover pause, stop, restart, failed selection and replacement. Adapter unit tests
reject arbitrary identifiers/invalid numbers; desktop tests cover partial facts,
multichannel labels, lifecycle visibility, local BPM/seek/gaps, conservative codec
badges and compiled error precedence. A real native session test preempts a
600-second library job, measures a 120→140 BPM current timeline past the 64-second
excerpt with MIX off, then prioritizes its requested plot.
The native preview includes the technical line at minimum width in both palettes.

Executed on Linux: workspace/all-target tests, Clippy with warnings denied and
the isolated native preview at 1x/2x passed. Source-versus-output assertions use
the restricted stereo/Float32 sink to retain mono source facts. The first manual
2x preview used the wrong scale variable; the existing native harness passed at
both scales. The session responsiveness harness passed three WAV and three FLAC
trials, plus malformed-input/dead-monitor checks, with resource trials disabled.
No Windows/mobile execution, live-spectrum frame pacing, physical
device quality or new whole-app memory baseline is claimed.

## Edge Cases

Unknown codec does not reject playable music. Reconfiguration can temporarily
hide fields; periodic reads retry. These are non-atomic source observations, not
physical-device measurements. Encoded bit depth awaits a trustworthy source;
the [live spectrum](live-spectrum.md) now consumes PCM on Linux; platform gates remain open.

`StreamInfo` and codec labels are portable core values. Native reads remain in the
existing Linux audio adapter; Windows/Android/iOS execution is not implemented or
validated by this slice. Their adapters must supply the same facts and unknown
states, with shared interpretation. No PipeWire dependency was added to production.

PCM investigation: installed mpv 0.41.0 headers/exported APIs expose video
rendering and source-input callbacks, not a decoded-audio callback. The
[render API](https://github.com/mpv-player/mpv/blob/v0.41.0/include/mpv/render.h)
is not an audio tap. A private PipeWire/Pulse monitor setup accepted an exact
stream selector but produced no samples without graph wiring; this is not a
validated tap or benchmark. A later [isolated LADSPA proof](../../testing/live-tap-validation.md)
preserves captured audio; production timing and cross-platform packaging remain open.

## Related Docs

[Analysis target](../target/analysis-and-radio.md),
[test plan](../../testing/test-plan.md), [appearance](player-appearance.md).
