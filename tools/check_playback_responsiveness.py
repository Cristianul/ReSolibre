#!/usr/bin/env python3
"""Measure local controls at an isolated virtual monitor, with receipt upper bounds."""

from collections import defaultdict
from pathlib import Path
import argparse
import hashlib
import json
import os
import select
import subprocess
import sys
import tempfile
import time

from check_pcm_conversion import build_worker, limit_child_files
from pcm_signal import require
from private_audio import PrivateAudio
from responsiveness_probe import Probe, read_usage, start_monitor
from responsiveness_signal import (
    BLOCK_FRAMES, MAX_SOURCE_BYTES, POSITION_TOLERANCE, RATE,
    MonitorBuffer, check_reference_budgets, summarize, write_markers,
)


def fingerprint(path):
    """Hash a bounded source incrementally and preserve modification metadata."""
    metadata = path.stat()
    require(metadata.st_size <= MAX_SOURCE_BYTES, "source exceeds bound")
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(65536):
            digest.update(block)
    return digest.digest(), metadata.st_mtime_ns, metadata.st_mode & 0o777


def encode_flac(source, destination, directory):
    """Encode only the generated fixture with bounded offline FFmpeg output."""
    require(not destination.exists(), "FLAC destination exists")
    with (directory / "encode.log").open("xb+") as log, destination.open("xb") as output:
        process = subprocess.Popen([
            "ffmpeg", "-nostdin", "-v", "error", "-protocol_whitelist", "file,pipe",
            "-i", str(source), "-map_metadata", "-1", "-c:a", "flac", "-f", "flac", "pipe:1",
        ], stdout=subprocess.PIPE, stderr=log, preexec_fn=limit_child_files)
        deadline = time.monotonic() + 30
        total = 0
        try:
            while True:
                require(time.monotonic() < deadline, "FLAC encoding deadline")
                ready, _, _ = select.select([process.stdout], [], [], 0.1)
                if not ready:
                    continue
                block = os.read(process.stdout.fileno(), 65536)
                if not block:
                    break
                total += len(block)
                require(total <= MAX_SOURCE_BYTES, "FLAC source exceeds bound")
                output.write(block)
            require(process.wait(timeout=max(0.01, deadline - time.monotonic())) == 0,
                    "generated FLAC encoding failed")
        finally:
            if process.poll() is None:
                process.kill()
            process.wait()
            process.stdout.close()
    destination.chmod(0o444)


def record_control(probe, results, name, action, target=None, silent=False):
    """Measure dispatch/native duration and confirmed signal for one submitted command."""
    request = probe.send(action)
    dispatch, native = probe.acknowledge(request)
    observation = probe.observe(request, target, silent, POSITION_TOLERANCE)
    results[name].append(observation["upper_ms"])
    results["dispatch_ms"].append(dispatch)
    results["native_ms"].append(native)
    if target is not None:
        results["seek_position_error_ms"].append((observation["position"] / RATE - target) * 1000)
    return observation


def calibration(probe, results):
    """Verify source/sample continuity and report receipt jitter, without subtracting lag."""
    probe.hold(0.4)
    blocks = list(probe.history.blocks)[-30:]
    require(len(blocks) == 30, "insufficient calibration blocks")
    for first, second in zip(blocks, blocks[1:]):
        require(first.position is not None and second.position == first.position + BLOCK_FRAMES,
                "calibration source timeline mismatch")
    sample_ms = (blocks[-1].index - blocks[0].index) * 1000 / RATE
    received_ms = (blocks[-1].received_ns - blocks[0].received_ns) / 1e6
    results["calibration_span_error_ms"].append(abs(received_ms - sample_ms))
    results["calibration_max_delivery_gap_ms"].append(max(
        (second.received_ns - first.received_ns) / 1e6
        for first, second in zip(blocks, blocks[1:])
    ))


