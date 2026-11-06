import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import itertools
import random
import unittest
from unittest.mock import patch

from metrics import _assignment, correction_metrics, diarization_metrics, entity_metrics, text_metrics


class MetricsTest(unittest.TestCase):
    def test_text_policy_and_counts(self):
        result = text_metrics('가  나\n다', '가 나 라')
        self.assertEqual(result['normalized_cer']['substitutions'], 1)
        self.assertEqual(result['normalized_cer']['reference_length'], 5)
        self.assertEqual(result['normalized_wer']['value'], 1 / 3)
        self.assertGreater(result['strict_cer']['value'], result['normalized_cer']['value'])
        self.assertEqual(text_metrics('', '가')['strict_cer']['insertions'], 1)
        self.assertIsNone(text_metrics('', '가')['strict_cer']['value'])

    def test_correction_and_entities(self):
        result = correction_metrics('철수', '철소', '철수')
        self.assertEqual(result['strict_cer_delta'], -0.5)
        self.assertFalse(result['unchanged'])
        entities = entity_metrics('철수와 만남', '철수와 영희 만남', ['철수', '영희'])
        self.assertEqual(entities['accuracy'], 1)
        self.assertEqual(entities['hallucinated_labels'], ['영희'])

    def test_correction_when_large_input_is_unsupported(self):
        with patch("metrics._edits", return_value=None):
            self.assertIsNone(correction_metrics("가", "나", "다")["strict_cer_delta"])

    def test_assignment_against_permutations(self):
        random.seed(0)
        for n in range(1, 6):
            for _ in range(10):
                matrix = [[random.randrange(6) for _ in range(n)] for _ in range(n)]
                got = sum(matrix[i][j] for i, j in _assignment(matrix))
                expected = max(sum(matrix[i][p[i]] for i in range(n)) for p in itertools.permutations(range(n)))
                self.assertEqual(got, expected)
        matrix = [[int(i == j) for j in range(10)] for i in range(10)]
        self.assertEqual(sum(matrix[i][j] for i, j in _assignment(matrix)), 10)

    def test_diarization(self):
        reference = [{'start': 0, 'end': 2, 'speaker': 'a'}, {'start': 1, 'end': 3, 'speaker': 'b'}]
        hyp = [{'start': 0, 'end': 2, 'speaker': 'x'}, {'start': 1, 'end': 3, 'speaker': 'y'}, {'start': 3, 'end': 4, 'speaker': 'x'}]
        scored = diarization_metrics(reference, hyp)
        self.assertEqual(scored['reference_speaker_seconds'], 4)
        self.assertEqual(scored['der'], .25)
        self.assertEqual(diarization_metrics(reference, hyp, overlap_policy='exclude')['reference_speaker_seconds'], 2)
        self.assertEqual(diarization_metrics(reference, [], collar=0)['status'], 'not_scored')
        empty = diarization_metrics(reference, [], output_available=True)
        self.assertEqual(empty['status'], 'scored')
        self.assertEqual(empty['der'], 1)
        self.assertEqual(empty['miss_seconds'], 4)
        self.assertEqual(diarization_metrics([], hyp)['false_alarm_seconds'], 5)
        self.assertIsNone(diarization_metrics([], hyp)['der'])
        self.assertAlmostEqual(diarization_metrics(reference, hyp, collar=.25)['reference_speaker_seconds'], 2)


if __name__ == '__main__':
    unittest.main()
