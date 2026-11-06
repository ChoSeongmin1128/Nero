import copy
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
import wave

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import protocol
import runner


class EvaluationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nero-eval-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.audio = self.root / "audio.wav"
        with wave.open(str(self.audio), "wb") as audio:
            audio.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
            audio.writeframes(b"\0\0" * 16000)
        self.reference = self.root / "reference.json"
        self.reference.write_text(json.dumps({"text": "용어 회의", "entities": ["용어", "비발화이름"]}), encoding="utf-8")
        self.context = self.root / "context.json"
        self.context.write_text(json.dumps({"terms": ["용어"], "corrections": [{"source": "용오", "replacement": "용어"}]}), encoding="utf-8")
        self.model = self.root / "asr-model"
        self.model.mkdir()
        (self.model / "config.json").write_text("{}")
        self.gguf = self.root / "synthetic.gguf"
        self.gguf.write_bytes(b"synthetic model bytes, never loaded")
        self.config = {
            "schema_version": 1, "output_root": str(self.root / "runs"), "timeout_seconds": 5,
            "models": {
                "qwen3-asr-1.7b-ko": {"path": str(self.model), "runtime": {"python": sys.executable}},
                "gemma-4-e4b-q4": {"path": str(self.gguf), "runtime": {"python": sys.executable, "executable": sys.executable}, "source": {"repository": "synthetic/test", "revision": "f" * 40, "file": self.gguf.name, "sha256": hashlib.sha256(self.gguf.read_bytes()).hexdigest()}},
            },
            "cases": [{"id": "sample", "audio": str(self.audio), "reference": str(self.reference), "context": str(self.context)}],
            "experiments": [{"id": "asr", "model": "qwen3-asr-1.7b-ko", "track": "asr", "context": "none"}, {"id": "fix", "model": "gemma-4-e4b-q4", "track": "postprocess", "context": "dictionary", "baseline": "asr"}],
        }
        self.config_path = self.root / "config.json"

    def load(self):
        self.config_path.write_text(json.dumps(self.config))
        return protocol.load_config(self.config_path)

    def fake_worker(self, completion="complete", fail=False, scoring_supported=True, segments=None):
        script = self.root / "fake-worker.py"
        script.write_text('''import json, sys
from pathlib import Path
request = json.loads(Path(sys.argv[-1]).read_text())
assert 'reference' not in request and 'entities' not in request
if '--doctor' in sys.argv:
    print(json.dumps({'installed': True}))
    sys.exit(0)
if FAIL:
    sys.exit(3)
text = '용오 회의'
if request['task'] == 'postprocess':
    text = request['text']
    for item in reversed(request['candidates']):
        text = text[:item['start']] + item['replacement'] + text[item['end']:]
payload = {'text': text, 'segments': SEGMENTS, 'completion': {'status': COMPLETION}, 'metadata': {'decoding': {'temperature': 0}, 'text_scoring_supported': SCORING}}
(Path(request['output_dir']) / 'result.json').write_text(json.dumps(payload))
'''.replace("FAIL", repr(fail)).replace("COMPLETION", repr(completion)).replace("SEGMENTS", repr(segments or [])).replace("SCORING", repr(scoring_supported)), encoding="utf-8")
        return script

    def test_plan_and_doctor_never_start_inference(self):
        config, profiles = self.load()
        fake = self.fake_worker(fail=True)
        with patch.object(runner, "ADAPTER", fake):
            self.assertTrue(all(row["status"] == "ready_for_explicit_run" for row in runner.plan(config, profiles, doctor=True)))
        self.assertFalse(Path(config["output_root"]).exists())

    def test_end_to_end_reference_isolation_and_report(self):
        config, profiles = self.load()
        with patch.object(runner, "ADAPTER", self.fake_worker()):
            directory, rows = runner.run(config, profiles)
        self.assertEqual([row["status"] for row in rows], ["scored", "scored"])
        self.assertLess(rows[1]["metrics"]["correction"]["strict_cer_delta"], 0)
        self.assertEqual(rows[0]["metrics"]["entities"]["reference_labels"], 1)
        self.assertEqual(rows[0]["metrics"]["diarization"]["status"], "not_scored")
        request = protocol.read_json(directory / "sample" / "asr" / "request.json")
        self.assertNotIn("reference", request)
        self.assertNotIn(str(self.reference), json.dumps(request))
        self.assertNotIn("비발화이름", json.dumps(request, ensure_ascii=False))
        self.assertTrue((directory / "report" / "report.html").is_file())
        self.assertEqual((directory / "results.json").stat().st_mode & 0o777, 0o600)
        self.assertEqual((directory / "sample" / "asr" / "worker").stat().st_mode & 0o777, 0o700)

    def test_incomplete_asr_never_qualifies_correction(self):
        config, profiles = self.load()
        with patch.object(runner, "ADAPTER", self.fake_worker(completion="returned")):
            _, rows = runner.run(config, profiles)
        self.assertTrue(all(row["status"] == "unqualified" for row in rows))
        self.assertTrue(all(row["full_input_completed"] is False for row in rows))

    def test_failure_preserves_receipt_and_blocks_dependent_model(self):
        config, profiles = self.load()
        with patch.object(runner, "ADAPTER", self.fake_worker(fail=True)):
            directory, rows = runner.run(config, profiles)
        self.assertEqual([row["status"] for row in rows], ["failed", "not_scored"])
        self.assertTrue((directory / "sample" / "asr" / "adapter.log").is_file())
        self.assertIn("baseline", rows[1]["error"])

    def test_wrong_model_hash_blocks_execution(self):
        config, profiles = self.load()
        self.gguf.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            runner.model_fingerprint(profiles["gemma-4-e4b-q4"])

    def test_unparsed_text_is_not_scored_or_corrected(self):
        config, profiles = self.load()
        with patch.object(runner, "ADAPTER", self.fake_worker(scoring_supported=False)):
            _, rows = runner.run(config, profiles)
        self.assertEqual([row["status"] for row in rows], ["unqualified", "not_scored"])
        self.assertEqual(rows[0]["metrics"]["text"]["status"], "not_scored")

    def test_invalid_metric_input_cannot_leave_scored_status(self):
        config, profiles = self.load()
        invalid = [{"start": 0, "end": .5, "speaker": 99}]
        with patch.object(runner, "ADAPTER", self.fake_worker(segments=invalid)):
            _, rows = runner.run(config, profiles)
        self.assertEqual(rows[0]["status"], "failed")
        self.assertIn("speaker", rows[0]["error"])

    def test_no_candidate_does_not_run_correction(self):
        self.context.write_text(json.dumps({"terms": ["용어"], "corrections": []}))
        config, profiles = self.load()
        with patch.object(runner, "ADAPTER", self.fake_worker()):
            directory, rows = runner.run(config, profiles)
        self.assertEqual(rows[1]["status"], "not_scored")
        self.assertFalse((directory / "sample" / "fix" / "adapter.log").exists())

    def test_rejects_ambiguous_and_unsafe_configuration(self):
        cases = [
            lambda c: c["cases"].append(copy.deepcopy(c["cases"][0])),
            lambda c: c["experiments"][1].update(baseline="fix"),
            lambda c: c["cases"][0].update(id="../escape"),
            lambda c: c.update(output_root=str(self.model / "output")),
            lambda c: c["models"]["gemma-4-e4b-q4"].update(decoding={"ignored_flag": True}),
        ]
        original = copy.deepcopy(self.config)
        for mutate in cases:
            self.config = copy.deepcopy(original)
            mutate(self.config)
            with self.assertRaises((ValueError, KeyError)):
                self.load()

    def test_no_dictionary_file_is_explicitly_blocked(self):
        del self.config["cases"][0]["context"]
        config, profiles = self.load()
        rows = runner.plan(config, profiles)
        self.assertEqual(rows[1]["status"], "blocked")
        self.assertIn("missing local context", rows[1]["issues"])

    def test_candidate_offsets_and_overlaps(self):
        context = protocol.load_context(self.context)
        candidates = protocol.candidates("용오 회의 용오", context)
        self.assertEqual([c["start"] for c in candidates], [0, 6])
        context["corrections"].append({"source": "용오 회의", "replacement": "용어 회의"})
        with self.assertRaisesRegex(ValueError, "overlapping"):
            protocol.candidates("용오 회의", context)

    @unittest.skipUnless(os.name == "posix", "POSIX process group test")
    def test_timeout_kills_owned_child_group(self):
        child = self.root / "child.py"
        marker = self.root / "child-terminated"
        child.write_text("import signal,time,sys\nfrom pathlib import Path\ndef stop(*a):\n Path(sys.argv[1]).write_text('terminated')\n raise SystemExit(0)\nsignal.signal(signal.SIGTERM,stop)\nwhile True: time.sleep(.01)\n")
        parent = self.root / "parent.py"
        parent.write_text("import subprocess,sys,time\nsubprocess.Popen([sys.executable,sys.argv[1],sys.argv[2]])\nwhile True: time.sleep(.01)\n")
        receipt = runner.execute([sys.executable, str(parent), str(child), str(marker)], self.root, self.root / "timeout.log", .5)
        self.assertEqual(receipt["status"], "timeout")
        self.assertTrue(marker.exists())

    @unittest.skipUnless(os.name == "posix", "POSIX cancellation test")
    def test_external_sigterm_cleans_worker_group(self):
        ready = self.root / "ready"
        marker = self.root / "terminated"
        worker = self.root / "worker.py"
        worker.write_text("import signal,time,sys\nfrom pathlib import Path\ndef stop(*a):\n Path(sys.argv[2]).write_text('terminated')\n raise SystemExit(0)\nsignal.signal(signal.SIGTERM,stop)\nPath(sys.argv[1]).write_text('ready')\nwhile True: time.sleep(.01)\n")
        harness = self.root / "harness.py"
        harness.write_text("import signal,sys\nfrom pathlib import Path\nsys.path.insert(0,sys.argv[1])\nfrom runner import execute,cancel_from_signal\nsignal.signal(signal.SIGTERM,cancel_from_signal)\nexecute([sys.executable,sys.argv[2],sys.argv[3],sys.argv[4]],Path(sys.argv[5]),Path(sys.argv[5])/'worker.log',30)\n")
        with open(self.root / "harness.log", "wb") as log:
            proc = subprocess.Popen([sys.executable, "-B", str(harness), str(protocol.HERE), str(worker), str(ready), str(marker), str(self.root)], stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 5
                while not ready.exists() and time.monotonic() < deadline and proc.poll() is None:
                    time.sleep(.01)
                self.assertTrue(ready.exists())
                proc.terminate()
                proc.wait(timeout=7)
                self.assertTrue(marker.exists())
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait()


if __name__ == "__main__":
    unittest.main()
