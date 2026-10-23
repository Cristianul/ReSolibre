#!/usr/bin/env python3
"""Capture production native trim/fade filters on private, immutable signals."""
from array import array
from pathlib import Path
import math
import tempfile
import subprocess
import wave
from check_pcm_conversion import build_worker, isolated_environment, run_bounded, snapshot
from pcm_signal import require


def fixture(path):
    """Create antiphase stereo tone with one second of quiet at each edge."""
    samples = array('h')
    rate = 48000
    for frame in range(rate * 5):
        time = frame / rate
        sample = round(8000 * math.sin(math.tau * 440 * time)) if 1 <= time < 4 else 0
        samples.extend((sample, -sample))
    with wave.open(str(path), 'wb') as output:
        output.setnchannels(2)
        output.setsampwidth(2)
        output.setframerate(rate)
        output.writeframes(samples.tobytes())
    path.chmod(0o444)


def main():
    """Check rendered ramps, trim duration, a seek into fade-out and source preservation."""
    binary = build_worker()
    with tempfile.TemporaryDirectory(prefix='resolibre-edges-') as temporary:
        for mode in range(8):
            directory = Path(temporary) / str(mode)
            directory.mkdir(mode=0o700)
            source = directory / 'private-input.wav'
            fixture(source)
            if mode >= 6:
                encoded = directory / ('converted.flac' if mode == 6 else 'converted.mp3')
                subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(source), str(encoded)], check=True, capture_output=True)
                source.unlink()
                encoded.rename(source)
                source.chmod(0o444)
            before = snapshot(source)
            env = isolated_environment(directory, 'edges')
            env['RESOUL_EDGES_CASE'] = str(mode)
            code, output, diagnostics = run_bounded([binary, '--ignored', '--exact', 'edges_capture_tests::capture_edges', '--nocapture'], directory, env)
            require(code == 0 and not diagnostics, f'edge capture {mode} failed: {output[-1200:]}')
            require(snapshot(source) == before, 'processing modified the source')
            with wave.open(str(directory / 'capture.wav'), 'rb') as capture:
                require(capture.getsampwidth() == 2 and capture.getnchannels() == 2, 'unexpected capture format')
                rate = capture.getframerate()
                samples = array('h', capture.readframes(capture.getnframes()))[::2]
            start = 0 if mode == 4 else 3.55 if mode == 5 else .95
            end = 5 if mode == 4 else 4.05
            expected_length = end - start
            # The untimed PCM sink can retain whole priming blocks before the paused seek.
            wanted = round(expected_length * rate)
            while len(samples) > wanted + rate * .1 and len(samples) >= 65536:
                samples = samples[65536:]
            require(abs(len(samples) / rate - expected_length) < .05, f'case {mode}: wrong effective duration {len(samples)/rate}')
            if mode >= 6:
                require(max(abs(value) for value in samples) > 5000, 'encoded input lost its audio')
                print(f'PASS edges {mode}: FLAC/MP3 bounds, audio and source unchanged', flush=True)
                continue
            errors = []
            for index, actual in enumerate(samples):
                time = start + index / rate
                original = 8000 * math.sin(math.tau * 440 * time) if 1 <= time < 4 else 0
                lower, upper = (0, 5) if mode == 4 else (.95, 4.05)
                gain = 1
                if mode in (1, 3, 4, 5):
                    gain *= min(1, max(0, time - lower))
                if mode in (2, 3, 4, 5):
                    gain *= min(1, max(0, upper - time))
                errors.append(abs(actual - original * gain) / 32768)
            require(sum(errors) / max(1, len(errors)) < .012, f'case {mode}: incorrect sample envelope, mean error {sum(errors)/len(errors)}')
            print(f'PASS edges {mode}: {len(samples)/rate:.3f}s, sample envelope/source unchanged', flush=True)
    print('PASS: native trim/fade digital captures')


if __name__ == '__main__':
    main()
