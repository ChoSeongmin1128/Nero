import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
REGISTRY = ROOT / 'scripts/model-evaluation/models.json'


class RegistryTests(unittest.TestCase):
    def test_profiles_and_capability_evidence(self):
        profiles = json.loads(REGISTRY.read_text())['models']
        self.assertEqual(len(profiles), 6)
        self.assertEqual(len({model['id'] for model in profiles}), 6)
        for model in profiles:
            self.assertEqual(set(model['capabilities']), {
                'transcribe', 'integrated_diarization', 'timestamps', 'context', 'postprocess'
            })
            for evidence in model['capabilities'].values():
                self.assertIn(evidence['status'], {'documented', 'claimed', 'verified', 'unverified'})
                self.assertTrue(evidence['note'])
            self.assertTrue(model['prerequisites'])

    def test_existing_pins_match_product_manifests(self):
        models = {m['id']: m for m in json.loads(REGISTRY.read_text())['models']}
        moss = json.loads((ROOT / 'crates/nero-core/resources/models/moss.json').read_text())
        gemma = json.loads((ROOT / 'apps/desktop/models/summary.json').read_text())
        for key, manifest in [('moss-transcribe-diarize', moss), ('gemma-4-e4b-q4', gemma)]:
            source = models[key]['source']
            self.assertEqual(source['repository'], manifest['repository'])
            self.assertEqual(source['revision'], manifest['revision'])
        self.assertEqual(models['gemma-4-e4b-q4']['source']['sha256'], gemma['files'][0]['sha256'])

    def test_unknown_conversions_are_not_guessed(self):
        models = {m['id']: m for m in json.loads(REGISTRY.read_text())['models']}
        for key in ('qwen3.5-4b-gguf', 'qwen3.5-9b-gguf'):
            source = models[key]['source']
            self.assertIsNone(source['conversion_repository'])
            self.assertIsNone(source['revision'])
            self.assertIsNone(source['file'])
            self.assertIsNone(source['sha256'])


if __name__ == '__main__':
    unittest.main()