def trial(audio, binary, monitor, history, linked, results, index):
    """Exercise one fresh player through controls, bursts, paused seek and EOF."""
    probe = Probe(audio, binary, monitor, history)
    request = probe.send("open")
    dispatch, native = probe.acknowledge(request)
    attached = time.monotonic_ns()
    audio.attach(probe.worker, min(probe.deadline, time.monotonic() + 5), linked)
    results["attachment_ms"].append((time.monotonic_ns() - attached) / 1e6)
    started = probe.observe(request, 0.0, tolerance=RATE)
    results["play_upper_ms"].append(started["upper_ms"])
    startup_group = "first_stream_upper_ms" if index == 0 else "subsequent_stream_upper_ms"
    results[startup_group].append(started["upper_ms"])
    results["open_native_ms"].append(native)
    results["dispatch_ms"].append(dispatch)
    if index == 0:
        calibration(probe, results)
    record_control(probe, results, "pause_upper_ms", "pause", silent=True)
    # Last whole active block bounds the continuation; partial transition is 10 ms.
    active = [block.position for block in history.blocks if block.position is not None]
    require(active, "missing pre-pause position")
    last_position = active[-1]
    probe.hold(0.04, silent=True)
    resumed = record_control(probe, results, "resume_upper_ms", "resume")
    continuation_error = resumed["position"] - (last_position + BLOCK_FRAMES)
    require(abs(continuation_error) <= POSITION_TOLERANCE, "resume skipped source position")
    results["resume_position_error_ms"].append(abs(continuation_error) * 1000 / RATE)
    for target in (103.0, 4.0, 0.0, 111.0):
        record_control(probe, results, "seek_upper_ms", f"seek {target}", target)
        probe.hold(0.03)
    record_control(probe, results, "pause_upper_ms", "pause", silent=True)
    probe.acknowledge(probe.send("seek 60"))
    probe.hold(0.04, silent=True)
    record_control(probe, results, "paused_seek_resume_upper_ms", "resume", 60.0)

    requests = []
    start = time.monotonic()
    for number, target in enumerate((8, 90, 2, 103, 20, 4, 100, 70)):
        due = start + number * 0.005
        while time.monotonic() < due:
            probe.pump(min(0.002, max(0, due - time.monotonic())))
        requests.append(probe.send(f"seek {target}"))
        results["burst_submission_lateness_ms"].append(max(0, time.monotonic() - due) * 1000)
    for request in requests:
        dispatch, native = probe.acknowledge(request)
        results["burst_dispatch_ms"].append(dispatch)
        results["native_ms"].append(native)
    settled = probe.observe(requests[-1], 70.0, tolerance=POSITION_TOLERANCE)
    results["burst_upper_ms"].append(settled["upper_ms"])
    results["seek_position_error_ms"].append((settled["position"] / RATE - 70) * 1000)
    probe.hold(0.12)
    results["settled_rss_mib"].append(read_usage(probe.worker.process.pid)[1])

    if index == 0:
        probe.acknowledge(probe.send("seek 111.9"))
        while not probe.events:
            probe.pump()
        require(list(probe.events) == ["eof"], "near-EOF journey did not end normally")
        probe.events.clear()
        probe.acknowledge(probe.send("pause"), success=False)
    probe.finish()


def child_rss(pid):
    """Sum only children of the known worker's threads; short-lived decoders may exit during sampling."""
    children = set()
    for entry in Path(f"/proc/{pid}/task").glob("*/children"):
        try:
            children.update(int(value) for value in entry.read_text().split())
        except FileNotFoundError:
            pass
    total = 0.0
    for child in children:
        try:
            total += read_usage(child)[1]
        except (FileNotFoundError, ProcessLookupError):
            pass
    return total


