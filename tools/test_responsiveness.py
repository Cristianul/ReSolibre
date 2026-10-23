"""Regress marker identity, bounded streaming and resource evidence independently of mpv."""

from contextlib import ExitStack
from pathlib import Path
import os
import sys
import tempfile
import unittest
from collections import deque

from check_playback_responsiveness import fingerprint
from private_audio import Child
from responsiveness_probe import Probe, validate_diagnostics
from responsiveness_signal import (
    BLOCK_FRAMES, MAX_SOURCE_BYTES, RATE, Block, MonitorBuffer,
    check_reference_budgets, consecutive, identify, marker_samples, summarize,
)


class SignalChecks(unittest.TestCase):
    """False position, silence and incomplete captures must never become seek evidence."""

    def test_absolute_positions(self):
        """Decode start, fractional, reversed and 103-second positions exactly."""
        for start in (0, 19, 1020, 103 * RATE, 4 * RATE, 111 * RATE):
            data = marker_samples(start, BLOCK_FRAMES)
            self.assertEqual(identify(data), (start, False))

    def test_silence_and_corruption(self):
        """Reject swapped channels, damaged samples and non-contiguous source blocks."""
        self.assertEqual(identify(bytes(BLOCK_FRAMES * 4)), (None, True))
        raw = marker_samples(103 * RATE, BLOCK_FRAMES)
        swapped = b"".join(raw[i + 2:i + 4] + raw[i:i + 2] for i in range(0, len(raw), 4))
        self.assertEqual(identify(swapped), (None, False))
        damaged = bytearray(raw)
        damaged[23] ^= 63
        self.assertEqual(identify(damaged), (None, False))
        mixed = raw[:960] + marker_samples(4 * RATE, BLOCK_FRAMES // 2)
        self.assertEqual(identify(mixed), (None, False))

    def test_stale_and_missing_blocks(self):
        """Source reversals and missing monitor samples cannot confirm continuity."""
        first = Block(0, 100, 1000, False)
        self.assertTrue(consecutive(first, Block(480, 200, 1480, False)))
        self.assertFalse(consecutive(first, Block(480, 200, 500, False)))
        self.assertFalse(consecutive(first, Block(960, 200, 1480, False)))
        self.assertFalse(consecutive(first, Block(480, 200, None, True)))

    def test_fragmented_stream(self):
        """Partial stereo frames retain byte alignment and the final receipt timestamp."""
        history = MonitorBuffer()
        raw = marker_samples(103 * RATE, BLOCK_FRAMES * 2)
        history.feed(raw[:3], 100)
        self.assertFalse(history.blocks)
        history.feed(raw[3:1922], 200)
        history.feed(raw[1922:], 300)
        self.assertEqual([block.position for block in history.blocks], [103 * RATE, 103 * RATE + 480])
        self.assertEqual([block.received_ns for block in history.blocks], [200, 300])
        self.assertEqual(history.pending, b"")

    def test_target_observation(self):
        """Only the requested signal can complete a seek; stale identity times out."""
        probe = Probe.__new__(Probe)
        probe.events = deque()
        probe.history = MonitorBuffer()
        probe.history.feed(marker_samples(4 * RATE, BLOCK_FRAMES * 2), 200)
        calls = []

        def pump(timeout=0.005):
            """Provide one capture turn, then enforce the simulated trial deadline."""
            calls.append(timeout)
            if len(calls) > 1:
                raise ValueError("control trial deadline")

        probe.pump = pump
        result = probe.observe((1, 100), 4, tolerance=960)
        self.assertEqual(result["position"], 4 * RATE)
        calls.clear()
        with self.assertRaisesRegex(ValueError, "deadline"):
            probe.observe((1, 100), 103, tolerance=960)
        calls.clear()
        # A previous player's 4-second tail cannot establish a fresh startup.
        with self.assertRaisesRegex(ValueError, "deadline"):
            probe.observe((1, 100), 0, tolerance=RATE)

    def test_memory_and_clock_bounds(self):
        """Long streams retain finite history; oversized chunks and reversed clocks fail."""
        history = MonitorBuffer()
        for index in range(200):
            history.feed(marker_samples(index * BLOCK_FRAMES, BLOCK_FRAMES), index)
        self.assertEqual(len(history.blocks), 120)
        self.assertLess(len(history.pending), BLOCK_FRAMES * 4)
        with self.assertRaisesRegex(ValueError, "bound"):
            history.feed(bytes(65537), 201)
        with self.assertRaisesRegex(ValueError, "backward"):
            history.feed(b"", 0)
        with self.assertRaisesRegex(ValueError, "incomplete"):
            identify(bytes(12))

    def test_buffered_tail_after_confirmation(self):
        """A wrong tail already read with confirmation must not escape hold checks."""
        probe = Probe.__new__(Probe)
        probe.events = deque()
        probe.history = MonitorBuffer()
        good = marker_samples(4 * RATE, BLOCK_FRAMES * 2)
        wrong_tail = marker_samples(103 * RATE, BLOCK_FRAMES)
        probe.history.feed(good + wrong_tail, 200)

        def pump(timeout=0.005):
            """The entire receipt is already buffered; no additional data is needed."""

        probe.pump = pump
        probe.observe((1, 100), 4, tolerance=960)
        with self.assertRaisesRegex(ValueError, "discontinuous"):
            probe.hold(0.01)

        probe.history = MonitorBuffer()
        probe.history.feed(bytes(BLOCK_FRAMES * 8) + wrong_tail, 200)
        probe.observe((2, 100), silence=True)
        with self.assertRaisesRegex(ValueError, "audio while paused"):
            probe.hold(0.01, silent=True)

    def test_source_bounds(self):
        """A sparse oversized source fails before hashing or allocation."""
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "fixture"
            with source.open("wb") as output:
                output.truncate(MAX_SOURCE_BYTES + 1)
            with self.assertRaisesRegex(ValueError, "bound"):
                fingerprint(source)
        with self.assertRaisesRegex(ValueError, "range"):
            marker_samples(-1, 1)

    def test_pipe_backpressure_cleanup(self):
        """An unread streaming pipe stays bounded and its timed-out child is reaped."""
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            child = Child(stack, [sys.executable, "-c", "import os; os.write(1, b'x' * 1048576)"],
                          Path(directory), os.environ.copy(), "pipe", pipe_stdout=True)
            with self.assertRaisesRegex(ValueError, "deadline"):
                child.wait(child.started + 0.1)
            child.stop()
            self.assertIsNotNone(child.process.returncode)
            self.assertEqual(child.stdout.tell(), 0)

    def test_reference_budget_rejection(self):
        """Incomplete, over-budget and non-finite evidence cannot pass the optional gate."""
        import copy

        measurements = {}
        for name, count in (("play", 100), ("pause", 200), ("resume", 100), ("seek", 400),
                            ("paused_seek_resume", 100), ("burst", 100)):
            measurements[name + "_upper_ms"] = {"count": count, "p95": 10}
        measurements["settled_rss_mib"] = {"max": 70}
        player = {"cpu_percent_one_core": 1, "sampled_peak_rss_mib": 70}
        run = {"playback": {"player": player}, "controls": {"player": player},
               "post_drop_rss_mib": [100] * 60, "cycle_active_rss_mib": [100] * 60}
        result = {"attempts": 100, "failures": 0, "measurements": measurements,
                  "resources": [run] * 3}
        valid = {"wav": result, "flac": result, "allocator_policy": "application-128k"}
        check_reference_budgets(valid)
        with self.assertRaisesRegex(ValueError, "allocator"):
            check_reference_budgets({"wav": result, "flac": result})
        for value in (151, float("nan"), -1):
            bad = copy.deepcopy(valid)
            bad["wav"]["measurements"]["seek_upper_ms"]["p95"] = value
            with self.assertRaisesRegex(ValueError, "budget"):
                check_reference_budgets(bad)
        bad = copy.deepcopy(valid)
        bad["wav"]["resources"] = []
        with self.assertRaisesRegex(ValueError, "incomplete"):
            check_reference_budgets(bad)
        bad = copy.deepcopy(valid)
        bad["wav"]["resources"][0]["post_drop_rss_mib"] = [101] * 60
        with self.assertRaisesRegex(ValueError, "budget"):
            check_reference_budgets(bad)
        bad = copy.deepcopy(valid)
        bad["wav"]["resources"][0]["cycle_active_rss_mib"] = [float("nan")] * 60
        with self.assertRaisesRegex(ValueError, "budget"):
            check_reference_budgets(bad)

    def test_session_diagnostics_are_exact_and_private(self):
        """Only correlated safe error lines pass; private text and duplicate logs fail."""
        safe = b"error: playback failed (libmpv code -9)\n"
        validate_diagnostics(safe, True, 1)
        for text, session, count in ((safe, False, 1), (safe, True, 0),
                                     (safe + safe, True, 1), (b"private path\n", True, 1),
                                     (b"", True, 1)):
            with self.assertRaises(ValueError):
                validate_diagnostics(text, session, count)

    def test_diagnostic_categories_do_not_echo_payloads(self):
        """Unexpected warnings stay failures with a useful category, while private payloads remain hidden."""
        with self.assertRaisesRegex(ValueError, "rhythm excerpt warning"):
            validate_diagnostics(b"warn: rhythm excerpt unavailable\n", True, 0)
        with self.assertRaises(ValueError) as failure:
            validate_diagnostics(b"private path /sensitive/recording.flac\n", True, 0)
        self.assertNotIn("sensitive", str(failure.exception))
        self.assertIn("unclassified", str(failure.exception))

    def test_percentiles(self):
        """Nearest-rank summaries retain count and maximum; empty evidence fails."""
        result = summarize(list(range(1, 101)))
        self.assertEqual(result, {"count": 100, "max": 100, "p50": 50, "p95": 95, "p99": 99})
        with self.assertRaisesRegex(ValueError, "empty"):
            summarize([])


if __name__ == "__main__":
    unittest.main()
