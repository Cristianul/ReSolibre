#!/usr/bin/env python3
"""Check CLI privacy and input isolation using disposable, owned local fixtures."""

from pathlib import Path
import hashlib
import os
import re
import socket
import subprocess
import sys
import tempfile
import wave


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/examples/local-play"


def run_player(arguments, environment):
    """Run the built example with a process deadline and capture private output."""
    return subprocess.run(
        [str(BINARY), *map(str, arguments)],
        env=environment,
        capture_output=True,
        timeout=10,
        check=False,
    )


def check_rejection(arguments, environment):
    """Require a visible failure without leaking fixture paths or native content."""
    result = run_player(arguments, environment)
    output = result.stdout + result.stderr
    assert result.returncode != 0, "invalid input accepted"
    assert b"private" not in output, "private input leaked into diagnostics"
    assert b"loaded" not in output, "invalid input reached loaded state"


def check_hostile_inputs(directory, environment):
    """Reject invalid inputs and disguised playlists/manifests without network access."""
    check_rejection([], environment)
    check_rejection([directory, directory], environment)
    check_rejection([directory / "private-missing.wav"], environment)
    check_rejection(["https://example.invalid/private-token"], environment)

    fifo = directory / "private-pipe.wav"
    os.mkfifo(fifo, 0o600)
    check_rejection([fifo], environment)

    # A loopback listener detects accidental URL following without external traffic.
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(0.2)
        port = listener.getsockname()[1]
        url = f"http://127.0.0.1:{port}/private-token"
        disguised = [
            ("wav", f"#EXTM3U\n{url}\n"),
            ("mp4", f"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\n{url}\n"),
            ("mkv", f"ffconcat version 1.0\nfile '{url}'\n"),
        ]
        for extension, contents in disguised:
            playlist = directory / f"private-disguised.{extension}"
            playlist.write_text(contents)
            before = playlist.read_bytes()
            check_rejection([playlist], environment)
            assert playlist.read_bytes() == before, "rejected input changed"

        try:
            connection, _ = listener.accept()
        except TimeoutError:
            pass
        else:
            connection.close()
            raise AssertionError("content-triggered network access")


def check_device_playback(directory, environment):
    """Play generated silence through the real default output and check preservation."""
    path = directory / "private ü local.wav"
    with wave.open(str(path), "wb") as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(48000)
        audio.writeframes(b"\x00\x00" * 48000)
    path.chmod(0o444)
    before_hash = hashlib.sha256(path.read_bytes()).digest()
    before_mtime = path.stat().st_mtime_ns

    result = run_player([path], environment)
    assert result.returncode == 0, "default-device playback failed"
    assert b"info: local audio loaded\n" in result.stdout
    assert b"info: source-decoded: 48000 Hz, 1 channels, Signed16\n" in result.stdout
    assert re.search(
        rb"info: output-audio-api: [1-9][0-9]* Hz, [1-9][0-9]* channels, \w+\n",
        result.stdout,
    )
    assert b"warn:" not in result.stdout
    assert result.stdout.endswith(b"info: playback ended\n")
    assert b"private" not in result.stdout
    assert result.stderr == b"", "unexpected native diagnostics"
    assert hashlib.sha256(path.read_bytes()).digest() == before_hash
    assert path.stat().st_mtime_ns == before_mtime
    assert path.stat().st_mode & 0o777 == 0o444


def main():
    """Exercise a real device only when requested; keep fixtures outside music."""
    if sys.argv[1:] not in ([], ["--device"]):
        print("Usage: python3 tools/check_local_playback.py [--device]", file=sys.stderr)
        return 1
    if not BINARY.is_file():
        print("Build first: cargo build -p resolibre-audio --example local-play", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory(prefix="resolibre-cli-") as temporary:
        directory = Path(temporary)
        config = directory / "private-config"
        scripts = config / "scripts"
        scripts.mkdir(parents=True)
        marker = directory / "private-script-ran"
        # If config is inherited, this invalid option breaks the valid WAV journey.
        (config / "mpv.conf").write_text("private-invalid-option=yes\n")
        (scripts / "private.lua").write_text(
            'local file = io.open(os.getenv("RESOUL_TEST_MARKER"), "w")\n'
            'if file then file:write("unexpected script"); file:close() end\n'
        )
        environment = os.environ.copy()
        environment["MPV_HOME"] = str(config)
        environment["RESOUL_TEST_MARKER"] = str(marker)

        check_hostile_inputs(directory, environment)
        if sys.argv[1:] == ["--device"]:
            check_device_playback(directory, environment)
        assert not marker.exists(), "external script executed"

    print("PASS: CLI rejection, privacy, FIFO and network-reference checks")
    if sys.argv[1:] == ["--device"]:
        print("PASS: default-device silence, config/script isolation and original preservation")
    return 0


if __name__ == "__main__":
    sys.exit(main())
