#!/usr/bin/env python3
"""Verify application software gain using owned tone fixtures and captured native PCM."""

from pathlib import Path
import math
import tempfile
import subprocess
import sys

from check_audible_playback import write_tones
from check_pcm_conversion import build_worker, isolated_environment, run_bounded, snapshot, WORKER_TEST
from pcm_signal import read_capture, require, rms


def main():
    """Check mute, the perceptual midpoint and unity without modifying an OS mixer."""
    binary = build_worker()
    with tempfile.TemporaryDirectory(prefix="resolibre-volume-") as temporary:
        root = Path(temporary)
        for percent in (0, 50, 100):
            directory = root / str(percent)
            directory.mkdir(mode=0o700)
            source = directory / "private-input.wav"
            write_tones(source, 44100, 2)
            before = snapshot(source)
            environment = isolated_environment(directory, f"volume-{percent}")
            code, output, diagnostics = run_bounded(
                [binary, "--ignored", "--exact", WORKER_TEST, "--nocapture", "--test-threads=1"],
                directory, environment,
            )
            require(code == 0 and diagnostics == b"", "volume capture failed")
            require(snapshot(source) == before, "source changed")
            require(b"private" not in output + diagnostics, "private diagnostic leaked")
            require(not (directory / "private-script-ran").exists(), "external script executed")
            rate, samples = read_capture(directory / "capture.wav")
            require(rate == 44100, "unexpected output rate")
            left = samples[0::2]
            window = left[round(0.55 * rate):round(1.25 * rate)]
            measured = rms(window)
            expected = 0.08 / math.sqrt(2) * (percent / 100) ** 2
            print(f"volume check {percent}%: measured={measured:.8f}, expected={expected:.8f}", flush=True)
            require(abs(measured - expected) <= 0.00006, "incorrect software gain")
            if percent == 0:
                require(max(abs(value) for value in samples) <= 1, "mute leaked audio")
            print(f"PASS volume {percent}%: measured RMS={measured:.6f}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        message = str(error) if type(error) is ValueError else "volume capture infrastructure failed"
        print(f"FAIL: {message}", file=sys.stderr)
        sys.exit(1)
