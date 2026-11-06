import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import types
from unittest.mock import patch
import unittest
import wave

SCRIPT = Path(__file__).resolve().parents[1] / 'adapters' / 'adapter.py'
spec = importlib.util.spec_from_file_location('adapter', SCRIPT)
adapter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adapter)


class AdapterTests(unittest.TestCase):
    def test_request_rejects_reference_and_wrong_task(self):
        request = {'schema_version': 1, 'adapter': 'qwen', 'task': 'asr', 'model': {'path': '/local'}, 'reference': 'private'}
        with self.assertRaisesRegex(ValueError, 'Unexpected request fields'):
            adapter.validate_request(request)
        del request['reference']
        request['task'] = 'postprocess'
        with self.assertRaisesRegex(ValueError, 'Task does not match'):
            adapter.validate_request(request)

    def test_server_response_is_preserved_before_json_parsing(self):
        class Response:
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def read(self): return b'invalid json response'
        class Opener:
            def open(self, *args, **kwargs): return Response()
        with tempfile.TemporaryDirectory() as d, patch.object(adapter.urllib.request, 'build_opener', return_value=Opener()):
            native = Path(d) / 'native.json'
            with self.assertRaises(json.JSONDecodeError):
                adapter.local_request(1, 'key', '/v1/chat/completions', {}, native)
            self.assertEqual(native.read_bytes(), b'invalid json response')

    def test_unsupported_context_and_decoding_are_rejected(self):
        request = {'schema_version': 1, 'adapter': 'moss', 'task': 'asr', 'model': {'path': '/local'}, 'context': {'terms': ['term']}}
        with self.assertRaisesRegex(ValueError, 'dictionary context'):
            adapter.validate_request(request)
        request['context'] = {}
        request['decoding'] = {'temperature': 0}
        with self.assertRaisesRegex(ValueError, 'Unsupported decoding'):
            adapter.validate_request(request)
        request.update(adapter='qwen', decoding={'temperature': 0})
        with self.assertRaisesRegex(ValueError, 'Unsupported decoding'):
            adapter.validate_request(request)

    def test_decisions_preserve_original_and_require_exact_ids(self):
        text = 'alpha beta'
        candidates = [{'id': 'a', 'start': 0, 'end': 5, 'original': 'alpha', 'replacement': 'A'}, {'id': 'b', 'start': 6, 'end': 10, 'original': 'beta', 'replacement': 'B'}]
        adapter.validate_candidates(text, candidates)
        result = adapter.apply_decisions(text, candidates, {'decisions': [{'id': 'a', 'decision': 'accept'}, {'id': 'b', 'decision': 'uncertain'}]})
        self.assertEqual(result, 'A beta')
        with self.assertRaises(ValueError):
            adapter.apply_decisions(text, candidates, {'decisions': [{'id': 'a', 'decision': 'accept'}, {'id': 'a', 'decision': 'keep'}]})

    def test_rejects_overlapping_and_unmatched_spans(self):
        candidates = [{'id': 'a', 'start': 0, 'end': 3, 'original': 'abc', 'replacement': 'x'}, {'id': 'b', 'start': 2, 'end': 4, 'original': 'cd', 'replacement': 'y'}]
        with self.assertRaises(ValueError):
            adapter.validate_candidates('abcd', candidates)
        candidates[0]['original'] = 'wrong'
        with self.assertRaises(ValueError):
            adapter.validate_candidates('abcd', candidates)

    def test_doctor_does_not_load_models(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            req = {'schema_version': 1, 'adapter': 'qwen', 'task': 'asr', 'model': {'path': str(root)}}
            request = root / 'request.json'
            request.write_text(json.dumps(req))
            run = subprocess.run([sys.executable, '-B', str(SCRIPT), '--doctor', str(request)], capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertTrue(json.loads(run.stdout)['model_exists'])

    def test_moss_checks_native_output_against_raw_and_coverage(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            audio, exe = root / 'audio.wav', root / 'worker'
            exe.write_bytes(b'synthetic')
            with wave.open(str(audio), 'wb') as stream:
                stream.setnchannels(1); stream.setsampwidth(2); stream.setframerate(16000)
                stream.writeframes(b'\0\0' * 16000)
            evidence = {'promptTokens': 64, 'maxTokens': 65536, 'effectiveMaxTokens': 65536, 'contextSize': 131072, 'generationTokens': 12, 'terminationReason': 'eos', 'terminalTokenId': 151645, 'configuredEosTokenIds': [151645], 'tokenizerEosTokenId': 151645, 'inputAudioSamples': 16000, 'featureChunkCount': 1, 'featureTokenLengths': [13], 'audioTokenCount': 13, 'audioEmbeddingCount': 13, 'metadataSchemaVersion': 2, 'eosPolicy': 'model-generation-config+tokenizer-v1', 'samplingPolicy': 'greedy-argmax-v1'}
            chunk = dict(evidence, index=0, start=0, end=1, generationCount=1, parsedCompletely=True, hitTokenLimit=False, rawText='[0][S1]hello[1]', generationMetadata=[evidence])
            data = dict(evidence, runtime='mlx-audio-swift-v0.1.3+nero-moss-whole-v2', chunkingPolicy='whole-recording-v1', speakerScope='recording', generationCount=1, generationMetadata=[evidence], inputAudioFrames=16000, processedAudioFrames=16000, hitTokenLimit=False, hasUnparsedText=False, audioDurationSeconds=1, chunkDurationSeconds=1, chunks=[chunk], segments=[{'start': 0, 'end': 1, 'text': '[S1] hello', 'speaker_id': 'S1', 'asr_chunk': 0}], text='[S1] hello')
            request = {'adapter': 'moss', 'model': {'path': str(root)}, 'runtime': {'executable': str(exe)}, 'audio_path': str(audio)}
            def fake_run(args, **kwargs):
                Path(args[2]).write_text(json.dumps(data))
            with patch.object(adapter.subprocess, 'run', side_effect=fake_run):
                result = adapter.moss(request, root)
                self.assertEqual(result['completion']['status'], 'complete')
                self.assertEqual(result['text'], 'hello')
                self.assertEqual(result['segments'][0]['speaker'], 'S1')
                self.assertEqual(result['metadata']['decoding']['max_new_tokens'], 65536)
            evidence['samplingPolicy'] = chunk['samplingPolicy'] = data['samplingPolicy'] = 'greedy'
            with patch.object(adapter.subprocess, 'run', side_effect=fake_run), self.assertRaisesRegex(ValueError, 'EOS or feature'):
                adapter.moss(request, root)
            evidence['samplingPolicy'] = chunk['samplingPolicy'] = data['samplingPolicy'] = 'greedy-argmax-v1'
            evidence['eosPolicy'] = chunk['eosPolicy'] = data['eosPolicy'] = 'wrong'
            with patch.object(adapter.subprocess, 'run', side_effect=fake_run), self.assertRaisesRegex(ValueError, 'EOS or feature'):
                adapter.moss(request, root)
            evidence['eosPolicy'] = chunk['eosPolicy'] = data['eosPolicy'] = 'model-generation-config+tokenizer-v1'
            data['segments'][0]['text'] = 'changed'
            with patch.object(adapter.subprocess, 'run', side_effect=fake_run), self.assertRaisesRegex(ValueError, 'disagree'):
                adapter.moss(request, root)

    def test_moss_rejects_audio_over_90_minutes_before_worker(self):
        class FakeWave:
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def getnchannels(self): return 1
            def getframerate(self): return 16000
            def getnframes(self): return 90 * 60 * 16000 + 1
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            audio, exe = root / 'audio.wav', root / 'worker'
            audio.touch(); exe.touch()
            req = {'adapter': 'moss', 'model': {'path': str(root)}, 'runtime': {'executable': str(exe)}, 'audio_path': str(audio)}
            with patch.object(adapter.wave, 'open', return_value=FakeWave()), patch.object(adapter.subprocess, 'run') as run, self.assertRaisesRegex(ValueError, '90 minutes'):
                adapter.moss(req, root)
            run.assert_not_called()

    def test_qwen_uses_local_runtime_and_preserves_returned_text(self):
        seen = {}
        class Model:
            @classmethod
            def from_pretrained(cls, model, **kwargs):
                seen.update(kwargs)
                return cls()
            def transcribe(self, **kwargs):
                seen.update(kwargs)
                return [types.SimpleNamespace(text='synthetic', language='Korean')]
        with tempfile.TemporaryDirectory() as d, patch.dict(sys.modules, {'qwen_asr': types.SimpleNamespace(Qwen3ASRModel=Model)}):
            root = Path(d)
            audio = root / 'synthetic.wav'
            audio.write_bytes(b'synthetic')
            request = {'adapter': 'qwen', 'model': {'path': str(root)}, 'audio_path': str(audio), 'context': {'terms': ['term']}}
            result = adapter.qwen(request, root)
            self.assertTrue(seen['local_files_only'])
            self.assertEqual(seen['max_inference_batch_size'], 1)
            self.assertEqual(result['metadata']['runtime']['max_inference_batch_size'], 1)
            self.assertEqual(seen['context'], 'term')
            self.assertEqual(result['text'], 'synthetic')
            self.assertEqual(result['completion']['status'], 'returned')

    def test_correction_manages_fake_runtime(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            model = root / 'model.gguf'
            model.write_bytes(b'synthetic')
            server = root / 'server'
            server.write_bytes(b'synthetic')
            class Socket:
                def __enter__(self): return self
                def __exit__(self, *args): pass
                def bind(self, address): self.address = address
                def getsockname(self): return ('127.0.0.1', 1234)
            class Process:
                stopped = False
                def poll(self): return None
                def terminate(self): self.stopped = True
                def wait(self, timeout=None): return 0
            process = Process()
            response = {'choices': [{'finish_reason': 'stop', 'message': {'content': json.dumps({'decisions': [{'id': 'a', 'decision': 'accept'}]})}}]}
            request = {'adapter': 'correction', 'model': {'path': str(model)}, 'runtime': {'executable': str(server)}, 'text': 'alpha beta', 'candidates': [{'id': 'a', 'start': 0, 'end': 5, 'original': 'alpha', 'replacement': 'A'}]}
            with patch.object(adapter.socket, 'socket', return_value=Socket()), patch.object(adapter.subprocess, 'Popen', return_value=process) as popen, patch.object(adapter, 'local_request', side_effect=[None, response]) as calls:
                result = adapter.correction(request, root)
            self.assertEqual(result['text'], 'A beta')
            self.assertTrue(process.stopped)
            args = popen.call_args.args[0]
            for flag, value in (('--reasoning', 'off'), ('--ctx-size', '8192'), ('--parallel', '1'), ('--chat-template-kwargs', '{"enable_thinking": false}')):
                self.assertEqual(args[args.index(flag) + 1], value)
            self.assertIn('--jinja', args)
            self.assertIn('--no-context-shift', args)
            body = calls.call_args_list[1].args[3]
            self.assertEqual(body['chat_template_kwargs'], {'enable_thinking': False})
            self.assertEqual((body['temperature'], body['top_k'], body['max_tokens']), (0, 1, 768))
            schema = body['response_format']['json_schema']
            self.assertTrue(schema['strict'])
            self.assertEqual(schema['schema']['properties']['decisions']['items']['properties']['id']['enum'], ['a'])
            self.assertEqual(result['metadata']['runtime']['reasoning'], 'off')

    def test_correction_rejects_empty_content(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / 'model.gguf').write_bytes(b'synthetic')
            (root / 'server').write_bytes(b'synthetic')
            class Process:
                def poll(self): return None
                def terminate(self): pass
                def wait(self, timeout=None): return 0
            response = {'choices': [{'finish_reason': 'stop', 'message': {'content': '', 'reasoning_content': 'thinking'}}]}
            request = {'adapter': 'correction', 'model': {'path': str(root / 'model.gguf')}, 'runtime': {'executable': str(root / 'server')}, 'text': 'alpha beta', 'candidates': [{'id': 'a', 'start': 0, 'end': 5, 'original': 'alpha', 'replacement': 'A'}]}
            with patch.object(adapter.subprocess, 'Popen', return_value=Process()), patch.object(adapter, 'local_request', side_effect=[None, response]):
                with self.assertRaisesRegex(ValueError, 'no content'):
                    adapter.correction(request, root)

    def test_correction_output_must_fit_server_context(self):
        request = {'schema_version': 1, 'adapter': 'correction', 'task': 'postprocess', 'model': {}, 'decoding': {'max_new_tokens': 8192}}
        with self.assertRaisesRegex(ValueError, 'server context'):
            adapter.validate_request(request)

    def test_correction_sampling_overrides_reach_request(self):
        body = adapter.correction_body([], [{'id': 'a'}], {'temperature': 0.7, 'top_p': 0.8, 'top_k': 20, 'min_p': 0, 'presence_penalty': 1.5, 'seed': 2})
        self.assertEqual((body['temperature'], body['top_p'], body['top_k'], body['presence_penalty'], body['seed']), (0.7, 0.8, 20, 1.5, 2))
        self.assertEqual(body['chat_template_kwargs'], {'enable_thinking': False})
        request = {'schema_version': 1, 'adapter': 'correction', 'task': 'postprocess', 'model': {}, 'decoding': {'top_k': -1}}
        with self.assertRaisesRegex(ValueError, 'top_k'):
            adapter.validate_request(request)

    @unittest.skipUnless(importlib.util.find_spec('numpy'), 'chunked Qwen ASR needs numpy from the Qwen runtime')
    def test_qwen_chunked_transcription_covers_input(self):
        import wave as wavemod
        import numpy as np
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / 'model').mkdir()
            audio = root / 'a.wav'
            with wavemod.open(str(audio), 'wb') as w:
                w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000); w.writeframes(b'\x01\x00' * 16000 * 5)
            seen = {}
            class Result:
                def __init__(self, text): self.text, self.language = text, 'Korean'
            class Model:
                @classmethod
                def from_pretrained(cls, path, **kwargs): return cls()
                def transcribe(self, audio, language, context):
                    seen['audio'] = audio
                    return [Result('part%d' % i) for i in range(len(audio))]
            def split(wav, sr, max_chunk_sec):
                step = int(sr * max_chunk_sec)
                return [(wav[i:i + step], i / sr) for i in range(0, len(wav), step)]
            fake = types.ModuleType('qwen_asr'); fake.Qwen3ASRModel = Model
            inference = types.ModuleType('qwen_asr.inference'); utils = types.ModuleType('qwen_asr.inference.utils'); utils.split_audio_into_chunks = split
            with patch.dict(sys.modules, {'qwen_asr': fake, 'qwen_asr.inference': inference, 'qwen_asr.inference.utils': utils}):
                result = adapter.qwen({'adapter': 'qwen', 'model': {'path': str(root / 'model')}, 'audio_path': str(audio), 'decoding': {'chunk_seconds': 2}}, root)
            self.assertEqual(result['text'], 'part0 part1 part2')
            self.assertEqual(result['metadata']['chunking']['chunks'], 3)
            self.assertEqual(sum(len(c) for c, _ in seen['audio']), 16000 * 5)
        pcm = np.arange(10, dtype=np.float32) + 1
        self.assertEqual(adapter.verify_chunk_coverage(pcm, [(pcm[:8], 0.0), (np.concatenate([pcm[8:], np.zeros(4, np.float32)]), 8 / 16000)]), 1)
        with self.assertRaisesRegex(ValueError, 'cover'):
            adapter.verify_chunk_coverage(pcm, [(pcm[:8], 0.0), (pcm[9:], 9 / 16000)])
        request = {'schema_version': 1, 'adapter': 'qwen', 'task': 'asr', 'model': {}, 'decoding': {'chunk_seconds': 0.5}}
        with self.assertRaisesRegex(ValueError, 'chunk_seconds'):
            adapter.validate_request(request)

    @unittest.skipUnless(importlib.util.find_spec('numpy'), 'chunked Qwen ASR needs numpy from the Qwen runtime')
    def test_qwen_pause_boundary_pad_and_gain(self):
        import wave as wavemod
        import numpy as np
        sr = 16000
        rng = np.random.default_rng(0)
        speech = lambda s: (rng.standard_normal(int(s * sr)) * 0.1).astype(np.float32)
        quiet = lambda s: (rng.standard_normal(int(s * sr)) * 0.0005).astype(np.float32)
        pcm = np.concatenate([speech(7), quiet(0.15), speech(1.3), quiet(0.8), speech(4)])
        chunks = adapter.pause_split(pcm, sr, 8.0)
        self.assertEqual(len(chunks), 2)
        self.assertTrue(8.45 <= chunks[1][1] <= 9.25)
        self.assertEqual(adapter.verify_chunk_coverage(pcm, chunks), 0)
        raised = adapter.window_gain(np.concatenate([np.zeros(sr, np.float32), speech(1) * 0.2]), sr, -20)
        self.assertTrue((raised[:sr] == 0).all())
        self.assertAlmostEqual(20 * np.log10(np.sqrt(np.mean(raised[sr:sr + sr // 2] ** 2))), -20, delta=1.5)
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / 'model').mkdir()
            audio = root / 'a.wav'
            with wavemod.open(str(audio), 'wb') as w:
                w.setnchannels(1); w.setsampwidth(2); w.setframerate(sr); w.writeframes((pcm * 32767).astype('<i2').tobytes())
            seen = {}
            class Result:
                def __init__(self, text): self.text, self.language = text, 'Korean'
            class Model:
                @classmethod
                def from_pretrained(cls, path, **kwargs): return cls()
                def transcribe(self, audio, language, context):
                    seen['audio'] = audio
                    return [Result('part%d' % i) for i in range(len(audio))]
            fake = types.ModuleType('qwen_asr'); fake.Qwen3ASRModel = Model
            inference = types.ModuleType('qwen_asr.inference'); utils = types.ModuleType('qwen_asr.inference.utils'); utils.split_audio_into_chunks = None
            decoding = {'chunk_seconds': 8, 'chunk_boundary': 'pause', 'chunk_pad_seconds': 1, 'chunk_gain_db': -20}
            with patch.dict(sys.modules, {'qwen_asr': fake, 'qwen_asr.inference': inference, 'qwen_asr.inference.utils': utils}):
                result = adapter.qwen({'adapter': 'qwen', 'model': {'path': str(root / 'model')}, 'audio_path': str(audio), 'decoding': decoding}, root)
            self.assertEqual(result['text'], 'part0 part1')
            self.assertEqual(result['metadata']['chunking']['boundary'], 'pause')
            self.assertEqual([len(x) for x, _ in seen['audio']], [len(c) + 2 * sr for c, _ in chunks])
            self.assertTrue((seen['audio'][0][0][:sr] == 0).all())
        base = {'schema_version': 1, 'adapter': 'qwen', 'task': 'asr', 'model': {}}
        adapter.validate_request({**base, 'decoding': decoding})
        for bad, message in [({'chunk_boundary': 'pause'}, 'require chunk_seconds'), ({**decoding, 'chunk_boundary': 'vad'}, 'chunk_boundary'), ({**decoding, 'chunk_pad_seconds': 6}, 'chunk_pad_seconds'), ({**decoding, 'chunk_gain_db': 0}, 'chunk_gain_db')]:
            with self.assertRaisesRegex(ValueError, message):
                adapter.validate_request({**base, 'decoding': bad})

    def test_qwen_dtype_is_limited(self):
        request = {'schema_version': 1, 'adapter': 'qwen', 'task': 'asr', 'model': {}, 'runtime': {'dtype': 'bfloat16'}}
        adapter.validate_request(request)
        request['runtime']['dtype'] = 'int8'
        with self.assertRaisesRegex(ValueError, 'dtype'):
            adapter.validate_request(request)
        moss = {'schema_version': 1, 'adapter': 'moss', 'task': 'asr', 'model': {}, 'runtime': {'dtype': 'bfloat16'}}
        with self.assertRaisesRegex(ValueError, 'runtime fields'):
            adapter.validate_request(moss)

    def test_nemotron_fixed_server_limits_are_reported(self):
        metadata = adapter.base({'adapter': 'nemotron', 'model': {}})
        self.assertEqual(metadata['runtime']['gpu_memory_utilization'], 0.9)
        self.assertEqual(metadata['runtime']['max_model_len'], 8192)
        request = {'schema_version': 1, 'adapter': 'nemotron', 'task': 'asr', 'model': {}, 'decoding': {'max_new_tokens': 8193}}
        with self.assertRaisesRegex(ValueError, 'exceeds'):
            adapter.validate_request(request)

    def test_nemotron_output_markers_require_review(self):
        self.assertTrue(adapter.nemotron_has_markers('<|begin_of_text|>hello'))
        self.assertTrue(adapter.nemotron_has_markers('<spk:0>hello'))
        self.assertFalse(adapter.nemotron_has_markers('hello'))

    def test_backbone_is_rejected_before_server_start(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            model = root / 'backbone-64e-w4a16-g64'
            model.mkdir()
            with self.assertRaisesRegex(ValueError, 'speech checkpoint'):
                adapter.nemotron({'model': {'path': str(model)}}, root)


if __name__ == '__main__':
    unittest.main()
