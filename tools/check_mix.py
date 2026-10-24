#!/usr/bin/env python3
"""Measure two production libmpv streams on an isolated audio server with owned rhythmic fixtures."""

from array import array
import argparse
from pathlib import Path
import re
import resource
import statistics
import math
import tempfile
import time
import wave

from check_pcm_conversion import build_worker, snapshot
from private_audio import PrivateAudio
from pcm_signal import require
from responsiveness_probe import read_usage


def limit_mix_worker_files():
    """Allow bounded full-analysis SQLite writes while raw audio and logs keep their 2 MiB checks."""
    maximum = 16 * 1024 * 1024
    resource.setrlimit(resource.RLIMIT_FSIZE, (maximum, maximum))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def fixture(path, bpm, channel, edge=1, tonal=False, duration=24, rate=48000, sparse=False):
    """Place each song's attacks in a separate channel so monitor timing can be measured independently."""
    samples = array('h')
    for index in range(duration * rate):
        seconds = index / rate
        phase = (seconds - edge) % (60 / bpm)
        value = 0
        omitted = sparse and math.floor((seconds - edge) / (60 / bpm)) % 7 == 3
        if edge <= seconds < duration - edge and tonal:
            value = round(8000 * math.sin(math.tau * (900 if channel == 0 else 440) * seconds))
        elif edge <= seconds < duration - edge and phase < .035 and not omitted:
            value = round(14000 * math.sin(math.tau * 900 * phase) * math.exp(-110 * phase))
        samples.extend((value, 0) if channel == 0 else (0, value))
    with wave.open(str(path), 'wb') as output:
        output.setnchannels(2)
        output.setsampwidth(2)
        output.setframerate(rate)
        output.writeframes(samples.tobytes())
    path.chmod(0o444)


def link_streams(audio, worker, linked, deadline):
    """Link only the private graph's owned playback aliases; reject unexpected stream identities."""
    aliases = {}
    for direction in ('-o', '-i'):
        _, raw = audio.command(['pw-link', '-r', 'resolibre-test', '-I', direction], deadline)
        for line in raw.decode('utf-8').splitlines():
            match = re.fullmatch(r'\s*(\d+)\s+(.+)', line)
            if match:
                aliases.setdefault(match[2], []).append(int(match[1]))
    for channel in ('FL', 'FR'):
        pairs = [(source, target) for source in (aliases.get(f'mpv:output_{channel}', []) + aliases.get(f'resolibre-test-{worker.process.pid}:output_{channel}', []))
                 for target in aliases.get(f'resolibre-test:playback_{channel}', [])]
        pairs += [(source, target) for source in aliases.get(f'resolibre-test:monitor_{channel}', [])
                  for target in aliases.get(f'parec:input_{channel}', [])]
        for pair in pairs:
            if pair not in linked:
                audio.command(['pw-link', '-r', 'resolibre-test', *map(str, pair)], deadline, allow_failure=True)
                linked.add(pair)
    streams = audio.pulse_query('sink-inputs', deadline)
    require(len(streams) <= 2, 'unexpected extra playback streams')
    for stream in streams:
        require(stream['properties'].get('application.process.id') == str(worker.process.pid)
                or stream['properties'].get('application.name') == f'resolibre-test-{worker.process.pid}', 'foreign playback owner')
        # Pulse briefly publishes a new or draining stream without an attached sink.
        require(stream['sink'] in (audio.sink_index, 0xffffffff), 'unexpected playback sink')
    return sum(stream['sink'] == audio.sink_index for stream in streams)


def attacks(values, rate=48000):
    """Measure per-channel RMS onsets at one-millisecond resolution, suppressing within-attack duplicates."""
    size = rate // 1000
    energies = [sum(value * value for value in values[index:index + size]) / size
                for index in range(0, len(values) - size, size)]
    found = []
    for index in range(1, len(energies) - 1):
        if energies[index] > 25 ** 2 and energies[index] >= energies[index - 1] and energies[index] > energies[index + 1]:
            if not found or index - found[-1][0] > 200:
                found.append((index, energies[index]))
            elif energies[index] > found[-1][1]:
                found[-1] = (index, energies[index])
    return [index / 1000 for index, _ in found]


