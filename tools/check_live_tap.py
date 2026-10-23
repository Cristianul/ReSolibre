#!/usr/bin/env python3
"""Benchmark an experimental libmpv/LADSPA tap; never load it into the player."""

import argparse
import ctypes as native
import hashlib
import json
import math
import os
from pathlib import Path
import resource
import stat
import subprocess
import sys
import tempfile
import time
import wave

from check_pcm_conversion import ROOT, snapshot
from pcm_signal import require

BLOCK_SAMPLES = 2048
MAX_CHANNELS = 16


def open_fixture(path):
    """Open a bounded regular fixture read-only without following a final symlink."""
    descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    try:
        metadata = os.fstat(descriptor)
        require(stat.S_ISREG(metadata.st_mode), "fixture must be a regular file")
        require(0 < metadata.st_size <= 4 * 1024 * 1024, "fixture size is outside test bounds")
        return os.fdopen(descriptor, "rb")
    except BaseException:
        os.close(descriptor)
        raise


def reserve_capture(path):
    """Refuse existing output and require the harness's private, owned directory."""
    directory = path.parent.stat()
    require(directory.st_uid == os.getuid() and directory.st_mode & 0o077 == 0,
            "capture requires a private owned directory")
    with path.open("xb"):
        pass


class Event(native.Structure):
    """Borrow the public mpv_event structure until the next native event poll."""
    _fields_ = [("id", native.c_int), ("error", native.c_int),
                ("reply", native.c_uint64), ("data", native.c_void_p)]


class EndFile(native.Structure):
    """Read the initial reason/error fields of mpv_event_end_file."""
    _fields_ = [("reason", native.c_int), ("error", native.c_int)]


def bind(library, name, arguments, result):
    """Set each ABI explicitly before making native calls through ctypes."""
    function = getattr(library, name)
    function.argtypes = arguments
    function.restype = result
    return function


class Player:
    """Own a single native engine with restricted local input and a test-only sink."""

    def __init__(self, source, plugin, destination):
        """Configure the proof graph before initialization; clean up partial failure."""
        self.library = native.CDLL("libmpv.so")
        create = bind(self.library, "mpv_create", [], native.c_void_p)
        self.destroy = bind(self.library, "mpv_terminate_destroy", [native.c_void_p], None)
        self.option = bind(self.library, "mpv_set_option_string",
                           [native.c_void_p, native.c_char_p, native.c_char_p], native.c_int)
        initialize = bind(self.library, "mpv_initialize", [native.c_void_p], native.c_int)
        self.invoke = bind(self.library, "mpv_command",
                           [native.c_void_p, native.POINTER(native.c_char_p)], native.c_int)
        self.poll = bind(self.library, "mpv_wait_event",
                         [native.c_void_p, native.c_double], native.POINTER(Event))
        self.handle = create()
        require(bool(self.handle), "native allocation failed")
        self.source = None
        try:
            settings = {
                "config": "no", "load-scripts": "no", "ytdl": "no", "osc": "no",
                "terminal": "no", "msg-level": "all=no", "vid": "no", "sid": "no",
                "autoload-files": "no", "access-references": "no", "idle": "yes",
                "save-position-on-quit": "no", "resume-playback": "no",
                "demuxer": "lavf", "demuxer-lavf-probesize": "1048576",
                "demuxer-lavf-analyzeduration": "2", "demuxer-max-bytes": "8MiB",
                "cache": "no", "cache-on-disk": "no", "demuxer-max-back-bytes": "0",
                "demuxer-lavf-o": "format_whitelist=wav,protocol_whitelist=none",
                "audio-pitch-correction": "no", "ao": "pcm" if destination else "null",
            }
            if destination:
                reserve_capture(destination)
                settings["ao-pcm-file"] = str(destination)
            if plugin:
                # Only the analysis branch negotiates Float32; the audible branch is untouched.
                require(not any(char in str(plugin) for char in "':;,[]\\"), "unsafe test plugin path")
                settings["af"] = f"lavfi=[asplit[out][tap];[tap]ladspa=file={plugin}:plugin=resolibre_probe,anullsink]"
            for key, value in settings.items():
                code = self.option(self.handle, key.encode(), value.encode())
                require(code >= 0, f"native test option rejected: {key}")
            require(initialize(self.handle) >= 0, "native initialization failed")
            self.source = open_fixture(source)
            self.command("loadfile", f"/proc/self/fd/{self.source.fileno()}")
        except BaseException:
            self.close()
            raise

    def command(self, *arguments):
        """Submit one owned command without invoking a shell or exposing native text."""
        encoded = [argument.encode() for argument in arguments]
        values = (native.c_char_p * (len(encoded) + 1))(*encoded, None)
        require(self.invoke(self.handle, values) >= 0, "native test command failed")

    def close(self):
        """Destroy the engine before releasing its source descriptor."""
        if self.handle:
            self.destroy(self.handle)
            self.handle = None
        if self.source:
            self.source.close()
            self.source = None


