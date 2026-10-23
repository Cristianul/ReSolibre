"""Bounded source-position markers and streaming monitor analysis for control tests."""

from array import array
from collections import deque
from dataclasses import dataclass
import sys
import math
import wave

from pcm_signal import require

RATE = 48000
BLOCK_FRAMES = 480
SOURCE_SECONDS = 112
MAX_SOURCE_BYTES = 32 * 1024 * 1024
POSITION_TOLERANCE = 960  # First whole 10 ms block plus one transition block.


@dataclass(frozen=True)
class Block:
    """One 10 ms monitor block with sample identity and a receipt-time upper bound."""

    index: int
    received_ns: int
    position: int | None
    silent: bool


def marker_samples(start, count):
    """Encode absolute frame indices in stereo PCM with quantization guard spacing."""
    require(0 <= start <= start + count <= SOURCE_SECONDS * RATE, "source marker range")
    samples = array("h")
    for frame in range(start, start + count):
        samples.append((frame % 1024 - 512) * 16)
        samples.append((frame // 1024 - 3072) * 4)
    if sys.byteorder != "little":
        samples.byteswap()
    return samples.tobytes()


def write_markers(path):
    """Generate a finite read-only WAV incrementally, never overwriting a destination."""
    require(not path.exists() and not path.is_symlink(), "source destination exists")
    require(SOURCE_SECONDS * RATE * 4 + 44 <= MAX_SOURCE_BYTES, "source exceeds bound")
    with path.open("xb") as destination, wave.open(destination, "wb") as output:
        output.setnchannels(2)
        output.setsampwidth(2)
        output.setframerate(RATE)
        for start in range(0, SOURCE_SECONDS * RATE, 4096):
            output.writeframesraw(marker_samples(start, min(4096, SOURCE_SECONDS * RATE - start)))
    path.chmod(0o444)


def identify(data):
    """Accept only silence or a full contiguous marker; reject ambiguous transitions."""
    require(len(data) == BLOCK_FRAMES * 4, "incomplete marker block")
    samples = array("h", data)
    if sys.byteorder != "little":
        samples.byteswap()
    if max(abs(value) for value in samples) <= 2:
        return None, True
    first = None
    for index in range(BLOCK_FRAMES):
        left, right = samples[index * 2:index * 2 + 2]
        low = round(left / 16) + 512
        high = round(right / 4) + 3072
        position = high * 1024 + low
        if not 0 <= low < 1024 or not 0 <= position < SOURCE_SECONDS * RATE:
            return None, False
        if abs(left - (low - 512) * 16) > 1 or abs(right - (high - 3072) * 4) > 1:
            return None, False
        if first is None:
            first = position
        if position != first + index:
            return None, False
    return first, False


class MonitorBuffer:
    """Retain at most 1.2 seconds of analyzed blocks and one incomplete PCM block."""

    def __init__(self):
        """Initialize finite history and sample accounting independently of wall time."""
        self.pending = bytearray()
        self.blocks = deque(maxlen=120)
        self.frames = 0
        self.last_received = 0

    def feed(self, data, received_ns):
        """Timestamp receipt, not physical output; input chunks are bounded before copy."""
        require(len(data) <= 65536, "monitor chunk exceeds bound")
        require(received_ns >= self.last_received, "monitor clock moved backward")
        self.last_received = received_ns
        self.pending.extend(data)
        while len(self.pending) >= BLOCK_FRAMES * 4:
            raw = bytes(self.pending[:BLOCK_FRAMES * 4])
            del self.pending[:BLOCK_FRAMES * 4]
            position, silent = identify(raw)
            self.blocks.append(Block(self.frames, received_ns, position, silent))
            self.frames += BLOCK_FRAMES


def consecutive(first, second):
    """Confirm source progression over adjacent monitor blocks without stale audio."""
    return (first.position is not None and second.position is not None
            and second.index == first.index + BLOCK_FRAMES
            and second.position == first.position + BLOCK_FRAMES)


def summarize(values):
    """Report nearest-rank percentiles and count without hiding failed observations."""
    require(bool(values), "empty measurement series")
    ordered = sorted(values)
    result = {"count": len(values), "max": round(ordered[-1], 3)}
    for name, percentile in (("p50", 0.50), ("p95", 0.95), ("p99", 0.99)):
        index = max(0, math.ceil(len(values) * percentile) - 1)
        result[name] = round(ordered[index], 3)
    return result


def check_reference_budgets(results):
    """Check the complete reference workload only; other hardware may use reporting mode."""
    require(results.get("allocator_policy") == "application-128k", "missing application allocator evidence")
    controls = {
        "play_upper_ms": (100, 200), "pause_upper_ms": (200, 150),
        "resume_upper_ms": (100, 150), "seek_upper_ms": (400, 150),
        "paused_seek_resume_upper_ms": (100, 150), "burst_upper_ms": (100, 150),
    }
    for codec in ("wav", "flac"):
        result = results[codec]
        require(result["attempts"] == 100 and result["failures"] == 0, "incomplete reference trials")
        measurements = result["measurements"]
        for name, (count, limit) in controls.items():
            metric = measurements[name]
            require(metric["count"] == count, "incomplete reference observations")
            require(0 <= metric["p95"] <= limit, "reference response budget exceeded")
        require(0 <= measurements["settled_rss_mib"]["max"] <= 100, "reference fresh RSS budget exceeded")
        require(len(result["resources"]) == 3, "incomplete resource sessions")
        for run in result["resources"]:
            for phase, cpu_limit in (("playback", 5), ("controls", 10)):
                player = run[phase]["player"]
                require(0 <= player["cpu_percent_one_core"] <= cpu_limit, "reference CPU budget exceeded")
                require(0 <= player["sampled_peak_rss_mib"] <= 100, "reference sampled RSS budget exceeded")
            cycles = run["post_drop_rss_mib"]
            require(len(cycles) == 60, "incomplete lifecycle observations")
            require(all(0 <= value <= 100 for value in cycles), "reference lifecycle RSS budget exceeded")
            active = run["cycle_active_rss_mib"]
            require(len(active) == 60, "incomplete active lifecycle observations")
            require(all(0 <= value <= 100 for value in active), "reference active lifecycle RSS budget exceeded")