def final_beat_periods(beats, capture_duration):
    """Measure four settled periods with enough capture left to contain each complete attack."""
    # Abrupt test shutdown can retain only a faint pre-echo of the next pulse,
    # before its real peak. Keep 100 ms of context; do not change timing tolerances.
    complete = [beat for beat in beats if beat < capture_duration - .1]
    tail = complete[-5:]
    return [after - before for before, after in zip(tail, tail[1:])]


def trial(directory, binary, outgoing_bpm, incoming_bpm, missing_clock=False, trim=True, tonal=False, long=False, pipewire=False, edge=None, invalid_clock=False, sparse=False, tempo_only=False, beat_ratio=(1, 1)):
    """Capture natural transitions with and without TRIM; require audible overlap, not just state labels."""
    outgoing = directory / 'outgoing.wav'
    incoming = directory / 'incoming.wav'
    if edge is None:
        edge = 1 if trim else 2
    duration = 240 if long else 24
    rate = 8000 if long else 48000
    fixture(outgoing, outgoing_bpm, 0, edge, tonal, duration, rate, sparse)
    fixture(incoming, incoming_bpm, 1, edge, tonal, duration, rate, sparse)
    before = [snapshot(path) for path in (outgoing, incoming)]
    with PrivateAudio(directory) as audio:
        audio.start_server()
        if pipewire:
            audio.environment["RESOUL_TEST_MIX_PIPEWIRE"] = "1"
            (directory / 'native-client.conf').write_text(
                'context.properties = { support.dbus = false }\n'
                'context.spa-libs = { audio.convert.* = audioconvert/libspa-audioconvert support.* = support/libspa-support }\n'
                'context.modules = [ { name = libpipewire-module-protocol-native } '
                '{ name = libpipewire-module-client-node } { name = libpipewire-module-adapter } ]\n'
                'stream.properties = { adapter.auto-port-config = { mode = dsp position = preserve } }\n')
        audio.environment["RESOUL_TEST_MIX_RATIO"] = f"{beat_ratio[0]}:{beat_ratio[1]}"
        if tempo_only:
            audio.environment["RESOUL_TEST_MIX_TEMPO_ONLY"] = "1"
        if long:
            audio.environment["RESOUL_TEST_MIX_LONG"] = "1"
        if invalid_clock:
            audio.environment["RESOUL_TEST_INVALID_AUDIO_CLOCK"] = "1"
        if missing_clock:
            audio.environment["RESOUL_TEST_NO_AUDIO_CLOCK"] = "1"
        if not trim:
            audio.environment["RESOUL_TEST_MIX_NO_TRIM"] = "1"
        monitor = None
        capture = None
        worker = audio.start(['env', 'MALLOC_MMAP_THRESHOLD_=131072',
                              'GLIBC_TUNABLES=glibc.malloc.mmap_threshold=131072',
                              *(['PIPEWIRE_CONFIG_NAME=native-client.conf'] if pipewire else []), str(binary), '--ignored', '--exact', 'mix_capture_tests::capture_mix', '--nocapture'],
                             file_limit=limit_mix_worker_files)
        deadline = time.monotonic() + (70 if long else 40)
        linked = set()
        peak_rss = 0.0
        while worker.process.poll() is None:
            require(time.monotonic() < deadline, 'mix capture deadline')
            peak_rss = max(peak_rss, read_usage(worker.process.pid)[1])
            link_streams(audio, worker, linked, deadline)
            # An 8 kHz monitor retains onset timing and the post-handover tail within 2 MiB.
            if monitor is None and time.monotonic() - worker.started >= (14 if trim else 15):
                monitor, capture = audio.start_monitor('mix', 8000)
            if monitor is not None:
                require(monitor.process.poll() is None, 'mix monitor ended before capture completed')
            time.sleep(.03)
        if worker.process.returncode != 0 or worker.read(worker.stderr):
            for line in worker.read(worker.stderr).decode('utf-8', errors='replace').splitlines():
                if line.startswith(('warn: MIX unavailable at ', 'warn: some track metadata unavailable;', 'warn: persistent media cache unavailable;', 'mix-error: ', 'assertion', 'measured BPM never', 'beat mixing was', 'fixture ', 'long MIX', 'long overlap', 'handover must', 'fallback cannot')):
                    print(line, flush=True)
        require(worker.process.returncode == 0, 'mix worker failed')
        require(monitor is not None, 'mix finished before capture window')
        monitor.stop()
        require(not worker.read(worker.stderr), 'unexpected mix diagnostics')
        require(b'mix-complete' in worker.read(worker.stdout), 'missing mix completion')
        if invalid_clock:
            require(b'mix-clock-invalid' in worker.read(worker.stdout), 'clock fault was not exercised')
        raw = capture.read_bytes()
        require(len(raw) < 2 * 1024 * 1024, 'capture exceeded bound')
        samples = array('h', raw[:len(raw) - len(raw) % 4])
        capture_rate = 8000
        left = attacks(samples[::2], capture_rate)
        right = attacks(samples[1::2], capture_rate)
        errors = [min(abs(first - second) for first in left) for second in right if left and left[0] < second < left[-1]]
        if beat_ratio != (1, 1) and not tempo_only:
            errors = check_shared_cycles(left, right, outgoing_bpm, incoming_bpm, beat_ratio)
        if sparse:
            # Fold only a single missing observed attack; distant gaps cannot pass as alignment.
            period = 60 / outgoing_bpm
            errors = [min(error, abs(error - period)) for error in errors]
        print(outgoing_bpm, '→', incoming_bpm, 'TRIM:', trim, 'tones:', tonal, 'long:', long,
              'backend:', 'PipeWire' if pipewire else 'Pulse',
              'clock:', 'invalid-first' if invalid_clock else 'missing' if missing_clock else 'audio', 'peak MiB:', round(peak_rss, 2),
              'alignment samples:', len(errors),
              'alignment errors ms:', [round(error * 1000, 1) for error in errors] if trim and not tempo_only else 'not beat-matched', flush=True)
        if tonal:
            simultaneous = sum(abs(left) > 300 and abs(right) > 300
                               for left, right in zip(samples[::2], samples[1::2])) / capture_rate
            print("audible simultaneous seconds:", round(simultaneous, 3), flush=True)
            require(simultaneous > (18 if long else 2.5), "MIX did not produce the expected audible overlap")
            check_gain_ramp(samples, capture_rate)
        else:
            require(len(errors) >= 5, 'insufficient measured overlap')
        if not tonal and not sparse:
            require(all(after - before < 1.5 * 60 / outgoing_bpm
                        for before, after in zip(left, left[1:])),
                    'outgoing attack lost during tempo preparation')
        if trim and not tempo_only:
            # Onsets are measured on a 1 ms grid; subtracting float seconds can
            # otherwise reject an exactly 35 ms result as 35.000000000001 ms.
            maximum_error_ms = round(max(errors) * 1000)
            if maximum_error_ms > 35:
                print('first fixture attacks:', left[:12], right[:12], flush=True)
            require(maximum_error_ms <= 35, 'beat alignment exceeded 35 ms')
        if tempo_only:
            # Both songs must reach the shared midpoint after the opening ramp.
            steady = [beat for beat in right if right[0] + 8 < beat < left[-1] - 1.5]
            periods = [after - before for before, after in zip(steady, steady[1:])]
            require(len(periods) >= 8, 'insufficient rate-only overlap evidence')
            target_period = math.sqrt(60 * beat_ratio[0] / (outgoing_bpm * beat_ratio[1]) * 60 / incoming_bpm)
            require(abs(statistics.mean(periods) - target_period) < .008
                    and all(abs(period - target_period) < .015 for period in periods),
                    'incoming BPM did not reach the midpoint')
            print('measured incoming period during overlap:', round(sum(periods) / len(periods), 4), flush=True)
        if long and trim and not tonal:
            check_speed_return(left, right, outgoing_bpm, incoming_bpm, beat_ratio)
        final_periods = final_beat_periods(right, len(samples) / (2 * capture_rate))
        if tempo_only or beat_ratio != (1, 1):
            print('final incoming periods:', [round(value, 4) for value in final_periods], flush=True)
            require(len(final_periods) == 4
                    and all(abs(period - 60 / incoming_bpm) < .015 for period in final_periods)
                    and abs(statistics.median(final_periods) - 60 / incoming_bpm) < .008,
                    'incoming speed did not return to normal')
        elif not tonal:
            require(len(final_periods) >= 2 and abs(final_periods[-1] - 60 / incoming_bpm) < .02, 'incoming speed did not return to normal')
    require([snapshot(path) for path in (outgoing, incoming)] == before, 'mix changed original files')


