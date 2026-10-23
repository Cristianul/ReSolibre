#!/usr/bin/env python3
"""Require continuous source samples while changing TRIM/FADE on a private audio graph."""

from pathlib import Path
import os
import tempfile
import time

from check_pcm_conversion import build_worker
from check_playback_responsiveness import fingerprint, encode_flac
from private_audio import PrivateAudio
from responsiveness_probe import Probe, start_monitor
from responsiveness_signal import BLOCK_FRAMES, MonitorBuffer, RATE, write_markers
from pcm_signal import require
from check_equalizer import Samples, write_tone, collect, amplitude


def trial(directory, source, binary):
    """Toggle every mode in mid-song, retaining sample identity through cold analysis and cached changes."""
    directory.mkdir(mode=0o700)
    os.link(source, directory / "private-input.wav")
    before = fingerprint(source)
    with PrivateAudio(directory) as audio:
        audio.start_server()
        monitor = start_monitor(audio)
        probe = Probe(audio, ["env", "RESOUL_SESSION_PROBE=1", str(binary)],
                      monitor, MonitorBuffer(), lifetime=30)
        probe.acknowledge(probe.send("open"))
        audio.attach(probe.worker, min(probe.deadline, time.monotonic() + 5))
        probe.observe((0, time.monotonic_ns()))
        probe.acknowledge(probe.send("seek 10"))
        probe.observe((0, time.monotonic_ns()), 10.0, tolerance=RATE)
        probe.hold(0.3)
        probe.acknowledge(probe.send("eq 0"))
        for trim, fade in [(1, 0), (0, 0), (0, 1), (0, 2), (0, 3),
                           (0, 0), (1, 3), (0, 3), (1, 3), (0, 0)]:
            probe.acknowledge(probe.send(f"edges {trim} {fade}"))
            # Do not re-confirm after a command: that would hide its gap/repeated samples.
            probe.hold(1.0)
            probe.acknowledge(probe.send("edges-ready"))
        request = probe.send("pause")
        probe.acknowledge(request)
        probe.observe(request, silence=True)
        active = [block.position for block in probe.history.blocks if block.position is not None]
        require(active, "missing pre-pause position")
        last_position = active[-1]
        for trim, fade in [(1, 3), (0, 1), (0, 2), (1, 0), (0, 0)]:
            probe.acknowledge(probe.send(f"edges {trim} {fade}"))
            probe.hold(0.08, silent=True)
        request = probe.send("resume")
        probe.acknowledge(request)
        resumed = probe.observe(request)
        require(abs(resumed["position"] - last_position - BLOCK_FRAMES) <= 2 * BLOCK_FRAMES,
                "paused edge changes moved source position")
        probe.hold(0.3)
        probe.finish()
    require(fingerprint(source) == before, "live processing changed source")


def live_envelope(directory, binary, disable):
    """Measure runtime fade-out parameters at EOF without a seek after enabling/disabling them."""
    directory.mkdir(mode=0o700)
    source = directory / "private-input.wav"
    write_tone(source, RATE, 1000, .04, seconds=6)
    before = fingerprint(source)
    with PrivateAudio(directory) as audio:
        audio.start_server()
        samples = Samples()
        probe = Probe(audio, ["env", "RESOUL_SESSION_PROBE=1", str(binary)],
                      start_monitor(audio), samples, lifetime=15)
        probe.acknowledge(probe.send("open"))
        audio.attach(probe.worker, time.monotonic() + 5)
        collect(probe, .3)
        probe.acknowledge(probe.send("seek 3"))
        collect(probe, .3)
        probe.acknowledge(probe.send("edges 0 2"))
        collect(probe, .4)
        probe.acknowledge(probe.send("edges-ready"))
        if disable:
            probe.acknowledge(probe.send("edges 0 0"))
        while not probe.events:
            probe.pump()
        require(list(probe.events) == ["eof"], "live envelope failed before EOF")
        drain = time.monotonic() + .35
        while time.monotonic() < drain:
            probe.pump()
        values = list(samples.values)[::2]
        last = max(index for index, value in enumerate(values) if abs(value) > 2)
        for distance in (.2, .5, .8):
            center = last - round(distance * RATE)
            half = round(.04 * RATE)
            require(center >= half, "missing final envelope samples")
            measured = amplitude(values[center-half:center+half]) / .04
            expected = 1 if disable else distance
            require(abs(measured - expected) < .035, "runtime fade envelope is incorrect")
        probe.finish()
    require(fingerprint(source) == before, "live envelope changed source")
    print(f"PASS runtime fade-out {'disabled' if disable else 'enabled'}: measured sample envelope", flush=True)


def main():
    """Use generated WAV/FLAC markers and owned servers; never access desktop audio or user music."""
    binary = build_worker()
    with tempfile.TemporaryDirectory(prefix="resolibre-live-edges-") as temporary:
        root = Path(temporary)
        wav = root / "source.wav"
        flac = root / "source.flac"
        write_markers(wav)
        encode_flac(wav, flac, root)
        for source in (wav, flac):
            trial(root / source.suffix[1:], source, binary)
            print(f"PASS {source.suffix[1:]}: ten live + five paused TRIM/FADE changes, continuous source samples", flush=True)
        for disable in (False, True):
            live_envelope(root / f"envelope-{disable}", binary, disable)


if __name__ == "__main__":
    main()
