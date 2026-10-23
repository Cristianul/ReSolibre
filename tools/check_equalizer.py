#!/usr/bin/env python3
"""Measure real native EQ response and live coefficient changes on owned signals only."""

from array import array
from collections import deque
from pathlib import Path
import cmath
import math
import tempfile
import time
import wave

from check_pcm_conversion import build_worker, isolated_environment, run_bounded, snapshot
from pcm_signal import require
from private_audio import PrivateAudio
from responsiveness_probe import Probe, start_monitor, read_usage

CENTERS = (31.25, 62.5, 125, 250, 500, 1000, 2000, 4000, 8000, 16000)


def write_tone(path, rate, frequency, amplitude, seconds=2):
    """Create a bounded read-only stereo PCM16 source with known frequency and amplitude."""
    samples = array('h')
    for index in range(round(rate * seconds)):
        value = round(32767 * amplitude * math.sin(2 * math.pi * frequency * index / rate))
        samples.extend((value, value))
    with wave.open(str(path), 'wb') as output:
        output.setnchannels(2)
        output.setsampwidth(2)
        output.setframerate(rate)
        output.writeframes(samples.tobytes())
    path.chmod(0o444)


def read_pcm(path):
    """Read only the bounded PCM16 fixture capture, including standard extensible WAV headers."""
    require(path.stat().st_size <= 2 * 1024 * 1024, 'oversized EQ capture')
    with wave.open(str(path), 'rb') as source:
        require(source.getnchannels() == 2 and source.getsampwidth() == 2, 'unexpected EQ output format')
        rate = source.getframerate()
        samples = array('h', source.readframes(source.getnframes()))
    return rate, samples[::2]


def amplitude(samples):
    """Estimate sine amplitude from a settled multi-cycle RMS window."""
    return math.sqrt(2 * sum(value * value for value in samples) / len(samples)) / 32768


def response(rate, frequency, gains):
    """Independent RBJ peaking-response calculation with the specified Q, evaluated on the unit circle."""
    z = cmath.exp(-2j * math.pi * frequency / rate)
    result = 1.0
    for center, gain in zip(CENTERS, gains):
        if center >= rate / 2:
            continue
        a = 10 ** (gain / 40)
        omega = 2 * math.pi * center / rate
        alpha = math.sin(omega) / (2 * 1.4)
        numerator = (1 + alpha * a) - 2 * math.cos(omega) * z + (1 - alpha * a) * z * z
        denominator = (1 + alpha / a) - 2 * math.cos(omega) * z + (1 - alpha / a) * z * z
        result *= abs(numerator / denominator)
    return result


def write_transient(path, mode):
    """Generate an impulse or logarithmic chirp to expose ringing and wideband behavior."""
    rate = 48000
    values = array('h')
    phase = 0.0
    for index in range(rate * 2):
        frequency = 20 * 1000 ** (index / (rate * 2))
        phase += 2 * math.pi * frequency / rate
        value = (.05 if index == 2400 else 0.0) if mode == 16 else .004 * math.sin(phase)
        values.extend((round(value * 32767), round(value * 32767)))
    with wave.open(str(path), 'wb') as output:
        output.setnchannels(2); output.setsampwidth(2); output.setframerate(rate)
        output.writeframes(values.tobytes())
    path.chmod(0o444)


def reference_signal(samples, rate):
    """Evaluate the specified RBJ cascade independently in Python for transient comparisons."""
    values = [value / 32768 * 10 ** (-2 / 20) for value in samples]
    for center in CENTERS:
        a = 10 ** (12 / 40)
        omega = 2 * math.pi * center / rate
        alpha = math.sin(omega) / (2 * 1.4)
        a0 = 1 + alpha / a
        b0, b1, b2 = (1 + alpha * a) / a0, -2 * math.cos(omega) / a0, (1 - alpha * a) / a0
        a1, a2 = -2 * math.cos(omega) / a0, (1 - alpha / a) / a0
        x1 = x2 = y1 = y2 = 0.0
        for index, value in enumerate(values):
            output = b0 * value + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2
            x2, x1 = x1, value
            y2, y1 = y1, output
            values[index] = output
    return values