def check_speed_return(left, right, outgoing_bpm, incoming_bpm, ratio):
    """Require late onset and continued rate recovery after handover, rejecting an immediate unity reset."""
    normal_period = 60 / incoming_bpm
    matched_period = math.sqrt(60 * ratio[0] / (outgoing_bpm * ratio[1]) * normal_period)
    difference = normal_period - matched_period
    if abs(difference) < normal_period * .02:
        return
    steady_periods = [after - before for before, after in zip(right, right[1:])
                      if left[-1] - 4.5 < (before + after) / 2 < left[-1] - 2]
    require(len(steady_periods) >= 2, 'insufficient pre-return evidence')
    require(abs(statistics.mean(steady_periods) - matched_period) < .01,
            'tempo return began too early')
    # The middle three seconds of the six-second return provide enough beats
    # even for the slow side of a 3:1 ratio, while excluding completed recovery.
    later_periods = [after - before for before, after in zip(right, right[1:])
                     if left[-1] + 1.5 < (before + after) / 2 < left[-1] + 4.5]
    require(len(later_periods) >= 2, 'insufficient post-handover evidence')
    # A stretch window can move one attack without changing the average tempo.
    # Average consecutive periods so window-sized steps cannot hide the ramp.
    recovered = (statistics.mean(later_periods) - matched_period) / difference
    print('post-handover tempo recovery fraction:', round(recovered, 3), flush=True)
    require(.15 < recovered < .9, 'tempo return did not continue smoothly after handover')