def sample_resources(probe, processes, phase):
    """Sample one five-second workload, accounting for each owned process separately."""
    initial = {name: read_usage(pid)[0] for name, pid in processes.items()}
    samples = defaultdict(list)
    combined_rss = []
    started = time.monotonic()
    next_sample = started
    next_seek = started
    count = 0
    while time.monotonic() - started < 5:
        if phase == "controls" and time.monotonic() >= next_seek:
            target = 4 if count % 2 else 103
            probe.acknowledge(probe.send(f"seek {target}"))
            next_seek += 0.2
            count += 1
        probe.pump()
        require(not probe.events, "unexpected resource-phase termination")
        if time.monotonic() >= next_sample:
            for name, pid in processes.items():
                samples[name].append(read_usage(pid)[1])
            combined_rss.append(samples["player"][-1] + child_rss(processes["player"]))
            next_sample += 0.1

    elapsed = time.monotonic() - started
    result = {}
    for name, pid in processes.items():
        cpu_seconds = read_usage(pid)[0] - initial[name]
        result[name] = {
            "cpu_percent_one_core": round(100 * cpu_seconds / elapsed, 2),
            "sampled_peak_rss_mib": round(max(samples[name]), 2),
            "settled_rss_mib": round(samples[name][-1], 2),
            "samples": len(samples[name]),
        }
    result["player"]["with_owned_children_peak_rss_mib"] = round(max(combined_rss), 2)
    return result


def measure_cycles(probe, count):
    """Sample active/post-Drop RSS over same-process lifecycles, plus allocator counters."""
    cycles = []
    active = []
    allocator = []
    for _ in range(count):
        probe.acknowledge(probe.send("drop"))
        probe.acknowledge(probe.send("open"))
        probe.audio.attach(probe.worker, min(probe.deadline, time.monotonic() + 5), True)
        probe.observe((0, time.monotonic_ns()), 0.0, tolerance=RATE)
        after_open = read_usage(probe.worker.process.pid)[1]
        probe.acknowledge(probe.send("seek 103"))
        probe.observe((0, time.monotonic_ns()), 103, tolerance=RATE)
        after_seek = read_usage(probe.worker.process.pid)[1]
        active.append(round(max(after_open, after_seek), 2))
        probe.acknowledge(probe.send("drop"))
        end = time.monotonic() + 0.1
        while time.monotonic() < end:
            probe.pump()
        cycles.append(round(read_usage(probe.worker.process.pid)[1], 2))
        probe.acknowledge(probe.send("memory"))
        allocator.append(probe.memory)
    return cycles, active, allocator


def measure_resources(audio, binary, monitor, history, diagnostic=False):
    """Run bounded workloads and lifecycle cycles; trim only in explicit diagnostics."""
    probe = Probe(audio, binary, monitor, history, lifetime=90)
    processes = {
        "player": probe.worker.process.pid, "server": audio.server.process.pid,
        "pulse": audio.pulse.process.pid, "monitor": monitor.process.pid,
        "harness": os.getpid(),
    }
    probe.acknowledge(probe.send("open"))
    audio.attach(probe.worker, min(probe.deadline, time.monotonic() + 5), True)
    probe.observe((0, time.monotonic_ns()), 0.0, tolerance=RATE)
    result = {}
    for phase in ("playback", "controls"):
        result[phase] = sample_resources(probe, processes, phase)

    cycles, active, allocator = measure_cycles(probe, 60)
    result["post_drop_rss_mib"] = cycles
    result["cycle_active_rss_mib"] = active
    result["allocator_bytes"] = allocator
    if diagnostic:
        probe.acknowledge(probe.send("trim"))
        result["diagnostic_trim_rss_mib"] = read_usage(probe.worker.process.pid)[1]
        probe.acknowledge(probe.send("memory"))
        result["diagnostic_trim_allocator"] = probe.memory
    probe.finish()
    return result


def protocol_negatives(audio, binary, monitor, history):
    """Reject malformed/replayed packets without accepting them or poisoning a player."""
    probe = Probe(audio, binary, monitor, history)
    for packet in (b"1 seek NaN", b"1 seek -1", b"1 seek inf", b"1 pause extra", b"x" * 4096):
        before = probe.rejections
        probe.socket.send(packet)
        while probe.rejections == before:
            probe.pump()
    request = probe.send("drop")
    probe.acknowledge(request)
    before = probe.rejections
    probe.socket.send(f"{request[0]} open".encode())
    while probe.rejections == before:
        probe.pump()
    probe.finish()