def static_cases(binary, root):
    """Check centers, bounded preamp, bypass, combined boosts, clipping and low-rate behavior."""
    for mode in range(18):
        directory = root / str(mode)
        directory.mkdir(mode=0o700)
        rate = 8000 if mode == 15 else 48000
        frequency = CENTERS[mode] if mode < 10 else 1000
        level = .8 if mode == 13 else .04
        source = directory / 'private-input.wav'
        if mode >= 16:
            write_transient(source, mode)
        else:
            write_tone(source, rate, frequency, level)
        before = snapshot(source)
        environment = isolated_environment(directory, 'eq')
        environment['RESOUL_EQ_CASE'] = str(mode)
        code, output, diagnostics = run_bounded([binary, '--ignored', '--exact', 'eq_capture_tests::capture_equalizer', '--nocapture'], directory, environment)
        require(code == 0 and not diagnostics, 'native EQ capture failed')
        require(snapshot(source) == before, 'EQ modified its source')
        require(b'private' not in output and not (directory / 'private-script-ran').exists(), 'EQ privacy/isolation failure')
        actual_rate, samples = read_pcm(directory / 'capture.wav')
        require(actual_rate == rate, 'EQ changed source rate')
        require(len(samples) == rate * 2, 'EQ capture lost source frames')
        settled = samples[rate:]
        if mode >= 16:
            _, original = read_pcm(source)
            expected_signal = reference_signal(original, rate)
            require(len(samples) == len(expected_signal), 'EQ impulse/sweep length changed')
            error = max(abs(actual / 32768 - expected) for actual, expected in zip(samples, expected_signal))
            require(error < .0002, 'EQ impulse/sweep differs from specified response')
            print(f'PASS EQ case {mode}: impulse/sweep max error={error:.7f}', flush=True)
            continue
        if mode == 13:
            require(max(abs(value) for value in settled) >= 32767, 'manual overload case did not expose clipping')
        else:
            if mode < 10:
                expected = level * 10 ** (6 / 20)
            elif mode == 10:
                expected = level * 10 ** (2 / 20)
            elif mode in (12, 14):
                expected = level * response(rate, frequency, [12] * 10) * (10 ** (-2 / 20) if mode == 14 else 1)
            else:
                expected = level
            measured = amplitude(settled)
            error_db = 20 * math.log10(measured / expected)
            require(abs(error_db) < .10, 'EQ response outside 0.10 dB tolerance')
            require(max(abs(value) for value in settled) < 32767, 'unexpected EQ clipping')
        print(f'PASS EQ case {mode}: response/peak/source checks', flush=True)


class Samples:
    """Retain at most two seconds of private monitor samples; no unbounded audio capture."""
    def __init__(self):
        """Create a bounded interleaved PCM16 window and partial-byte buffer."""
        self.values = deque(maxlen=48000 * 4)
        self.pending = b''

    def feed(self, data, received_ns):
        """Consume bounded monitor chunks while preserving stereo frame boundaries."""
        require(len(data) <= 65536, 'oversized live EQ chunk')
        data = self.pending + data
        length = len(data) // 4 * 4
        self.values.extend(array('h', data[:length]))
        self.pending = data[length:]


def collect(probe, seconds):
    """Pump real output for a bounded interval and reject any native terminal failure."""
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        probe.pump()
        require(not probe.events, 'live EQ unexpectedly stopped')


def live_case(binary, root):
    """Measure live 1 kHz gain ramps, output discontinuity residual and pause/seek recovery."""
    directory = root / 'live'
    directory.mkdir(mode=0o700)
    source = directory / 'private-input.wav'
    write_tone(source, 48000, 1000, .04, seconds=9)
    before = snapshot(source)
    with PrivateAudio(directory) as audio:
        audio.start_server()
        monitor = start_monitor(audio)
        samples = Samples()
        worker = ['env', 'RESOUL_SESSION_PROBE=1', 'MALLOC_MMAP_THRESHOLD_=131072', binary]
        probe = Probe(audio, worker, monitor, samples, lifetime=25)
        probe.acknowledge(probe.send('open'))
        audio.attach(probe.worker, time.monotonic() + 5)
        collect(probe, .5)
        started = time.monotonic()
        cpu_before = read_usage(probe.worker.process.pid)[0]
        peak_rss = 0
        for db in (12, -12, 6, 0):
            samples.values.clear()
            probe.acknowledge(probe.send(f'eq {db}'))
            collect(probe, 1.3)
            values = list(samples.values)[::2]
            measured = amplitude(values[-12000:])
            peak_rss = max(peak_rss, read_usage(probe.worker.process.pid)[1])
            require(abs(20 * math.log10(measured / (.04 * 10 ** (db / 20)))) < .15, 'live EQ gain did not settle')
            residual = max(abs(values[i] - 2 * math.cos(2 * math.pi * 1000 / 48000) * values[i-1] + values[i-2]) / 32768 for i in range(2, len(values)))
            require(residual < .005, 'live EQ discontinuity exceeded fixture threshold')
            print(f'PASS live EQ {db:+} dB: residual peak={residual:.6f}', flush=True)
        cpu = 100 * (read_usage(probe.worker.process.pid)[0] - cpu_before) / (time.monotonic() - started)
        require(peak_rss <= 100, 'live EQ audio RSS budget exceeded')
        print(f'live EQ resources: sampled RSS={peak_rss:.3f} MiB, CPU={cpu:.2f}% one core', flush=True)
        probe.acknowledge(probe.send('pause'))
        probe.acknowledge(probe.send('eq 6'))
        collect(probe, .3)
        probe.acknowledge(probe.send('seek 1'))
        probe.acknowledge(probe.send('resume'))
        collect(probe, 1.0)
        require(abs(20 * math.log10(amplitude(list(samples.values)[::2][-12000:]) / (.04 * 10 ** (6 / 20)))) < .15, 'EQ lost after paused seek')
        probe.finish()
    require(snapshot(source) == before, 'live EQ changed source')


def main():
    """Run offline captures and a private monitor; no physical output/listening claim."""
    binary = build_worker()
    with tempfile.TemporaryDirectory(prefix='resolibre-eq-') as temporary:
        root = Path(temporary)
        static_cases(binary, root)
        live_case(binary, root)
    print('PASS: EQ digital evidence; manual overload is intentional, no automatic limiter')


if __name__ == '__main__':
    main()