def check_shared_cycles(left, right, outgoing_bpm, incoming_bpm, ratio):
    """Require independently measured pulse rates and recurring shared-cycle coincidence, including the final ramp."""
    outgoing_count, incoming_count = ratio
    start = max(left[0], right[0]) + 1
    steady_end = left[-1] - 1.5
    midpoint_bpm = math.sqrt(outgoing_bpm * incoming_bpm * outgoing_count / incoming_count)
    for beats, period in [(left, 60 / midpoint_bpm),
                          (right, 60 * outgoing_count / (midpoint_bpm * incoming_count))]:
        steady = [beat for beat in beats if start + 7 < beat < steady_end]
        periods = [after - before for before, after in zip(steady, steady[1:])]
        require(len(periods) >= 8, 'insufficient shared-cycle rate evidence')
        require(abs(statistics.mean(periods) - period) < .008
                and all(abs(value - period) < .015 for value in periods),
                'measured pulse rate does not match ratio')
    overlap = [beat for beat in left if start < beat < right[-1]]
    candidates = []
    for phase in range(outgoing_count):
        shared = overlap[phase::outgoing_count]
        errors = [min(abs(outgoing - incoming) for incoming in right) for outgoing in shared]
        if len(errors) >= 5:
            candidates.append(errors)
    require(candidates, 'insufficient shared-cycle phase evidence')
    best = min(candidates, key=max)
    print('shared beat ratio:', f'{outgoing_count}:{incoming_count}', flush=True)
    return best