class Tap:
    """Consume bounded channel rings from the explicitly loaded proof library."""

    def __init__(self, path, channels):
        """Keep the library loaded until the native engine has completed teardown."""
        self.library = native.CDLL(str(path))
        self.read = bind(self.library, "resolibre_tap_read",
                         [native.c_uint, native.POINTER(native.c_float), native.c_ulong], native.c_ulong)
        self.dropped = bind(self.library, "resolibre_tap_dropped", [native.c_uint], native.c_uint)
        self.buffer = (native.c_float * BLOCK_SAMPLES)()
        self.counts = [0] * channels
        self.power = [0.0] * channels
        require(self.read(MAX_CHANNELS, self.buffer, BLOCK_SAMPLES) == 0, "invalid channel accepted")
        require(self.read(0, self.buffer, BLOCK_SAMPLES - 1) == 0, "short destination accepted")

    def drain(self):
        """Read at most one block per channel per poll, retaining only aggregate power."""
        for channel in range(len(self.counts)):
            count = self.read(channel, self.buffer, BLOCK_SAMPLES)
            require(count <= BLOCK_SAMPLES, "oversized tap block")
            self.counts[channel] += count
            self.power[channel] += sum(self.buffer[index] ** 2 for index in range(count))

    def result(self):
        """Report counts, power and drops without retaining or publishing private PCM."""
        return {
            "samples": self.counts,
            "rms": [math.sqrt(power / count) if count else 0.0
                    for count, power in zip(self.counts, self.power)],
            "dropped": [self.dropped(channel) for channel in range(len(self.counts))],
        }


def worker(arguments):
    """Run one generated source in a fresh process with bounded wall/file limits."""
    resource.setrlimit(resource.RLIMIT_FSIZE, (8 * 1024 * 1024, 8 * 1024 * 1024))
    tap = Tap(arguments.plugin, arguments.channels) if arguments.mode != "baseline" else None
    started = time.monotonic()
    cpu_started = time.process_time()
    player = Player(arguments.source, arguments.plugin if tap else None, arguments.capture)
    commands = []
    loaded = False
    pause_checked = False
    try:
        while time.monotonic() - started < 10:
            event = player.poll(player.handle, 0.005).contents
            require(event.id != 24, "native event overflow")
            if event.id == 8:
                loaded = True
            if event.id == 7:
                ended = native.cast(event.data, native.POINTER(EndFile)).contents
                require(ended.reason == 0 and ended.error == 0, "unexpected playback termination")
                break
            if tap and arguments.mode == "consume":
                tap.drain()
            if loaded and not arguments.capture and not pause_checked and time.monotonic() - started > 0.6:
                for value in ("yes", "no"):
                    before = time.monotonic()
                    player.command("set", "pause", value)
                    commands.append((time.monotonic() - before) * 1000)
                pause_checked = True
        else:
            raise ValueError("native tap playback deadline")
    finally:
        player.close()
    require(loaded, "no loaded event")
    if tap and arguments.mode == "consume":
        for _ in range(4):
            tap.drain()
    result = {
        "mode": arguments.mode,
        "elapsed_s": round(time.monotonic() - started, 4),
        "cpu_s": round(time.process_time() - cpu_started, 4),
        "peak_rss_mib": round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024, 3),
        "command_max_ms": round(max(commands, default=0), 3),
    }
    if tap:
        result.update(tap.result())
    print(json.dumps(result))


