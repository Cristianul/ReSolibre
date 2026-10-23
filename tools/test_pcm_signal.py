"""Negative controls for the capture analyzer, independent of libmpv."""

from array import array
from pathlib import Path
import math
import struct
import tempfile
import unittest

from check_audible_playback import write_tones
from pcm_signal import MAX_CAPTURE_BYTES, analyze_signal, parse_wav


class SignalChecks(unittest.TestCase):
    """Require signal and container corruption to fail visibly."""

    @classmethod
    def setUpClass(cls):
        """Generate one disposable source and retain bounded bytes for mutations."""
        with tempfile.TemporaryDirectory(prefix="resolibre-analyzer-") as temporary:
            path = Path(temporary) / "source.wav"
            write_tones(path, 48000, 2)
            cls.valid = path.read_bytes()
        cls.rate, cls.samples = parse_wav(cls.valid)

    def test_valid_signal(self):
        """Accept a known fixture without using the native converter as an oracle."""
        result = analyze_signal(self.rate, self.samples, 48000)
        self.assertEqual(result["duration_s"], 3)

    def test_corrupt_signals(self):
        """Reject silence, channel swaps, wrong pitch, clipping, truncation and dropouts."""
        silence = array("h", [0]) * len(self.samples)
        swapped = array("h", self.samples)
        swapped[0::2], swapped[1::2] = swapped[1::2], swapped[0::2]
        wrong_tone = array("h", self.samples)
        for frame in range(round(0.55 * self.rate), round(1.25 * self.rate)):
            wrong_tone[frame * 2] = round(0.08 * 32767 * math.sin(2 * math.pi * 660 * frame / self.rate))
        clipped = array("h", self.samples)
        clipped[self.rate * 2] = 32767
        dropout = array("h", self.samples)
        dropout[self.rate * 2:self.rate * 2 + 1920] = array("h", [0]) * 1920

        for name, samples in (
            ("silence", silence), ("swapped", swapped), ("wrong tone", wrong_tone),
            ("clipped", clipped), ("truncated", self.samples[:-1000]), ("dropout", dropout),
        ):
            with self.subTest(name=name), self.assertRaises(ValueError):
                analyze_signal(self.rate, samples, 48000)

    def test_wrong_rate(self):
        """Reject a captured rate that disagrees with the requested case."""
        with self.assertRaisesRegex(ValueError, "captured rate"):
            analyze_signal(self.rate, self.samples, 44100)

    def test_invalid_containers(self):
        """Reject inconsistent sizes, duplicate chunks and unsupported PCM metadata."""
        huge_chunk = bytearray(self.valid)
        struct.pack_into("<I", huge_chunk, 40, 0xFFFFFFFF)
        bad_rate = bytearray(self.valid)
        struct.pack_into("<I", bad_rate, 24, 44100)
        bad_channels = bytearray(self.valid)
        struct.pack_into("<H", bad_channels, 22, 1)
        duplicate = bytearray(self.valid + self.valid[12:36])
        struct.pack_into("<I", duplicate, 4, len(duplicate) - 8)
        for data in (
            b"", self.valid[:-1], self.valid + b"extra", huge_chunk, bad_rate,
            bad_channels, duplicate, b"\0" * (MAX_CAPTURE_BYTES + 1),
        ):
            with self.subTest(length=len(data)), self.assertRaises(ValueError):
                parse_wav(data)

    def test_extensible_metadata(self):
        """Accept the native PCM subtype and reject misleading extension fields."""
        base = bytearray(self.valid[20:36])
        struct.pack_into("<H", base, 0, 65534)
        extension = struct.pack("<HHI", 22, 16, 3)
        extension += bytes.fromhex("0100000000001000800000aa00389b71")
        body = b"WAVEfmt " + struct.pack("<I", 40) + base + extension + self.valid[36:]
        valid = b"RIFF" + struct.pack("<I", len(body)) + body
        rate, samples = parse_wav(valid)
        self.assertEqual(rate, self.rate)
        self.assertEqual(samples, self.samples)

        for offset in (36, 38, 40, 44):
            damaged = bytearray(valid)
            damaged[offset] ^= 1
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                parse_wav(damaged)


if __name__ == "__main__":
    unittest.main()
