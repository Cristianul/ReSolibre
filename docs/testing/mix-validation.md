# MIX audio validation

Use the `dj-mix-validation` reading profile for native signal tests.
Read [MIX behavior](../architecture/features/dj-mix.md) first;
[native setup](../development/native-environment.md) supplies library paths.

## Procedure

`python3 tools/check_mix.py` uses generated channel-separated attacks and private
PipeWire/Pulse servers. It measures overlap, onset alignment and completed speed
recovery; it never attaches to desktop audio. Natural TRIM captures at
120→124, 124→120 and 120→120 BPM retain the 35 ms onset-error limit.
`--scenario long` checks 24-second overlap/gains; `--scenario wide-tempo` checks
120↔142 BPM. `--scenario ratios` checks all six non-unity ratios and unverified
1:3 TEMPO: independently measured pulse rates, common attacks and recovery.
`--scenario smooth-return` checks late onset and post-handover 1:1/1:3/3:1 recovery.
Midpoint pulse periods must stay within 15 ms individually and 8 ms on average;
regular fixtures also reject a lost outgoing attack during tempo preparation.
Settled-rate checks exclude the final 100 ms so shutdown cannot turn a truncated
pre-echo into a complete attack. Overlap timing tolerances remain unchanged.
`--scenario sparse` checks omitted attacks; `--scenario tempo-only` checks rates
without phase. `--scenario pipewire` uses native PipeWire; `--scenario invalid-clock`
requires continued overlap after one invalid optional clock. Transport errors stay strict.

`python3 tools/check_mix_pitch.py` captures 55/440 Hz tones through the production
MIX filter and tempo curves at 100↔120 BPM. Four captures cover outgoing live
activation/approach and incoming recovery in both directions. Sliding 200 ms
windows bound pitch drift to 10 cents, residual distortion to 12% and RMS variation
to 10%, including unity settling. Analyzer negative controls reject pitch shifts,
phase jumps, flutter, silence, dropouts and insufficient evidence:

```sh
python3 -m unittest discover -s tools -p 'test_mix_pitch.py' -v
```

Tests preserve original bytes/metadata, reject invalid native speed inputs and
ignore hostile external configuration/scripts. Child lifetimes and output files
are bounded. The native session suite also covers pause, seeking, MIX-off before
and during overlap, handover and scanning after MIX. `check_pcm_conversion.py`
requires ordinary session PCM and bundled taps to remain byte-identical to their
controls, alongside conversion and malformed-input checks.

## Measured evidence and limits

On 2026-10-23, libmpv 0.41.0 with Rubber Band 4.0.0 passed all four pitch captures:
maximum drift was 0.01 cents; worst residuals were 3.06% at 55 Hz and 4.14% at
440 Hz during outgoing activation/approach, and 0.07% during incoming recovery.
The 48 kHz R3 graph pads 2048 samples and removes 2048 output samples, matching
the library's real-time startup contract. Separate channels preserve fixture
isolation. A persistent 1 ppm pitch guard prevents the phase reset at exact unity
without changing the final tempo. Normal playback does not install this graph.

The old 10 ms scaletempo2 search produced 98.8% residual at 55 Hz in a fixed-speed
comparison. Wider search windows improved tones but exceeded the retained 35 ms
attack limit. Unprimed R3 lost samples on live insertion, and exact-unity R3
produced a localized phase jump. The final tests cover each of these failures;
changing only the BPM curve or averaging pitch over the whole song would miss them.

Long captures at 120↔142, 80→220 (1:3) and 220→80 (3:1) measured at most 17 ms
attack error while retaining the six-second recovery. These are generated-signal
measurements, not a listening acceptance or physical-device result. Fixtures do
not establish accuracy on arbitrary music or variable-tempo phrases. Private
captures are not 100-trial CPU/RAM acceptance gates; those budgets remain open.

The extended matrix passed all six shared ratios (maximum 28 ms), PipeWire
(34 ms), invalid-clock, sparse, rate-only and long crossfade cases. Validation
also passed 63 core tests, 76 native audio tests across the library/integration
suites, the allocator check, eighteen PCM captures, three analyzer tests,
Clippy, formatting and documentation checks. The desktop binary was rebuilt.
