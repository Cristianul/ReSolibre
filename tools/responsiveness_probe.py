"""Owned command transport and streaming receipt-time measurements; no desktop audio."""

from collections import deque
import os
import re
import resource
import select
import socket
import time

from pcm_signal import require
from check_pcm_conversion import limit_child_files
from responsiveness_signal import BLOCK_FRAMES, RATE, MonitorBuffer, consecutive

WORKER_TEST = "responsiveness_tests::control_fixture"


def limit_analysis_files():
    """Allow one bounded detector WAV while keeping capture/monitor processes at their smaller limits."""
    resource.setrlimit(resource.RLIMIT_FSIZE, (24 * 1024 * 1024, 24 * 1024 * 1024))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def start_monitor(audio):
    """Stream bounded chunks through a pipe rather than growing a capture file."""
    monitor = audio.start([
        "parec", f"--server={audio.environment['PULSE_SERVER']}",
        "--device=resolibre-test.monitor", "--raw", "--format=s16le",
        "--rate=48000", "--channels=2", "--latency-msec=20", "--process-time-msec=20",
    ], pipe_stdout=True)
    os.set_blocking(monitor.process.stdout.fileno(), False)
    return monitor


class Probe:
    """One owning worker, a finite response table and a rolling PCM monitor window."""

    def __init__(self, audio, worker_command, monitor, history, lifetime=15):
        """Connect private datagrams before starting the worker; never overwrite sockets."""
        self.audio = audio
        self.monitor = monitor
        self.history = history
        self.session_worker = "RESOUL_SESSION_PROBE=1" in worker_command
        self.failure_events = 0
        self.sequence = 0
        self.acks = {}
        self.events = deque(maxlen=64)
        self.rejections = 0
        self.memory = None
        self.endpoint = audio.directory / "controller.sock"
        require(not self.endpoint.exists(), "controller endpoint exists")
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
        audio.stack.callback(self.close)
        self.socket.bind(str(self.endpoint))
        self.socket.setblocking(False)
        self.worker = audio.start([
            *worker_command, "--ignored", "--exact", WORKER_TEST, "--nocapture", "--test-threads=1",
        ], file_limit=limit_analysis_files if "RESOUL_PROBE_ANALYSIS=1" in worker_command else limit_child_files)
        self.deadline = self.worker.started + lifetime
        target = audio.directory / "worker.sock"
        while not target.exists():
            require(self.worker.process.poll() is None, "worker failed startup")
            require(time.monotonic() < self.worker.started + 5, "worker readiness deadline")
            time.sleep(0.002)
        self.socket.connect(str(target))

    def close(self):
        """Close only this controller; its directory remains owned by the parent."""
        self.socket.close()

    def pump(self, timeout=0.005, allow_monitor_exit=False):
        """Receive finite PCM/control chunks, preserving actual receipt timestamps."""
        require(time.monotonic() < self.deadline, "control trial deadline")
        if not allow_monitor_exit:
            require(self.monitor.process.poll() is None, "monitor terminated")
        ready, _, _ = select.select([self.socket, self.monitor.process.stdout], [], [], timeout)
        for stream in ready:
            if stream is self.socket:
                packet = self.socket.recv(257)
                require(len(packet) <= 256, "oversized worker response")
                text = packet.decode("ascii")
                if text == "reject":
                    self.rejections += 1
                elif re.fullmatch(r"ack [0-9]+ [0-9]+ [0-9]+ [01]", text):
                    _, number, entered, returned, success = text.split()
                    require(len(self.acks) < 64 and int(number) not in self.acks,
                            "response table overflow or duplicate")
                    self.acks[int(number)] = tuple(map(int, (entered, returned, success)))
                elif re.fullmatch(r"memory [0-9]+ [0-9]+ [0-9]+ [0-9]+", text):
                    self.memory = dict(zip(("arena", "allocated", "free", "mapped"),
                                           (int(value) for value in text.split()[1:])))
                elif re.fullmatch(r"event (eof|error) [0-9]+", text):
                    require(len(self.events) < 64, "event queue overflow")
                    event = text.split()[1]
                    self.events.append(event)
                    if event == "error":
                        self.failure_events += 1
                else:
                    raise ValueError("unexpected worker response")
            else:
                data = os.read(stream.fileno(), 8192)
                received = time.monotonic_ns()
                require(data or allow_monitor_exit, "monitor ended unexpectedly")
                if data:
                    self.history.feed(data, received)

    def send(self, action):
        """Submit without waiting for native acceptance; full transport is a failure."""
        self.sequence += 1
        require(len(self.acks) < 64, "command queue bound")
        packet = f"{self.sequence} {action}".encode("ascii")
        require(len(packet) <= 64, "command exceeds bound")
        submitted = time.monotonic_ns()
        self.socket.send(packet)
        return self.sequence, submitted

    def acknowledge(self, request, success=True, allow_monitor_exit=False):
        """Match acceptance to submission and validate the cross-process clock order."""
        sequence, submitted = request
        while sequence not in self.acks:
            self.pump(allow_monitor_exit=allow_monitor_exit)
            require(sequence in self.acks or self.worker.process.poll() is None,
                    "worker ended without acknowledgement")
        entered, returned, accepted = self.acks.pop(sequence)
        require(submitted <= entered <= returned <= time.monotonic_ns(), "invalid clock correlation")
        require(bool(accepted) == success, "unexpected command outcome")
        return (entered - submitted) / 1e6, (returned - entered) / 1e6

    def observe(self, request, target=None, silence=False, tolerance=None):
        """Confirm two blocks; receipt is an upper bound including monitor buffering."""
        _, submitted = request
        checked = -1
        previous = None
        while True:
            self.pump()
            require(not self.events, "terminal event while awaiting output")
            for block in self.history.blocks:
                if block.index <= checked or block.received_ns < submitted:
                    continue
                checked = block.index
                if silence:
                    valid = block.silent and previous is not None and previous.silent
                else:
                    valid = previous is not None and consecutive(previous, block)
                    if valid and target is not None:
                        offset = previous.position - round(target * RATE)
                        valid = 0 <= offset <= tolerance
                if valid:
                    self.confirmed_block = block
                    return {
                        "upper_ms": (block.received_ns - submitted) / 1e6,
                        "position": previous.position,
                        "sample_index": previous.index,
                    }
                previous = block

    def hold(self, seconds, silent=False):
        """Require continuity or silence in every whole block after confirmation."""
        end = time.monotonic() + seconds
        checked = self.confirmed_block.index
        expected = self.confirmed_block.position
        require(silent or expected is not None, "missing confirmed source position")
        while time.monotonic() < end:
            self.pump()
            require(not self.events, "terminal event during observation")
            for block in self.history.blocks:
                if block.index <= checked:
                    continue
                checked = block.index
                if silent:
                    require(block.silent, "audio while paused")
                else:
                    expected += BLOCK_FRAMES
                    require(block.position == expected, "stale or discontinuous settled audio")
                self.confirmed_block = block

    def finish(self, allow_monitor_exit=False):
        """Acknowledge Drop/quit, reap, validate safe output and release socket paths."""
        self.acknowledge(self.send("quit"), allow_monitor_exit=allow_monitor_exit)
        require(self.worker.wait(self.deadline) == 0, "worker exited unsuccessfully")
        validate_diagnostics(self.worker.read(self.worker.stderr), self.session_worker, self.failure_events)
        output = self.worker.read(self.worker.stdout).decode("ascii")
        normalized = output.replace(f"test {WORKER_TEST} ... ", "")
        for line in normalized.splitlines():
            require(not line or line in ("running 1 test", "ok") or re.fullmatch(
                r"test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s",
                line), "unexpected worker diagnostic")
        self.close()
        self.endpoint.unlink()
        (self.audio.directory / "worker.sock").unlink()
        require(not (self.audio.directory / "private-script-ran").exists(), "external script executed")


