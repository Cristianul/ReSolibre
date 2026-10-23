#!/usr/bin/env python3
"""Verify local playback across private virtual audio-server failures, never hardware."""

from array import array
from pathlib import Path
import math
import re
import subprocess
import sys
import tempfile
import time
import wave

from check_pcm_conversion import build_worker, snapshot
from pcm_signal import MAX_CAPTURE_BYTES, require
from private_audio import PrivateAudio


WORKER_TEST = "output_failure_tests::play_fixture"


def write_fixture(path):
    """Create a six-second, quiet stereo tone; make it read-only before playback."""
    samples = array("h")
    for frame in range(6 * 48000):
        seconds = frame / 48000
        envelope = min(1.0, seconds / 0.02, (6 - seconds) / 0.02)
        for frequency in (440, 880):
            samples.append(round(2600 * envelope * math.sin(2 * math.pi * frequency * seconds)))
    if sys.byteorder != "little":
        samples.byteswap()
    with wave.open(str(path), "wb") as output:
        output.setnchannels(2)
        output.setsampwidth(2)
        output.setframerate(48000)
        output.writeframes(samples.tobytes())
    path.chmod(0o444)


def wait_for_samples(worker, monitor, destination, deadline):
    """Require a block of non-silent S16 monitor samples while the worker is alive."""
    offset = 0
    while time.monotonic() < deadline:
        require(worker.process.poll() is None, "playback ended before monitor evidence")
        require(monitor.process.poll() is None, "monitor exited before audio evidence")
        if destination.exists():
            require(destination.stat().st_size <= MAX_CAPTURE_BYTES, "monitor capture exceeded bound")
            with destination.open("rb") as capture:
                capture.seek(offset)
                block = capture.read(4096)
            # Read full stereo frames; don't lose a partial frame between polls.
            complete_bytes = len(block) // 4 * 4
            offset += complete_bytes
            samples = array("h", block[:complete_bytes])
            if sys.byteorder != "little":
                samples.byteswap()
            active = sum(abs(sample) > 100 for sample in samples)
            if active >= 128:
                return time.monotonic()
        time.sleep(0.01)
    raise ValueError("non-silent monitor evidence deadline exceeded")


def verify_worker(worker, failure, source, before, directory):
    """Check exact safe lifecycle reports, original preservation and config isolation."""
    output = worker.read(worker.stdout)
    diagnostics = worker.read(worker.stderr)
    require(worker.process.returncode == 0 and not diagnostics, "worker failed its lifecycle assertions")
    require(len(output) < 4096, "excessive worker output")
    # This worker prints a closed vocabulary. Native text, paths and metadata fail.
    test_prefix = f"test {WORKER_TEST} ... "
    normalized = output.decode("utf-8").replace(test_prefix, "")
    for line in normalized.splitlines():
        safe = (
            not line or line == "running 1 test"
            or line in ("output-loaded", "output-eof", "output-formats-cleared", "output-dropped")
            or re.fullmatch(r"output-error: playback failed \(libmpv code -[0-9]+\)", line)
            or line == "ok"
            or re.fullmatch(r"test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s", line)
        )
        require(safe, "unexpected or private worker diagnostic")
    require(output.count(b"output-dropped\n") == 1, "missing completed destruction")
    require(output.count(b"output-formats-cleared\n") == 1, "terminal formats remained stale")
    if failure:
        require(output.count(b"output-error:") == 1 and b"output-eof" not in output,
                "failure was missing or reported as EOF")
    else:
        require(output.count(b"output-eof\n") == 1 and b"output-error:" not in output,
                "successful playback did not reach EOF")
    require(snapshot(source) == before, "original fixture changed")
    require(not (directory / "private-script-ran").exists(), "external script executed")


def playback_case(audio, binary, name, failure=False, disconnect=False):
    """Observe one real player through EOF/error and Drop within bounded lifetime."""
    source = audio.directory / "private-input.wav"
    before = snapshot(source)
    monitor = None
    if not failure or disconnect:
        monitor, destination = audio.start_monitor(name)
    worker = audio.start([
        binary, "--ignored", "--exact", WORKER_TEST, "--nocapture", "--test-threads=1",
    ])
    fault_time = worker.started if failure else None
    deadline = worker.started + 12
    if monitor:
        audio.attach(worker, min(deadline, worker.started + 5))
        confirmed = wait_for_samples(worker, monitor, destination, worker.started + 5)
        if disconnect:
            require(confirmed - worker.started < 3, "fault would be too close to source EOF")
            fault_time = time.monotonic()
            require(fault_time - confirmed < 1, "fault missed confirmed-audio window")
            audio.stop_pulse()
    if failure:
        # This timeout fails the harness; it never creates an adapter error.
        while b"output-error:" not in worker.read(worker.stdout):
            require(time.monotonic() < fault_time + 3, "adapter failure response deadline exceeded")
            if worker.process.poll() is not None:
                require(b"output-error:" in worker.read(worker.stdout), "worker ended without a safe error")
                break
            time.sleep(0.01)
        response_ms = round((time.monotonic() - fault_time) * 1000)
        require(response_ms <= 3000, "adapter failure response deadline exceeded")
    worker.wait(deadline)
    verify_worker(worker, failure, source, before, audio.directory)
    if monitor:
        monitor.stop()
        require(destination.stat().st_size <= MAX_CAPTURE_BYTES, "monitor capture exceeded bound")
    if failure:
        print(f"PASS {name}: error within {response_ms} ms; no EOF; formats cleared; destroyed", flush=True)
    else:
        print(f"PASS {name}: monitor audio; EOF; formats cleared; destroyed", flush=True)


def main():
    """Repeat the ordered four-case journey, then reject malformed generated media."""
    require(not sys.argv[1:], "usage: python3 tools/check_output_failure.py")
    binary = build_worker()
    with tempfile.TemporaryDirectory(prefix="resolibre-private-output-") as temporary:
        root = Path(temporary)
        for repetition in range(1, 4):
            directory = root / str(repetition)
            directory.mkdir(mode=0o700)
            write_fixture(directory / "private-input.wav")
            with PrivateAudio(directory) as audio:
                audio.start_server()
                playback_case(audio, binary, f"{repetition}-baseline")
                audio.stop_pulse()
                playback_case(audio, binary, f"{repetition}-absent", failure=True)
                audio.start_pulse()
                playback_case(audio, binary, f"{repetition}-loss", failure=True, disconnect=True)
                # The failed worker was destroyed and reaped before restoring service.
                audio.start_pulse()
                playback_case(audio, binary, f"{repetition}-recovery")
        directory = root / "malformed"
        directory.mkdir(mode=0o700)
        source = directory / "private-input.wav"
        source.write_bytes(b"private-invalid-media")
        source.chmod(0o444)
        with PrivateAudio(directory) as audio:
            audio.start_server()
            playback_case(audio, binary, "malformed", failure=True)
    require(not root.exists(), "private runtime cleanup failed")
    print("PASS: three private-output journeys and malformed media; no physical-device claim")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.TimeoutExpired, KeyError) as error:
        # Only fixed validation messages are public; native/OS output stays private.
        message = str(error) if type(error) is ValueError else "private audio infrastructure failed"
        print(f"FAIL: {message}", file=sys.stderr)
        sys.exit(1)
