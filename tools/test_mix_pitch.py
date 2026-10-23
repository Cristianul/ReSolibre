"""Independent negative controls for the MIX pitch and captured-tail analyzers."""

from array import array
import math
import unittest

from check_mix_pitch import check_tone
from check_mix import final_beat_periods


class PitchChecks(unittest.TestCase):
    """Ensure a passing capture cannot hide varispeed, flutter, silence or missing evidence."""

    def test_original_tone(self):
        """Accept both original fixture tones at an arbitrary phase."""
        for frequency in (55, 440):
            values = array('h', (round(8000 * math.sin(math.tau * frequency * index / 8000 + .7))
                                 for index in range(16000)))
            check_tone(values, 8000, frequency)

    def test_corrupted_tones(self):
        """Reject changed pitch, periodic phase wobble and a short dropout, not just average pitch."""
        for defect in ('speed', 'flutter', 'phase_jump', 'dropout', 'silence', 'short'):
            values = array('h')
            for index in range(16000):
                seconds = index / 8000
                phase = math.tau * 55 * seconds
                if defect == 'speed':
                    phase *= 1.02
                if defect == 'flutter':
                    phase += .4 * math.sin(math.tau * 6 * seconds)
                if defect == 'phase_jump' and seconds >= .75:
                    phase += .9
                value = round(8000 * math.sin(phase))
                if defect == 'silence' or (defect == 'dropout' and .5 < seconds < .6):
                    value = 0
                values.append(value)
            if defect == 'short':
                values = values[:1000]
            with self.subTest(defect=defect), self.assertRaises(ValueError):
                check_tone(values, 8000, 55)


class TailChecks(unittest.TestCase):
    """Keep shutdown fragments out of settled tempo evidence without hiding incomplete captures."""

    def test_complete_attacks_and_truncated_tail(self):
        """Retain complete pulses, reject the final truncated pre-echo and preserve evidence counts."""
        beats = [index * .5 for index in range(10)]
        self.assertEqual(final_beat_periods(beats, 5.0), [.5] * 4)
        self.assertEqual(final_beat_periods(beats + [4.98], 5.0), [.5] * 4)
        self.assertEqual(final_beat_periods([4.0, 4.5, 4.98], 5.0), [.5])
        self.assertEqual(final_beat_periods([4.98], 5.0), [])
        self.assertEqual(final_beat_periods([], 5.0), [])


if __name__ == '__main__':
    unittest.main()
