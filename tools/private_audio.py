"""Bounded, owned processes for the private audio-server regression fixture."""

from contextlib import ExitStack
import json
import os
import signal
import subprocess
import time

from check_pcm_conversion import ROOT, isolated_environment, limit_child_files
from pcm_signal import MAX_CAPTURE_BYTES, require


class Child:
    """Own one process group and its bounded files until cleanup completes."""

    def __init__(self, stack, command, directory, environment, name, pipe_stdout=False):
        """Own logs/process cleanup; optional stdout pipe requires bounded parent reads."""
        self.stdout = stack.enter_context((directory / f"{name}.out").open("xb+"))
        self.stderr = stack.enter_context((directory / f"{name}.err").open("xb+"))
        self.started = time.monotonic()
        self.stopped = False
        self.process = subprocess.Popen(
            command, cwd=directory, env=environment,
            stdout=subprocess.PIPE if pipe_stdout else self.stdout,
            stderr=self.stderr, stdin=subprocess.DEVNULL,
            start_new_session=True, preexec_fn=limit_child_files,
        )
        stack.callback(self.stop)
        if pipe_stdout:
            stack.callback(self.process.stdout.close)

    def read(self, stream):
        """Read bounded diagnostics without changing the child's shared file offset."""
        size = os.fstat(stream.fileno()).st_size
        require(size <= MAX_CAPTURE_BYTES, "child diagnostics exceeded file bound")
        return os.pread(stream.fileno(), size, 0)

    def wait(self, deadline):
        """Wait until an absolute deadline; cleanup is still owned by the stack."""
        remaining = deadline - time.monotonic()
        require(remaining > 0, "child process deadline exceeded")
        try:
            return self.process.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            raise ValueError("child process deadline exceeded") from None

    def signal_group(self, number):
        """Signal only the session created for this child, even after leader exit."""
        try:
            os.killpg(self.process.pid, number)
        except ProcessLookupError:
            pass

    def stop(self):
        """Terminate owned work, escalate after a short grace period and reap."""
        if self.stopped:
            return
        self.stopped = True
        self.signal_group(signal.SIGTERM)
        try:
            self.process.wait(timeout=0.3)
        except subprocess.TimeoutExpired:
            pass
        finally:
            # A descendant might outlive its group leader; kill the whole group.
            self.signal_group(signal.SIGKILL)
            self.process.wait()