def loss_case(audio, binary, monitor, history):
    """Disconnect while a command is outstanding and demand failure instead of EOF."""
    probe = Probe(audio, binary, monitor, history)
    probe.acknowledge(probe.send("open"))
    audio.attach(probe.worker, min(probe.deadline, time.monotonic() + 5), True)
    probe.observe((0, time.monotonic_ns()), 0.0, tolerance=RATE)
    probe.send("seek 103")
    audio.stop_pulse()
    while not probe.events:
        probe.pump(allow_monitor_exit=True)
    require(list(probe.events) == ["error"], "output loss became EOF or success")
    probe.finish(allow_monitor_exit=True)


def absent_case(audio, binary, monitor, history):
    """Require safe failure when opening against the deliberately stopped endpoint."""
    probe = Probe(audio, binary, monitor, history)
    probe.acknowledge(probe.send("open"), allow_monitor_exit=True)
    while not probe.events:
        probe.pump(allow_monitor_exit=True)
    require(list(probe.events) == ["error"], "absent output did not fail")
    probe.finish(allow_monitor_exit=True)


def native_negatives(root, binary, source):
    """Malformed media must fail safely; a dead monitor must fail the measurement."""
    for case in ("malformed", "dead-monitor"):
        directory = root / case
        directory.mkdir(mode=0o700)
        fixture = directory / "private-input.wav"
        if case == "malformed":
            fixture.write_bytes(b"invalid-generated-media")
            fixture.chmod(0o444)
        else:
            os.link(source, fixture)
        before = fingerprint(fixture)
        with PrivateAudio(directory) as audio:
            audio.start_server()
            monitor = start_monitor(audio)
            probe = Probe(audio, binary, monitor, MonitorBuffer())
            probe.acknowledge(probe.send("open"))
            if case == "malformed":
                while not probe.events:
                    probe.pump()
                require(list(probe.events) == ["error"], "malformed media did not fail")
                probe.finish()
            else:
                audio.attach(probe.worker, min(probe.deadline, time.monotonic() + 5))
                probe.observe((0, time.monotonic_ns()), 0.0, tolerance=RATE)
                monitor.stop()
                try:
                    probe.pump()
                except ValueError as error:
                    require(str(error) == "monitor terminated", "unexpected monitor failure")
                else:
                    raise ValueError("dead monitor accepted")
                probe.finish(allow_monitor_exit=True)
        require(fingerprint(fixture) == before, "negative case changed source")
    print("PASS malformed input and dead-monitor rejection", flush=True)


