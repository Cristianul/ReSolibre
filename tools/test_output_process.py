"""Regress private fixture isolation, process deadlines and startup failures."""

from contextlib import ExitStack
from pathlib import Path
import os
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from pcm_signal import MAX_CAPTURE_BYTES
from private_audio import Child, PrivateAudio


class OutputProcessChecks(unittest.TestCase):
    """Exercise the real helper boundary without playing through physical devices."""

    def test_timeout_reaps_group(self):
        """Escalate a TERM-resistant worker and descendant, then reap the worker."""
        with tempfile.TemporaryDirectory(prefix="resolibre-process-") as temporary:
            directory = Path(temporary)
            script = (
                "import os, signal, subprocess, sys, time\n"
                "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                "child = subprocess.Popen([sys.executable, '-c', "
                "'import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(30)'])\n"
                "open('descendant', 'w').write(str(child.pid))\n"
                "time.sleep(30)\n"
            )
            with ExitStack() as stack:
                worker = Child(stack, [sys.executable, "-c", script], directory, os.environ.copy(), "timeout")
                with self.assertRaisesRegex(ValueError, "deadline"):
                    worker.wait(worker.started + 1)
            with self.assertRaises(ChildProcessError):
                os.waitpid(worker.process.pid, os.WNOHANG)
            descendant = int((directory / "descendant").read_text())
            # Linux may briefly retain the orphan as a zombie until init reaps it.
            status = Path(f"/proc/{descendant}/status")
            deadline = time.monotonic() + 1
            while status.exists():
                try:
                    state = status.read_text()
                except FileNotFoundError:
                    break
                if "State:\tZ" in state:
                    break
                self.assertLess(time.monotonic(), deadline, "descendant remained alive")
                time.sleep(0.01)

    def test_file_limit(self):
        """An oversized child write must fail without exceeding the kernel bound."""
        with tempfile.TemporaryDirectory(prefix="resolibre-process-") as temporary:
            directory = Path(temporary)
            script = (
                "with open('capture', 'wb') as capture:\n"
                f"    capture.write(b'x' * {MAX_CAPTURE_BYTES + 1})\n"
            )
            with ExitStack() as stack:
                worker = Child(stack, [sys.executable, "-c", script], directory, os.environ.copy(), "limit")
                self.assertNotEqual(worker.wait(worker.started + 2), 0)
                self.assertLessEqual((directory / "capture").stat().st_size, MAX_CAPTURE_BYTES)

    def test_environment_isolation(self):
        """Inherited server, cookie, config and plugin overrides cannot redirect tests."""
        overrides = {
            "PULSE_SERVER": "tcp:unexpected", "PIPEWIRE_REMOTE": "unexpected",
            "PIPEWIRE_CONFIG_NAME": "unexpected.conf", "PULSE_COOKIE": "/unexpected",
            "SPA_PLUGIN_DIR": "/unexpected", "PIPEWIRE_MODULE_DIR": "/unexpected",
        }
        with tempfile.TemporaryDirectory(prefix="resolibre-isolation-") as temporary:
            directory = Path(temporary)
            with patch.dict(os.environ, overrides), PrivateAudio(directory) as audio:
                self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
                self.assertEqual(audio.environment["PULSE_SERVER"], f"unix:{directory}/native")
                self.assertEqual(audio.environment["PIPEWIRE_REMOTE"], "resolibre-test")
                for name in overrides.keys() - {"PULSE_SERVER", "PIPEWIRE_REMOTE"}:
                    self.assertNotIn(name, audio.environment)
                self.assertEqual(audio.environment["PIPEWIRE_CONFIG_DIR"], str(directory))
                self.assertEqual(audio.environment["XDG_CONFIG_HOME"], str(directory))

    def test_failed_service_startup(self):
        """Reject a missing required module and reap services without desktop access."""
        with tempfile.TemporaryDirectory(prefix="resolibre-startup-") as temporary:
            directory = Path(temporary)
            with PrivateAudio(directory) as audio:
                (directory / "server.conf").write_text(
                    "context.modules = [ { name = resolibre-nonexistent-module } ]\n"
                )
                with self.assertRaisesRegex(ValueError, "startup"):
                    audio.start_server()
            with self.assertRaises(ChildProcessError):
                os.waitpid(audio.server.process.pid, os.WNOHANG)
            self.assertFalse((directory / "native").exists())


if __name__ == "__main__":
    unittest.main()