class PrivateAudio:
    """Own a private PipeWire graph; never access the desktop's audio sockets."""

    def __init__(self, directory):
        """Prepare configuration and environment in a caller-owned 0700 directory."""
        self.directory = directory
        self.stack = ExitStack()
        self.sequence = 0
        self.server = None
        self.pulse = None
        self.environment = isolated_environment(directory, "unused")
        for key in list(self.environment):
            if key.startswith(("PIPEWIRE_", "PULSE_", "SPA_")):
                del self.environment[key]
        self.environment.update(
            PIPEWIRE_RUNTIME_DIR=str(directory), PIPEWIRE_REMOTE="resolibre-test",
            PIPEWIRE_CONFIG_DIR=str(directory), PULSE_RUNTIME_PATH=str(directory),
            PULSE_SERVER=f"unix:{directory}/native", PULSE_CLIENTCONFIG=str(directory / "pulse-client.conf"),
            XDG_RUNTIME_DIR=str(directory), XDG_CONFIG_HOME=str(directory),
            XDG_CONFIG_DIRS=str(directory),
        )
        for name in ("server.conf", "pulse.conf", "client.conf"):
            content = (ROOT / "tools/fixtures/audio-output" / name).read_bytes()
            (directory / name).write_bytes(content)
        # Pulse's default shared-memory pool exceeds the 2 MiB file-size limit.
        # Socket transport keeps that hard bound usable for every test process.
        (directory / "pulse-client.conf").write_text("autospawn = no\nenable-shm = no\nenable-memfd = no\n")

    def __enter__(self):
        """Make every subsequently started child subject to deterministic cleanup."""
        return self

    def __exit__(self, *exception):
        """Reap all children before the caller removes the private directory."""
        self.stack.close()

    def start(self, command, pipe_stdout=False):
        """Start an owned child with unique bounded logs and the private environment."""
        self.sequence += 1
        return Child(self.stack, command, self.directory, self.environment, str(self.sequence), pipe_stdout)

    def command(self, arguments, deadline, allow_failure=False):
        """Run a private control command; never expose native diagnostics on failure."""
        self.sequence += 1
        with ExitStack() as stack:
            child = Child(stack, arguments, self.directory, self.environment, str(self.sequence))
            code = child.wait(deadline)
            output = child.read(child.stdout)
            require(code >= 0, "private control process terminated by signal")
        if not allow_failure:
            require(code == 0, "private control command failed")
        return code, output

    def pulse_query(self, kind, deadline, allow_failure=False):
        """Query only the explicitly addressed private Pulse endpoint as JSON."""
        code, output = self.command(
            ["pactl", f"--server={self.environment['PULSE_SERVER']}",
             "--format=json", "list", kind], deadline, allow_failure,
        )
        if code != 0:
            return None
        return json.loads(output)

    def start_server(self):
        """Start the owned native server and Pulse bridge, with a five-second gate."""
        self.server = self.start(["pipewire", "-c", "server.conf"])
        self.start_pulse()

    def start_pulse(self):
        """Start a new owned bridge; require exactly the expected virtual sink."""
        deadline = time.monotonic() + 5
        # Native must be ready before the bridge attempts its initial connection.
        while not (self.directory / "resolibre-test").is_socket():
            require(self.server.process.poll() is None, "private native server failed startup")
            require(time.monotonic() < deadline, "private server readiness deadline")
            time.sleep(0.02)
        self.pulse = self.start(["pipewire-pulse", "-c", "pulse.conf"])
        while True:
            require(self.pulse.process.poll() is None, "private Pulse service failed startup")
            sinks = self.pulse_query("sinks", deadline, allow_failure=True)
            if sinks:
                require(len(sinks) == 1 and sinks[0]["name"] == "resolibre-test", "unexpected private graph")
                self.sink_index = sinks[0]["index"]
                return
            require(time.monotonic() < deadline, "private Pulse readiness deadline")
            time.sleep(0.02)

    def stop_pulse(self):
        """Sever only our Pulse bridge and prove the explicit endpoint is unavailable."""
        self.pulse.stop()
        result = self.pulse_query("sinks", time.monotonic() + 1, allow_failure=True)
        require(result is None, "private endpoint remained available")

    def start_monitor(self, name, rate=48000):
        """Capture the virtual sink's monitor with bounded raw stereo PCM output."""
        require(rate in (8000, 48000), "unsupported monitor rate")
        destination = self.directory / f"{name}.raw"
        require(not destination.exists(), "monitor destination already exists")
        child = self.start([
            "parec", f"--server={self.environment['PULSE_SERVER']}",
            "--device=resolibre-test.monitor", "--raw", "--format=s16le",
            f"--rate={rate}", "--channels=2", "--latency-msec=20",
            "--process-time-msec=20", str(destination),
        ])
        return child, destination

    def attach(self, worker, deadline, monitor_linked=False):
        """Link owned ports; optionally retain monitor links across verified workers."""
        while True:
            _, outputs = self.command(["pw-link", "-r", "resolibre-test", "-o"], deadline)
            _, inputs = self.command(["pw-link", "-r", "resolibre-test", "-i"], deadline)
            if b"mpv:output_FR" in outputs and b"parec:input_FR" in inputs:
                break
            require(worker.process.poll() is None, "worker exited before stream attachment")
            time.sleep(0.02)
        for channel in ("FL", "FR"):
            for source, target in (
                (f"resolibre-test:monitor_{channel}", f"parec:input_{channel}"),
                (f"mpv:output_{channel}", f"resolibre-test:playback_{channel}"),
            ):
                if monitor_linked and source.startswith("resolibre-test:monitor_"):
                    continue
                self.command(["pw-link", "-r", "resolibre-test", source, target], deadline)
        streams = self.pulse_query("sink-inputs", deadline)
        require(len(streams) == 1, "unexpected playback streams")
        stream = streams[0]
        require(stream["sink"] == self.sink_index, "worker attached to unexpected sink")
        require(stream["properties"]["application.process.id"] == str(worker.process.pid),
                "private stream has unexpected owner")
        require(stream["properties"]["target.object"] == "resolibre-test", "unexpected target selection")