def write_fixture(path, channels, width):
    """Generate two seconds of signed PCM, preserving phase-inverted stereo channels."""
    samples = bytearray()
    amplitude = (1 << (width * 8 - 1)) // 4
    for frame in range(48000 * 2):
        value = round(amplitude * math.sin(2 * math.pi * 440 * frame / 48000))
        for channel in range(channels):
            signed = value if channel % 2 == 0 else -value
            samples.extend(signed.to_bytes(width, "little", signed=True))
    with wave.open(str(path), "wb") as output:
        output.setnchannels(channels)
        output.setsampwidth(width)
        output.setframerate(48000)
        output.writeframes(samples)
    path.chmod(0o444)


def run_case(directory, source, plugin, channels, mode, capture):
    """Bound a child run and read only its small JSON result; suppress native diagnostics."""
    command = [sys.executable, str(Path(__file__).resolve()), "--worker",
               "--source", str(source), "--plugin", str(plugin), "--channels", str(channels),
               "--mode", mode]
    destination = directory / f"{source.stem}-{mode}.wav" if capture else None
    if destination:
        command.extend(["--capture", str(destination)])
    environment = os.environ.copy()
    environment.pop("FFREPORT", None)
    result = subprocess.run(command, env=environment, capture_output=True, timeout=15)
    require(result.returncode == 0, f"tap worker failed ({mode}); rerun worker for diagnostics")
    require(len(result.stdout) < 8192 and not result.stderr, "unexpected worker diagnostics")
    metrics = json.loads(result.stdout)
    if destination:
        metrics["output_sha256"] = hashlib.sha256(destination.read_bytes()).hexdigest()
    return metrics


def main(arguments):
    """Compare original/output bytes, overflow behavior and repeated native resource samples."""
    require((arguments.ladspa_include / "ladspa.h").is_file(), "provide a LADSPA 1.1 SDK include directory")
    all_results = []
    with tempfile.TemporaryDirectory(prefix="resolibre-live-tap-") as temporary:
        directory = Path(temporary)
        plugin = directory / "tap.so"
        built = subprocess.run([
            "cc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-O2", "-fPIC", "-shared",
            "-I", str(arguments.ladspa_include), str(ROOT / "tools/fixtures/live-tap/tap.c"),
            "-o", str(plugin),
        ], capture_output=True, timeout=30)
        require(built.returncode == 0, "test tap compilation failed")
        for channels, width in ((1, 2), (2, 2), (6, 3)):
            source = directory / f"pcm-{channels}ch-{width * 8}bit.wav"
            write_fixture(source, channels, width)
            original = snapshot(source)
            cases = [run_case(directory, source, plugin, channels, mode, True)
                     for mode in ("baseline", "consume", "stalled")]
            require(len({case["output_sha256"] for case in cases}) == 1, "analysis changed captured playback")
            require(all(count > 0 for count in cases[1]["samples"]), "missing tapped channel")
            require(all(0.17 < rms < 0.185 for rms in cases[1]["rms"]), "channel power lost or changed")
            require(all(count > 0 for count in cases[2]["dropped"]), "stalled consumer did not overflow")
            require(snapshot(source) == original, "source changed")
            all_results.append({"source": source.stem, "capture": cases})
        source = directory / "pcm-2ch-16bit.wav"
        timings = []
        for _ in range(arguments.trials):
            for mode in ("baseline", "consume", "stalled"):
                timings.append(run_case(directory, source, plugin, 2, mode, False))
        all_results.append({"timed_null_output": timings})
    report = {"passed": True, "scope": "isolated Linux proof, not production or physical-output evidence",
              "results": all_results}
    if arguments.report:
        arguments.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


def parse_arguments():
    """Accept explicit test-only paths and cap the number of benchmark repetitions."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ladspa-include", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--trials", type=int, choices=range(1, 6), default=3)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--source", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--plugin", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--capture", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--channels", type=int, choices=range(1, MAX_CHANNELS + 1), help=argparse.SUPPRESS)
    parser.add_argument("--mode", choices=("baseline", "consume", "stalled"), help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.worker:
        if not all((arguments.source, arguments.plugin, arguments.channels, arguments.mode)):
            parser.error("worker requires source, plugin, channels and mode")
    elif not arguments.ladspa_include:
        parser.error("--ladspa-include is required")
    return arguments


if __name__ == "__main__":
    args = parse_arguments()
    try:
        if args.worker:
            worker(args)
        else:
            main(args)
    except ValueError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
    except (OSError, subprocess.SubprocessError) as error:
        print(f"FAIL: test environment failure ({type(error).__name__})", file=sys.stderr)
        sys.exit(1)
