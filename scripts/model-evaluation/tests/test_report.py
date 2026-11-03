import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import csv
import json
import os
import tempfile
import unittest
from report import write_report


class ReportTest(unittest.TestCase):
    def test_private_and_escaped_outputs(self):
        row = {'case_id': '<script>', 'track': 'asr', 'model': 'm', 'hardware': 'cpu', 'decoding': {}, 'status': 'unqualified', 'metrics': {'cer': .5}, 'experiment': 'e1', 'context': 'dictionary', 'execution': {'wall_seconds': 5, 'rtf_end_to_end': .5}}
        with tempfile.TemporaryDirectory() as temp:
            paths = write_report([row], Path(temp) / 'report')
            self.assertEqual(json.loads(Path(paths['report.json']).read_text())[0]['status'], 'unqualified')
            self.assertNotIn('<script></td>', Path(paths['report.html']).read_text())
            self.assertIn('&lt;script&gt;', Path(paths['report.html']).read_text())
            self.assertIn('rtf_end_to_end', Path(paths['report.html']).read_text())
            self.assertIn('dictionary', Path(paths['report.html']).read_text())
            self.assertEqual(oct(os.stat(paths['report.json']).st_mode & 0o777), '0o600')
            with open(paths['report.csv'], newline='') as handle:
                self.assertEqual(list(csv.DictReader(handle))[0]['case_id'], '<script>')


if __name__ == '__main__':
    unittest.main()