def check_gain_ramp(samples, rate):
    """Measure actual constant-tone amplitudes through the fade, proving opposite gradual ramps and bounded summed gain."""
    size = rate // 10
    gains = []
    for start in range(0, len(samples) // 2 - size, size):
        block = samples[start * 2:(start + size) * 2]
        left = math.sqrt(sum(value * value for value in block[::2]) / size) / (8000 / math.sqrt(2))
        right = math.sqrt(sum(value * value for value in block[1::2]) / size) / (8000 / math.sqrt(2))
        if left > .1 and right > .1:
            gains.append((left, right))
    require(len(gains) >= 20, 'missing gradual amplitude envelope')
    require(gains[0][0] - gains[-1][0] > .5, 'outgoing volume did not decrease')
    require(gains[-1][1] - gains[0][1] > .5, 'incoming volume did not increase')
    require(all(abs(left + right - 1) < .08 for left, right in gains), 'unexpected mix gain dip or boost')
    require(all(after[0] <= before[0] + .03 and after[1] >= before[1] - .03
                for before, after in zip(gains, gains[1:])), 'mix gains did not progress smoothly')
    print('measured gain ramp:', tuple(round(value, 2) for value in gains[0]),
          '→', tuple(round(value, 2) for value in gains[-1]), flush=True)


def main():
    """Check slower, faster and unchanged tempo on separate private audio graphs and immutable inputs."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scenario', choices=('all', 'short', 'no-trim', 'tones', 'long', 'pipewire', 'invalid-clock', 'sparse', 'tempo-only', 'wide-tempo', 'ratios', 'smooth-return'), default='all')
    args = parser.parse_args()
    cases = [(120, 124, False, True, False), (124, 120, False, True, False),
             (120, 120, False, True, False), (120, 120, True, True, False),
             (120, 148, False, False, False), (120, 148, False, False, True),
             (120, 124, False, True, False, True), (120, 148, False, False, True, True),
             (120, 124, False, True, False, False, True, 0),
             (120, 124, False, True, False, False, False, 0, True),
             (120, 124, False, True, False, True, False, None, False, True),
             (120, 124, False, True, False, True, False, None, False, False, True),
             (120, 142, False, True, False), (142, 120, False, True, False)]
    ratio_cases = [(120, 220, (1, 2)), (220, 120, (2, 1)),
                   (80, 220, (1, 3)), (220, 80, (3, 1)),
                   (120, 174, (2, 3)), (174, 120, (3, 2))]
    cases += [(outgoing, incoming, False, True, False, True, False, None, False, False, False, ratio)
              for outgoing, incoming, ratio in ratio_cases]
    cases += [(80, 220, False, True, False, True, False, None, False, False, True, (1, 3))]
    cases += [(120, 142, False, True, False, True), (142, 120, False, True, False, True)]
    if args.scenario == 'smooth-return':
        cases = cases[21:] + cases[16:18]
    elif args.scenario == 'ratios':
        cases = cases[14:21]
    elif args.scenario == 'wide-tempo':
        cases = cases[12:14]
    elif args.scenario == 'tempo-only':
        cases = cases[11:12]
    elif args.scenario == 'sparse':
        cases = cases[10:11]
    elif args.scenario == 'invalid-clock':
        cases = cases[9:10]
    elif args.scenario == 'pipewire':
        cases = cases[8:9]
    elif args.scenario == 'long':
        cases = cases[6:8]
    elif args.scenario == 'short':
        cases = cases[:6]
    elif args.scenario != 'all':
        cases = [cases[4 if args.scenario == 'no-trim' else 5]]
    binary = build_worker()
    with tempfile.TemporaryDirectory(prefix='resolibre-mix-') as temporary:
        for index, case in enumerate(cases):
            directory = Path(temporary) / str(index)
            directory.mkdir(mode=0o700)
            trial(directory, binary, *case)
    print('PASS: audible overlap, applicable beat timing, completed tempo recovery and preserved originals')


if __name__ == '__main__':
    main()
