"""Exercise real subprocess deadlines and kernel write limits without libmpv."""

from pathlib import Path
import os
import sys
import tempfile
import unittest

from check_pcm_conversion import run_bounded
from pcm_signal import MAX_CAPTURE_BYTES


class ProcessChecks(unittest.TestCase):
    """Verify that a stuck or excessive writer cannot escape the capture bounds."""

    def test_timeout_reaps_worker(self):
        """Time out a child and verify it has been reaped before directory cleanup."""
        with tempfile.TemporaryDirectory(prefix="resolibre-process-") as temporary:
            directory = Path(temporary)
            script = 'import os, time; open("pid", "w").write(str(os.getpid())); time.sleep(30)'
            with self.assertRaisesRegex(ValueError, "deadline"):
                run_bounded([sys.executable, "-c", script], directory, os.environ.copy(), timeout=1)
            pid = int((directory / "pid").read_text())
            with self.assertRaises(ChildProcessError):
                os.waitpid(pid, os.WNOHANG)

    def test_kernel_bounds_file_writes(self):
        """Attempt an oversized native-style write and check the resulting file bound."""
        with tempfile.TemporaryDirectory(prefix="resolibre-process-") as temporary:
            directory = Path(temporary)
            script = (
                'with open("capture", "wb") as capture:\n'
                f'    capture.write(b"x" * {MAX_CAPTURE_BYTES + 1})\n'
            )
            code, _, _ = run_bounded([sys.executable, "-c", script], directory, os.environ.copy())
            self.assertNotEqual(code, 0)
            self.assertLessEqual((directory / "capture").stat().st_size, MAX_CAPTURE_BYTES)


if __name__ == "__main__":
    unittest.main()
