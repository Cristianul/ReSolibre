# Track information and spectrum plan

Status: active, partial implementation. The [technical strip](../architecture/features/track-information.md)
and [live spectrum](../architecture/features/live-spectrum.md), artwork button,
[spectrogram/BPM](../architecture/features/track-analysis.md) work on Linux.
Platform/device/resource acceptance remains open; this plan is not yet archived.
Approved: technical details below the title, full-track analysis from an artwork
hover button, and live spectrum between playback controls and volume. Keep artwork.

Read root/core/desktop rules, maps, security and tests via the `track-analysis-plan`
profile. Radio/model research is outside this task.

Platform requirement: track information and analysis must support Linux, Windows
and Android, with iOS considered. OS adapters may differ; typed results, analysis
and interpretation stay shared. A Linux-only tap does not satisfy this contract.

## Evidence and representation

The worker/snapshot now expose selected codec and source rate/channels to the UI.
Decoded Float32/Signed32 storage is not encoded bit depth; it is not displayed.
`media_formats.rs` selects candidates only.

Show observed codec, sample rate and channel count below the title, for example
`Lossless · 44.1 kHz · Stereo`. Add source bit depth only from trustworthy encoded
metadata, never from the decoder's working precision. Display mono/stereo or N
channels without guessing a surround layout. Keep actionable errors visible.

A codec can use lossless compression while containing previously lossy audio.
Use `Lossless codec` / `Lossy codec` / `Unknown`, separate from `Origin unknown`.
Do not classify by filename. Mixed-mode codecs require stream evidence or Unknown.
No spectral cutoff, bitrate, checksum or high sample rate certifies provenance.
Spectral observations may suggest prior processing; never report "true lossless"
or a numeric authenticity probability without validated supporting evidence.

The analysis window must explain measurements in plain language: observed codec,
sample rate, trustworthy source bit depth, channels and estimated sustained
bandwidth, with units and unknown/unavailable states. Separate measured facts
from suspected prior processing and its limitations; absence of suspicious
features does not verify origin. Do not turn high sample rate, bit depth or
bandwidth into an overall listening-quality score.

## UI contract

Reuse the shared physical ActionButton/Icon, palette and window lifecycle.
Place the live spectrum in the flexible space between the main playback buttons
and volume dial. It stays visible and runs automatically during playback without
a hover action or enable toggle. Bars show current energy from low to high
frequency; they are real audio measurements, not a decorative beat animation.
Render continuous, unsegmented light bars directly on the chassis, as light
diffused beneath translucent plastic: no separate display, inset, frame, grid or
digital LED blocks. Use softly diffused edges and restrained palette colors;
this spectrum treatment does not change solid button/state indicators. Preserve
control sizes and usable spacing at the minimum window width.
Motion must feel continuous: use time-based attack/release smoothing with quick
response to transients and a gentler fall, without stepped heights or lingering
peak markers. Keep analysis cadence separate from presentation cadence; render
smoothly between measured targets without inventing beats or overshooting values.
In a corner of the artwork, reveal one full-track analysis button on pointer
hover and keyboard focus, with a clear label/tooltip. Keep the artwork visible.

Full analysis uses a separate grouped window with progress, Cancel and explicit
track identity. Closing cancels/releases its work. Changing tracks must not label
an old result as the new track. On pause, freeze live data without idle animation;
on stop/EOF, clear bars. On seek/track change, discard stale frames and wait for
new samples. Hidden/minimized views suspend visual work and resume automatically;
respect reduced motion without fabricated activity. Both palettes share the
implementation.

## Implementation sequence

1. Expose typed current-stream metadata through the existing worker/snapshot;
   present the compact strip. No full decode, extra engine or FFT is needed.
