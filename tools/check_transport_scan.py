#!/usr/bin/env python3
"""Measure 3x pitch-preserving scans on generated WAV, FLAC and MP3; never user media."""
from pathlib import Path
import math
import subprocess
import tempfile
from array import array
import wave

from check_pcm_conversion import build_worker, isolated_environment, run_bounded, snapshot
from pcm_signal import frequency_hz, read_capture, require, rms


def write_fixture(path):
    """Create a read-only stereo tone whose pitch changes halfway to prove traversal order."""
    samples = array('h')
    rate = 48000
    for index in range(rate * 12):
        frequency = 440 if index < rate * 6 else 880
        value = round(4000 * math.sin(2 * math.pi * frequency * index / rate))
        samples.extend((value, value))
    with wave.open(str(path), 'wb') as output:
        output.setparams((2, 2, rate, 0, 'NONE', 'none'))
        output.writeframes(samples.tobytes())
    path.chmod(0o444)


def main():
    """Run bounded native captures and require unchanged pitch, 1/3 duration and source preservation."""
    binary = build_worker()
    with tempfile.TemporaryDirectory(prefix='resolibre-scan-') as temporary:
        root = Path(temporary)
        for extension in ('wav', 'flac', 'mp3'):
            for direction in ('forward', 'backward'):
                mode = f'{extension}-{direction}'
                directory = root / mode
                directory.mkdir(mode=0o700)
                wav = directory / 'private-input.wav'
                write_fixture(wav)
                source = directory / f'private-input.{extension}'
                if extension != 'wav':
                    subprocess.run(['ffmpeg', '-v', 'error', '-i', str(wav), str(source)],
                                   check=True, capture_output=True, timeout=10)
                    source.chmod(0o444)
                original = snapshot(source)
                environment = isolated_environment(directory, 'source')
                environment['RESOUL_SCAN_CASE'] = mode
                command = [binary, '--ignored', '--exact', 'scan_capture_tests::capture_scan', '--nocapture']
                code, output, diagnostics = run_bounded(command, directory, environment)
                require(code == 0, f'{mode}: capture failed; inspect the fixed capture test')
                require(b'1 passed' in output, 'capture did not execute')
                require(b'private' not in output + diagnostics, 'private diagnostic leaked')
                require(not (directory / 'private-script-ran').exists(), 'external script ran')
                require(snapshot(source) == original, 'source changed')
                rate, samples = read_capture(directory / 'capture.wav')
                captured_duration = len(samples) / (2 * rate)
                # mpv's untimed PCM sink writes its 65,536-frame device buffer immediately;
                # reset() cannot retract that pre-scan prefix as a real output device can.
                # Permit only whole priming buffers, then verify the scanned signal itself.
                prefix_buffers = round((captured_duration - 4) * rate / 65536)
                require(0 <= prefix_buffers <= 2, f'{mode}: excessive priming')
                samples = samples[prefix_buffers * 65536 * 2:]
                duration = len(samples) / (2 * rate)
                require(abs(duration - 4) < 0.2, f'{mode}: scan duration {duration}')
                expected = (440, 880) if direction == 'forward' else (880, 440)
                frequencies = []
                for start, frequency in zip((0.6, 2.6), expected):
                    window = samples[round(start * rate) * 2:round((start + 0.5) * rate) * 2:2]
                    measured = frequency_hz(window, rate)
                    require(abs(measured - frequency) < 3, f'{mode}: pitch {measured}')
                    require(rms(window) > 0.03, 'inaudible scan')
                    frequencies.append(round(measured, 2))
                print(f'PASS {mode}: scan {duration:.3f}s; pitch {frequencies} Hz; priming buffers {prefix_buffers}')


if __name__ == '__main__':
    main()
