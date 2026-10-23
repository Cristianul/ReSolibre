"""Bounded PCM16 WAV parsing and independent checks for the generated stereo tones."""

from array import array
import math
import struct
import sys


MAX_CAPTURE_BYTES = 2 * 1024 * 1024
PCM_GUID = bytes.fromhex("0100000000001000800000aa00389b71")


def require(condition, message):
    """Reject invalid evidence with a fixed category, even under python -O."""
    if not condition:
        raise ValueError(message)


def parse_format(chunk):
    """Validate only stereo PCM16 at the two fixture rates, including WAV extensible."""
    require(len(chunk) in (16, 40), "unsupported WAV format size")
    kind, channels, rate, byte_rate, alignment, bits = struct.unpack_from("<HHIIHH", chunk)
    require(channels == 2 and bits == 16 and alignment == 4, "unexpected PCM layout")
    require(rate in (44100, 48000) and byte_rate == rate * 4, "invalid PCM rate")
    if kind == 1:
        require(len(chunk) == 16, "unexpected PCM extension")
    else:
        require(kind == 65534 and len(chunk) == 40, "unsupported WAV subtype")
        extension, valid_bits, mask = struct.unpack_from("<HHI", chunk, 16)
        require(extension == 22 and valid_bits == 16 and mask == 3, "invalid WAV extension")
        require(chunk[24:] == PCM_GUID, "unsupported WAV GUID")
    return rate


def parse_wav(data):
    """Check lengths before extracting samples; reject duplicate or incomplete chunks."""
    require(12 <= len(data) <= MAX_CAPTURE_BYTES, "WAV size outside bound")
    require(data[:4] == b"RIFF" and data[8:12] == b"WAVE", "invalid WAV signature")
    require(struct.unpack_from("<I", data, 4)[0] + 8 == len(data), "invalid RIFF length")
    rate = None
    payload = None
    offset = 12

    while offset < len(data):
        require(offset + 8 <= len(data), "truncated chunk header")
        kind, size = struct.unpack_from("<4sI", data, offset)
        start = offset + 8
        end = start + size
        padded_end = end + size % 2
        require(padded_end <= len(data), "truncated chunk payload")
        if kind == b"fmt ":
            require(rate is None, "duplicate format chunk")
            rate = parse_format(data[start:end])
        elif kind == b"data":
            require(payload is None and size > 0 and size % 4 == 0, "invalid data chunk")
            payload = (start, end)
        offset = padded_end

    require(rate is not None and payload is not None, "missing WAV chunks")
    start, end = payload
    samples = array("h")
    samples.frombytes(data[start:end])
    if sys.byteorder != "little":
        samples.byteswap()
    return rate, samples


def read_capture(path):
    """Read at most the capture bound plus a sentinel byte before parsing."""
    with path.open("rb") as capture:
        data = capture.read(MAX_CAPTURE_BYTES + 1)
    return parse_wav(data)


def rms(samples):
    """Return normalized root mean square amplitude for a nonempty window."""
    require(len(samples) > 0, "empty signal window")
    mean_square = sum(float(sample) ** 2 for sample in samples) / len(samples)
    return math.sqrt(mean_square) / 32768


def frequency_hz(samples, rate):
    """Estimate frequency from interpolated rising zero crossings, without resampling."""
    crossings = []
    for index in range(1, len(samples)):
        previous = samples[index - 1]
        current = samples[index]
        if previous <= 0 < current:
            crossing = index - 1 + (-previous / (current - previous))
            crossings.append(crossing)
    require(len(crossings) > 2, "insufficient tone crossings")
    return (len(crossings) - 1) * rate / (crossings[-1] - crossings[0])


def analyze_signal(rate, samples, expected_rate):
    """Check schedule, routing, level and pitch against fixed fixture expectations."""
    require(rate == expected_rate, "unexpected captured rate")
    duration = len(samples) / (2 * rate)
    require(abs(duration - 3.0) <= 0.002, "incorrect signal duration")
    require(all(-32768 < sample < 32767 for sample in samples), "clipped signal")
    expected_rms = 0.08 / math.sqrt(2)
    metrics = {"duration_s": duration, "channels": []}

    for channel, start, frequency in ((0, 0.5, 440), (1, 1.7, 880)):
        values = samples[channel::2]
        end = start + 0.8
        active = [index for index, value in enumerate(values) if abs(value) / 32768 > 0.0002]
        require(bool(active), "missing tone")
        timing_error = max(abs(active[0] / rate - start), abs((active[-1] + 1) / rate - end))
        require(timing_error <= 0.002, "incorrect burst timing")

        window = values[round((start + 0.05) * rate):round((end - 0.05) * rate)]
        measured_rms = rms(window)
        require(abs(measured_rms / expected_rms - 1) <= 0.05, "incorrect tone level")
        measured_frequency = frequency_hz(window, rate)
        require(abs(measured_frequency - frequency) <= 2, "incorrect tone frequency")

        # Short windows also reject local dropouts hidden by whole-burst averages.
        window_frames = round(0.02 * rate)
        for offset in range(0, len(window) - window_frames + 1, window_frames):
            local_rms = rms(window[offset:offset + window_frames])
            require(abs(local_rms / expected_rms - 1) <= 0.05, "tone dropout or level jump")

        silent = values[:round((start - 0.05) * rate)] + values[round((end + 0.05) * rate):]
        silent_rms = rms(silent)
        require(silent_rms < 0.0001, "unexpected signal outside tone")
        metrics["channels"].append({
            "frequency_hz": measured_frequency,
            "rms": measured_rms,
            "timing_error_ms": timing_error * 1000,
            "silent_rms": silent_rms,
        })
    return metrics
