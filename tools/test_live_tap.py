"""Negative tests for the isolated PCM-tap fixture boundary; no native engine needed."""

import os
from pathlib import Path
import tempfile
import unittest

from check_live_tap import open_fixture, reserve_capture


class FixtureBoundaryTests(unittest.TestCase):
    """Protect originals and reject blocking, oversized or redirected fixture inputs."""

    def test_read_only_regular_source(self):
        """The opened descriptor cannot modify an otherwise writable fixture."""
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "source.wav"
            path.write_bytes(b"fixture")
            with open_fixture(path) as source:
                self.assertEqual(source.read(), b"fixture")
                with self.assertRaises(OSError):
                    os.write(source.fileno(), b"changed")
            self.assertEqual(path.read_bytes(), b"fixture")

    def test_rejects_symlink_fifo_directory_and_large_input(self):
        """Invalid fixtures fail before decoding, including FIFOs without a writer."""
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            large = directory / "large.wav"
            with large.open("wb") as output:
                output.truncate(4 * 1024 * 1024 + 1)
            link = directory / "link.wav"
            link.symlink_to(large)
            fifo = directory / "fifo.wav"
            os.mkfifo(fifo)
            empty = directory / "empty.wav"
            empty.touch()
            for path in (large, link, fifo, directory, empty):
                with self.subTest(kind=path.name):
                    with self.assertRaises((OSError, ValueError)):
                        open_fixture(path)

    def test_capture_cannot_replace_an_existing_file(self):
        """An output spelling that resolves to an original is rejected without truncation."""
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "original.wav"
            path.write_bytes(b"original")
            with self.assertRaises(FileExistsError):
                reserve_capture(path)
            self.assertEqual(path.read_bytes(), b"original")

    def test_capture_requires_private_directory(self):
        """Only the harness's owned directory may receive native PCM output."""
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            directory.chmod(0o755)
            path = directory / "capture.wav"
            with self.assertRaises(ValueError):
                reserve_capture(path)
            self.assertFalse(path.exists())
            directory.chmod(0o700)
            reserve_capture(path)
            self.assertEqual(path.stat().st_size, 0)


if __name__ == "__main__":
    unittest.main()
