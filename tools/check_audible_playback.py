#!/usr/bin/env python3
"""Play owned quiet tones; human listening remains separate from engine evidence."""

from pathlib import Path
import hashlib
import math
import os
import re
import struct
import sys
import tempfile
import wave

from check_local_playback import BINARY, run_player


def write_tones(path, rate, channels):
    """Generate three seconds of PCM16 with 20 ms fades and an 8% peak amplitude."""
    frames = bytearray()
    for frame in range(rate * 3):
        seconds = frame / rate
        # First burst: 0.5–1.3 s, second: 1.7–2.5 s; silence separates them.
        for channel in range(channels):
            sample = 0.0
            for burst, start in enumerate((0.5, 1.7)):
                elapsed = seconds - start
                active_channel = channels == 1 or channel == burst
                if active_channel and 0.0 <= elapsed < 0.8:
                    envelope = min(1.0, elapsed / 0.02, (0.8 - elapsed) / 0.02)
                    frequency = 440 if burst == 0 else 880
                    sample = 0.08 * envelope * math.sin(2 * math.pi * frequency * elapsed)
            frames.extend(struct.pack("<h", round(sample * 32767)))

    with wave.open(str(path), "wb") as audio:
        audio.setnchannels(channels)
        audio.setsampwidth(2)
        audio.setframerate(rate)
        audio.writeframes(frames)
    path.chmod(0o444)


def check_tones(directory, rate, channels):
    """Check engine formats, EOF and original preservation; print no private paths."""
    path = directory / "private-tone.wav"
    write_tones(path, rate, channels)
    before = path.stat()
    digest = hashlib.sha256(path.read_bytes()).digest()
    print(f"Playing {rate} Hz / {channels} channels: 440 Hz then 880 Hz", flush=True)
    result = run_player([path], os.environ.copy())
    assert result.returncode == 0, "tone playback failed"
    assert result.stderr == b"", "unexpected diagnostics"
    assert b"private" not in result.stdout, "private path leaked"
    expected = f"info: source-decoded: {rate} Hz, {channels} channels, Signed16\n"
    assert expected.encode() in result.stdout, "incorrect source format"
    assert re.search(
        rb"info: output-audio-api: [1-9][0-9]* Hz, [1-9][0-9]* channels, \w+\n",
        result.stdout,
    ), "missing output format"
    assert b"warn:" not in result.stdout, "incomplete format report"
    assert result.stdout.endswith(b"info: playback ended\n"), "missing EOF"
    assert hashlib.sha256(path.read_bytes()).digest() == digest, "original bytes changed"
    assert path.stat().st_mtime_ns == before.st_mtime_ns, "original mtime changed"
    assert path.stat().st_mode & 0o777 == 0o444, "original permissions changed"
    print(result.stdout.decode("utf-8"), end="")


def main():
    """Run explicit device checks without changing system volume or retaining audio."""
    if sys.argv[1:]:
        print("Usage: python3 tools/check_audible_playback.py", file=sys.stderr)
        return 1
    if not BINARY.is_file():
        print("Build first: cargo build -p resolibre-audio --example local-play", file=sys.stderr)
        return 1

    print("Two quiet bursts per sample; stereo: low tone left, high tone right.", flush=True)
    with tempfile.TemporaryDirectory(prefix="resolibre-listen-") as temporary:
        directory = Path(temporary)
        for rate, channels in ((44100, 1), (48000, 2)):
            case = directory / str(rate)
            case.mkdir()
            check_tones(case, rate, channels)

    print("PASS: engine format reports, EOF and original preservation")
    print("LISTENING: record what was heard separately; engine success is not audibility.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