def main():
    """Run three independent sessions per codec and print only bounded numeric evidence."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trials", type=int, default=100, help="total per codec, 3..100; default 100")
    parser.add_argument("--check-reference-budgets", action="store_true", help="check proposed latency/CPU limits and the 100 MiB memory objective")
    parser.add_argument("--allocator-diagnostic", action="store_true", help="60 cycles and test-only trim; not acceptance")
    parser.add_argument("--system-allocator", action="store_true", help="compare untuned worker; not application acceptance")
    parser.add_argument("--skip-resources", action="store_true", help="probe only; not full acceptance")
    parser.add_argument("--track-analysis", action="store_true", help="start on-demand original analysis during each session playback; includes decoder RSS sampling")
    parser.add_argument("--analysis-cache", action="store_true", help="enable persistent analysis in the private test profile")
    parser.add_argument("--session-worker", action="store_true", help="measure the production session instead of direct adapter calls")
    arguments = parser.parse_args()
    require(not arguments.track_analysis or arguments.session_worker, "track analysis needs the session worker")
    require(not arguments.analysis_cache or arguments.session_worker, "analysis cache needs the session worker")
    require(3 <= arguments.trials <= 100, "trials must be between 3 and 100")
    require(not (arguments.allocator_diagnostic and arguments.skip_resources),
            "allocator diagnostics require resource measurements")
    if arguments.check_reference_budgets:
        require(arguments.trials == 100 and not arguments.skip_resources
                and not arguments.allocator_diagnostic and not arguments.system_allocator,
                "reference budgets require the complete default workload")
    require(not (arguments.session_worker and arguments.allocator_diagnostic),
            "session worker has no trim diagnostic")
    binary = [str(build_worker())]
    if arguments.session_worker:
        binary = ["env", "RESOUL_SESSION_PROBE=1", *binary]
    if arguments.analysis_cache:
        binary = ["env", "RESOUL_PROBE_CACHE=1", *binary]
    if arguments.track_analysis:
        binary = ["env", "RESOUL_PROBE_ANALYSIS=1", *binary]
    if not arguments.system_allocator:
        # libtest has already created a thread when the ignored fixture runs.
        # Apply the app's fixed 128 KiB threshold at exec, never call MT-Unsafe
        # mallopt from that thread. env exec preserves the verified worker PID.
        # Both glibc spellings prevent an inherited threshold from overriding it.
        binary = ["env", "MALLOC_MMAP_THRESHOLD_=131072",
                  "GLIBC_TUNABLES=glibc.malloc.mmap_threshold=131072", *binary]
    all_results = {"analysis_cache": arguments.analysis_cache, "worker": "session" if arguments.session_worker else "direct", "allocator_policy": "system" if arguments.system_allocator else "application-128k"}
    with tempfile.TemporaryDirectory(prefix="resolibre-response-") as temporary:
        root = Path(temporary)
        wav = root / "source.wav"
        flac = root / "source.flac"
        write_markers(wav)
        encode_flac(wav, flac, root)
        for codec, source in (("wav", wav), ("flac", flac)):
            if arguments.allocator_diagnostic and codec == "flac":
                continue
            results = defaultdict(list)
            resources = []
            attempts = 0
            for session in range(1 if arguments.allocator_diagnostic else 3):
                directory = root / f"{codec}-{session}"
                directory.mkdir(mode=0o700)
                os.link(source, directory / "private-input.wav")
                before = fingerprint(source)
                with PrivateAudio(directory) as audio:
                    audio.start_server()
                    monitor = start_monitor(audio)
                    history = MonitorBuffer()
                    count = arguments.trials // 3 + int(session < arguments.trials % 3)
                    for index in range(count):
                        attempts += 1
                        try:
                            trial(audio, binary, monitor, history, index > 0, results, index)
                            require(fingerprint(source) == before, "source changed")
                        except (ValueError, OSError, subprocess.TimeoutExpired):
                            print(f"FAIL {codec} session {session + 1}: attempt {index + 1}/{count}; no retries", flush=True)
                            raise
                    protocol_negatives(audio, binary, monitor, history)
                    if not arguments.skip_resources:
                        resources.append(measure_resources(audio, binary, monitor, history, arguments.allocator_diagnostic))
                    loss_case(audio, binary, monitor, history)
                    absent_case(audio, binary, monitor, history)
                require(fingerprint(source) == before, "source changed after failure")
                print(f"PASS {codec} session {session + 1}: {count} trials; protocol and pending-command loss", flush=True)
            all_results[codec] = {"measurements": {name: summarize(values) for name, values in results.items()},
                                  "resources": resources, "failures": 0, "attempts": attempts}
        native_negatives(root, binary, wav)
    print(json.dumps(all_results, sort_keys=True), flush=True)
    if arguments.check_reference_budgets:
        check_reference_budgets(all_results)
        print("PASS: reference budgets", flush=True)
    print("PASS: virtual monitor receipt upper bounds only; no physical latency claim")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        message = str(error) if type(error) is ValueError else "responsiveness infrastructure failed"
        print(f"FAIL: {message}", file=sys.stderr)
        sys.exit(1)
