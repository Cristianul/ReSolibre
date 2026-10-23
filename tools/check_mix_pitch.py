#!/usr/bin/env python3
"""Measure MIX pitch and flutter during both tempo ramps on a private audio server."""

from array import array
import math
from pathlib import Path
import tempfile
import time
import wave

from check_mix import link_streams
from check_pcm_conversion import build_worker, snapshot
from pcm_signal import MAX_CAPTURE_BYTES, frequency_hz, require, rms
from private_audio import PrivateAudio


def write_fixture(path):
    """Generate immutable bass and treble in separate channels, independent of tempo detection."""
    rate = 48000
    samples = array('h')
    for index in range(rate * 24):
        for frequency in (55, 440):
            samples.append(round(8000 * math.sin(math.tau * frequency * index / rate)))
    with wave.open(str(path), 'wb') as output:
        output.setparams((2, 2, rate, 0, 'NONE', 'none'))
        output.writeframes(samples.tobytes())
    path.chmod(0o444)


def check_tone(values, rate, frequency):
    """Reject local pitch bends, modulation and dropouts, including near the return to unity."""
    # Both test tones have an integer number of cycles in 200 ms. Sine/cosine
    # projections measure distortion independently of the unknown output phase.
    size = rate // 5
    sine = [math.sin(math.tau * frequency * index / rate) for index in range(size)]
    cosine = [math.cos(math.tau * frequency * index / rate) for index in range(size)]
    worst_cents = 0.0
    worst_residual = 0.0
    expected_rms = 8000 / (32768 * math.sqrt(2))
    require(len(values) >= size * 5, 'insufficient pitch evidence')
    for start in range(0, len(values) - size + 1, size // 2):
        window = values[start:start + size]
        level = rms(window)
        require(abs(level / expected_rms - 1) < .1, 'pitch capture dropout or gain modulation')
        measured = frequency_hz(window, rate)
        cents = abs(1200 * math.log2(measured / frequency))
        power = sum(value * value for value in window) / size
        real = sum(value * reference for value, reference in zip(window, cosine)) / size
        imaginary = sum(value * reference for value, reference in zip(window, sine)) / size
        residual = math.sqrt(max(0, 1 - 2 * (real * real + imaginary * imaginary) / power))
        worst_cents = max(worst_cents, cents)
        worst_residual = max(worst_residual, residual)
    require(worst_cents < 10, f'pitch drift exceeded 10 cents: {worst_cents:.2f}')
    require(worst_residual < .12, f'tone distortion exceeded 12%: {worst_residual:.3f}')
    return round(worst_cents, 2), round(worst_residual * 100, 2)


def trial(directory, binary, direction, stream="incoming"):
    """Capture the real adapter, bound all children/files and verify original/config isolation."""
    source = directory / 'private-input.wav'
    write_fixture(source)
    before = snapshot(source)
    with PrivateAudio(directory) as audio:
        audio.start_server()
        audio.environment['RESOUL_TEST_PITCH_DIRECTION'] = direction
        audio.environment['RESOUL_TEST_PITCH_STREAM'] = stream
        monitor, capture = audio.start_monitor('pitch', 8000)
        worker = audio.start([str(binary), '--ignored', '--exact',
                              'mix_capture_tests::capture_pitch_return', '--nocapture'])
        deadline = time.monotonic() + 28
        linked = set()
        while worker.process.poll() is None:
            require(time.monotonic() < deadline, 'pitch capture deadline')
            require(monitor.process.poll() is None, 'pitch monitor exited early')
            link_streams(audio, worker, linked, deadline)
            time.sleep(.02)
        require(worker.process.returncode == 0, 'pitch worker failed')
        require(b'pitch-return-complete' in worker.read(worker.stdout), 'pitch return did not finish')
        require(not worker.read(worker.stderr), 'unexpected pitch diagnostics')
        require(not (directory / 'private-script-ran').exists(), 'external script executed')
        monitor.stop()
        with capture.open('rb') as capture_stream:
            raw = capture_stream.read(MAX_CAPTURE_BYTES + 1)
        require(0 < len(raw) <= MAX_CAPTURE_BYTES and len(raw) % 4 == 0, 'invalid pitch capture size')
        samples = array('h', raw)
        metrics = []
        for channel, frequency in enumerate((55, 440)):
            values = samples[channel::2]
            active = [index for index, value in enumerate(values) if abs(value) > 1000]
            require(bool(active), 'missing pitch signal')
            # Exclude only device startup/shutdown; retain both ramps and unity settling.
            start = active[0] + 8000
            end = active[-1] - 4000
            require(end - start > 12 * 8000, 'incomplete pitch ramp capture')
            metrics.append(check_tone(values[start:end], 8000, frequency))
        print(f'PASS {stream} {direction}: 55/440 Hz worst (cents, distortion %): {metrics}', flush=True)
    require(snapshot(source) == before, 'pitch test changed original audio')


def main():
    """Exercise the largest accepted tempo difference in both directions, without desktop audio."""
    binary = build_worker()
    with tempfile.TemporaryDirectory(prefix='resolibre-mix-pitch-') as temporary:
        for stream in ('outgoing', 'incoming'):
            for direction in ('up', 'down'):
                directory = Path(temporary) / f'{stream}-{direction}'
                directory.mkdir(mode=0o700)
                trial(directory, binary, direction, stream)


if __name__ == '__main__':
    main()
