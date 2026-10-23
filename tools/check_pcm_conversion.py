#!/usr/bin/env python3
"""Capture generated audio through the test-only ReSolibre worker and inspect PCM."""

from pathlib import Path
import hashlib
import json
import os
import re
import resource
import subprocess
import sys
import tempfile

from check_audible_playback import write_tones
from pcm_signal import MAX_CAPTURE_BYTES, analyze_signal, read_capture, require


ROOT = Path(__file__).resolve().parents[1]
WORKER_TEST = "pcm_capture_tests::capture_fixture"


def limit_child_files():
    """Apply kernel file bounds before exec in this single-threaded Linux harness."""
    resource.setrlimit(resource.RLIMIT_FSIZE, (MAX_CAPTURE_BYTES, MAX_CAPTURE_BYTES))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def run_bounded(command, directory, environment, timeout=10):
    """Bound native lifetime and output; subprocess.run kills and reaps on timeout."""
    with tempfile.TemporaryFile(dir=directory) as stdout, tempfile.TemporaryFile(dir=directory) as stderr:
        try:
            result = subprocess.run(
                command, cwd=directory, env=environment, stdout=stdout, stderr=stderr,
                timeout=timeout, preexec_fn=limit_child_files, check=False,
            )
        except subprocess.TimeoutExpired:
            raise ValueError("capture process deadline exceeded") from None

        stdout.seek(0)
        stderr.seek(0)
        output = stdout.read(16385)
        diagnostics = stderr.read(16385)
        require(len(output) <= 16384 and len(diagnostics) <= 16384, "excessive child diagnostics")
        return result.returncode, output, diagnostics


def build_worker():
    """Build offline and select the exact test executable from Cargo's JSON output."""
    result = subprocess.run(
        ["cargo", "test", "-p", "resolibre-audio", "--lib", "--no-run", "--offline", "--message-format=json"],
        cwd=ROOT, capture_output=True, timeout=120, check=False,
    )
    require(result.returncode == 0, "capture worker build failed; run cargo test --no-run for diagnostics")
    executables = []
    for line in result.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") != "compiler-artifact":
            continue
        if item["target"]["name"] == "resolibre_audio" and item["profile"]["test"] and item.get("executable"):
            executables.append(item["executable"])
    require(len(executables) == 1, "missing or ambiguous capture worker")
    return executables[0]


def snapshot(path):
    """Record generated input bytes and metadata without exposing its private path."""
    metadata = path.stat()
    digest = hashlib.sha256(path.read_bytes()).digest()
    return digest, metadata.st_mtime_ns, metadata.st_mode & 0o777


def isolated_environment(directory, mode):
    """Install hostile external config/script fixtures that the adapter must ignore."""
    config = directory / "private-config"
    scripts = config / "scripts"
    scripts.mkdir(parents=True)
    (config / "mpv.conf").write_text("private-invalid-option=yes\n")
    (scripts / "private.lua").write_text(
        'local file = io.open("private-script-ran", "w")\n'
        'if file then file:write("unexpected"); file:close() end\n'
    )
    environment = os.environ.copy()
    environment["MPV_HOME"] = str(config)
    environment["RESOUL_PCM_CASE"] = mode
    environment["RUST_BACKTRACE"] = "0"
    return environment


def capture_case(binary, directory, source_rate, mode, failure=None):
    """Run one owned fixture and require preservation, privacy and expected lifecycle."""
    directory.mkdir(mode=0o700)
    source = directory / "private-input.wav"
    destination = directory / "capture.wav"
    if failure == "media":
        source.write_bytes(b"private-invalid-media")
        source.chmod(0o444)
    else:
        write_tones(source, source_rate, 2)
    before = snapshot(source)
    if failure == "destination":
        destination.mkdir()
    environment = isolated_environment(directory, mode)
    command = [binary, "--ignored", "--exact", WORKER_TEST, "--nocapture", "--test-threads=1"]
    code, output, diagnostics = run_bounded(command, directory, environment)

    require(snapshot(source) == before and before[2] == 0o444, "original fixture changed")
    require(b"private" not in output + diagnostics, "private diagnostic leaked")
    require(str(directory).encode() not in output + diagnostics, "fixture path leaked")
    require(not (directory / "private-script-ran").exists(), "external script executed")
    require(not (directory / "audiodump.wav").exists(), "implicit output destination")

    if failure:
        require(code != 0 and b"pcm-finalized" not in output, "invalid capture accepted")
        require(b"pcm-output:" not in output, "failed capture selected an output")
        require(re.search(rb'Error: Native \{ operation: "playback", code: -[0-9]+ \}', diagnostics),
                "missing native playback failure")
        if failure == "destination":
            require(destination.is_dir() and not list(destination.iterdir()), "failed destination changed")
        return None

    require(code == 0 and diagnostics == b"", "capture worker failed")
    expected_rate = source_rate if mode in ("source", "tap-source", "tap-stalled") else 48000
    require(f"pcm-source: {source_rate} 2 Signed16\n".encode() in output, "missing source report")
    require(f"pcm-output: {expected_rate} 2 Signed16\n".encode() in output, "missing output report")
    require(b"pcm-finalized\n" in output, "missing finalized EOF")
    rate, samples = read_capture(destination)
    return analyze_signal(rate, samples, expected_rate)


def main():
    """Run three complete matrices and native negatives, leaving no generated media."""
    require(not sys.argv[1:], "usage: python3 tools/check_pcm_conversion.py")
    binary = build_worker()
    with tempfile.TemporaryDirectory(prefix="resolibre-private-pcm-") as temporary:
        directory = Path(temporary)
        for repetition in range(1, 4):
            for name, source_rate, mode in (
                ("control", 48000, "rate"),
                ("conversion", 44100, "rate"),
                ("isolation", 44100, "source"),
            ):
                result = capture_case(binary, directory / f"{repetition}-{name}", source_rate, mode)
                print(f"PASS {repetition} {name}: {json.dumps(result, sort_keys=True)}", flush=True)
            session = directory / f"{repetition}-session-normal"
            capture_case(binary, session, 48000, "volume-100")
            control = directory / f"{repetition}-control" / "capture.wav"
            require((session / "capture.wav").read_bytes() == control.read_bytes(),
                    "normal session processing changed captured playback")
            baseline = directory / f"{repetition}-isolation" / "capture.wav"
            for mode in ("tap-source", "tap-stalled"):
                case = directory / f"{repetition}-{mode}"
                capture_case(binary, case, 44100, mode)
                require((case / "capture.wav").read_bytes() == baseline.read_bytes(),
                        "bundled live tap changed captured playback")
        capture_case(binary, directory / "malformed", 44100, "rate", failure="media")
        capture_case(binary, directory / "uncreatable", 44100, "bad-output", failure="destination")
    require(not directory.exists(), "capture directory cleanup failed")
    print("PASS: eighteen captures including normal session processing, bundled taps, native negatives and preservation")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        # OS/subprocess text can contain paths; only our fixed validation text is safe.
        message = str(error) if isinstance(error, ValueError) else "capture infrastructure failed"
        print(f"FAIL: {message}", file=sys.stderr)
        sys.exit(1)