2. Prove cross-platform access to live PCM. The current libmpv client API controls playback but
   has no direct PCM callback in this integration; its render API is for video.
   Benchmark a supported tap/adapter before committing dependencies or changing
   playback engines. Assess Windows/Android/iOS viability alongside Linux and
   report any required engine/platform tradeoff for a decision.
   Live PCM comes from playback, never the system mix or a second decode.
   Decision: retain libmpv and pursue the filter. The [Linux proof](../testing/live-tap-validation.md)
   preserves captured audio. Linux lifecycle/UI work; exact timing and platform builds remain open.
3. Build one reusable FFT/STFT module with separate bounded live and offline
   consumers. Playback never waits for analysis: drop visual frames on overload.
   Analyze original decoded audio for file diagnostics, independent of EQ/volume;
   label the live tap's actual pre/post-DSP location rather than implying DAC data.
4. Add the live transport-row spectrum and grouped analysis window once real data exists.
   Full analysis is on demand, one cancellable job, streamed PCM and aggregated
   time bins. Preserve channels when accumulating power to avoid phase cancellation.
   Show frequency/time/dB scales and distinguish sample rate from spectral extent.
5. Validate sustained-bandwidth estimates on controlled/reference material before
   adding suspected-transcode hints. An inconclusive result is valid. No automatic
   provenance certification. [Library batches](../architecture/features/library-analysis.md)
   are approved; no implicit scan. [Persistent caching](../architecture/features/analysis-cache.md)
   is now approved and implemented for supported local audio; keys exclude text tags.

## Approved tempo scope

BPM and observed attacks are approved. Current rhythm runs automatically (excerpt,
then full timeline), ahead of other jobs. Stable overall tempo may recover sparse
notes whose measured attacks support its phase; silence and real tempo changes
must not become fabricated beats. Variable recordings show local estimates.
[MIX](../architecture/features/dj-mix.md) owns overlap and Linux proof: its key follows
FADE; only MIX analyzes the next track. Separate BPM/phase matching (TEMPO/SYNC). Overlap targets 10% of the shorter track,
with opposing gain ramps and additional bounded beat-free cuts at joining edges.
Return BPM over four seconds, starting one second before handover; tune in core.
Physical/platform/resource acceptance remains open.

## Acceptance and resource gate

Keep the approved 200 MiB desktop/100 MiB audio budgets; no increase is approved.
Measure any native decoder/tap overhead as well as bounded Rust buffers. Target
smooth presentation at 60 frames/s where supported, with time-based smoothing
that remains consistent at other refresh rates. About 20 analysis updates/s is
only an initial experiment, not the visual frame rate or a fixed requirement;
tune cadence and latency against real audio and resource measurements. These
are prototype targets, not benchmark claims. Keep FFT/result storage fixed;
large tracks must not retain their full PCM/matrix.

Test tones, sweeps, silence, mono/stereo/multichannel, phase-inverted channels,
true lossless and generated MP3-to-FLAC, filtered lossless, upsampling and noise.
Check codecs inside misleading extensions/video containers, unknown metadata,
seek/skip/stale completion, cancellation/close, malformed/long files and resource
limits. Verify no music writes, network references or foreign audio capture.
Compare playback responsiveness with analysis enabled/disabled, and inspect native
hover/focus control, always-enabled live placement at minimum width, pause/stop/
seek/EOF and minimize/resume, reduced motion, scales, 1x/2x, palettes and grouped-
window behavior. Inspect continuous motion, transient response, release tails,
frame pacing and audio/visual delay on real playback; compare CPU/GPU cost with
the spectrum hidden and reject expensive diffusion effects that exceed budgets.
Verify that explanations distinguish measurements, suspicions
and unknown origin without a quality/authenticity badge.

## References

[Spek](https://www.spek.cc/about) provides the spectrogram interaction reference.
[FLAC FAQ](https://www.xiph.org/flac/faq.html) describes lossless encoding guarantees.
[mpv client API](https://github.com/mpv-player/mpv/blob/v0.41.0/include/mpv/client.h)
and [render API](https://github.com/mpv-player/mpv/blob/v0.41.0/include/mpv/render.h)
define the current integration boundary. These references select no new dependency.