def read_usage(pid):
    """Read Linux process CPU ticks and resident pages without returning private text."""
    with open(f"/proc/{pid}/stat") as source:
        text = source.read(8192)
    fields = text[text.rfind(")") + 2:].split()
    ticks = int(fields[11]) + int(fields[12])
    rss = int(fields[21]) * os.sysconf("SC_PAGE_SIZE") / (1024 * 1024)
    return ticks / os.sysconf("SC_CLK_TCK"), rss


def validate_diagnostics(diagnostics, session_worker, failure_events):
    """Permit exactly one safe handled error per observed production-session failure."""
    if not session_worker or failure_events == 0:
        require(not diagnostics, "worker diagnostics present: " + diagnostic_category(diagnostics))
        return
    lines = diagnostics.splitlines()
    require(len(lines) == failure_events, "missing or repeated session failure log: " + diagnostic_category(diagnostics))
    for line in lines:
        require(re.fullmatch(rb"error: (playback|state|timing|seek|pause|volume) failed \(libmpv code -[0-9]+\)", line),
                "unsafe session failure diagnostic")


def diagnostic_category(diagnostics):
    """Name only closed warning categories; never echo native/private diagnostics on a failed probe."""
    lines = diagnostics.splitlines()
    if b"warn: rhythm excerpt unavailable" in lines:
        return "rhythm excerpt warning"
    if b"warn: Live spectrum capacity unavailable" in lines:
        return "live spectrum capacity warning"
    known = {
        b"warn: current audio information unavailable": "stream information warning",
        b"warn: some track metadata unavailable; using display fallbacks": "metadata warning",
        b"warn: Live spectrum clock unavailable": "live clock warning",
        b"warn: Live spectrum unavailable": "live spectrum warning",
        b"warn: Live spectrum format unavailable": "live format warning",
        b"warn: Live spectrum filter unavailable": "live filter warning",
        b"warn: Invalid live spectrum samples": "live samples warning",
        b"warn: Invalid live spectrum data": "live transport warning",
    }
    for line, category in known.items():
        if line in lines:
            return category
    for needle, category in ((b"broken pipe", "native pipe shutdown"), (b"alsa", "native ALSA diagnostic"), (b"invalid frequency", "native EQ frequency diagnostic")):
        if needle in diagnostics.lower():
            return category
    if re.search(rb"error: (playback|state|timing|seek|pause|volume) failed \(libmpv code -[0-9]+\)", diagnostics):
        return "uncorrelated transport failure"
    return "unclassified"
