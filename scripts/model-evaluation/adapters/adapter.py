#!/usr/bin/env python3
import argparse
import base64
import importlib.util
import importlib.metadata
import json
import math
import os
import platform
import re
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import wave

# Same llama-server limits as the desktop summary runtime (summary.rs).
CORRECTION_CONTEXT = 8192
CORRECTION_OUTPUT = 768
QWEN_DTYPES = ('bfloat16', 'float16', 'float32')
SAMPLING_FIELDS = ('top_p', 'top_k', 'min_p', 'presence_penalty', 'repeat_penalty', 'seed')
QWEN_CHUNK_FIELDS = ('chunk_seconds', 'chunk_boundary', 'chunk_pad_seconds', 'chunk_gain_db')
QWEN_BOUNDARIES = ('energy', 'pause')


def fail(message):
    raise ValueError(message)


def path(value, kind):
    p = Path(value).expanduser().resolve(strict=True)
    if kind == 'file' and not p.is_file():
        fail('Expected a local file')
    if kind == 'dir' and not p.is_dir():
        fail('Expected a local directory')
    return p


def offline():
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    os.environ['HF_DATASETS_OFFLINE'] = '1'
    os.environ['DO_NOT_TRACK'] = '1'
    os.environ['VLLM_NO_USAGE_STATS'] = '1'


def write_json(p, value):
    with open(p, 'x', encoding='utf-8') as f:
        json.dump(value, f, ensure_ascii=False, allow_nan=False, indent=2)
        f.write('\n')


def base(request):
    model = request['model']
    kind = request['adapter']
    supplied = request.get('decoding', {})
    defaults = {'moss': {'max_new_tokens': 65536, 'temperature': 0, 'top_p': 1, 'min_chunk_duration': 0, 'repetition_context_size': 100}, 'qwen': {'max_new_tokens': 1024}, 'nemotron': {'max_new_tokens': 1024, 'temperature': 0, 'enable_thinking': False, 'skip_special_tokens': False}, 'correction': {'max_new_tokens': CORRECTION_OUTPUT, 'temperature': 0, 'top_k': 1, 'top_p': 1, 'seed': 0, 'enable_thinking': False}}
    decoding = {**defaults[kind], **supplied}
    distributions = {'qwen': ('qwen-asr', 'transformers', 'torch'), 'nemotron': ('vllm', 'nemo-toolkit'), 'moss': (), 'correction': ()}
    versions = {}
    for name in distributions[kind]:
        try:
            versions[name] = importlib.metadata.version(name)
        except importlib.metadata.PackageNotFoundError:
            versions[name] = None
    runtime = {k: v for k, v in request.get('runtime', {}).items() if k in ('python', 'executable', 'device_map', 'dtype', 'startup_timeout_seconds')}
    runtime['python_version'] = platform.python_version()
    runtime['adapter_python_package_versions'] = versions
    if kind == 'qwen':
        runtime['max_inference_batch_size'] = 1
    if kind == 'correction':
        runtime.update(ctx_size=CORRECTION_CONTEXT, parallel=1, context_shift=False, reasoning='off', chat_template_kwargs={'enable_thinking': False}, response_format='json_schema strict')
    if kind == 'nemotron':
        runtime['gpu_memory_utilization'] = 0.9
        runtime['max_model_len'] = 8192
        runtime['trust_remote_code'] = False
        runtime['compatibility'] = 'unverified; model card documents trust-remote-code, which is not enabled'
    return {'model': {k: model.get(k) for k in ('id', 'format', 'variant')}, 'runtime': runtime, 'decoding': decoding, 'context_used': False}




def moss(req, output):
    exe = path(req['runtime']['executable'], 'file')
    model = path(req['model']['path'], 'dir')
    audio = path(req['audio_path'], 'file')
    with wave.open(str(audio), 'rb') as w:
        if w.getnchannels() != 1 or w.getframerate() != 16000 or not 0 < w.getnframes() <= 90 * 60 * 16000:
            fail('MOSS requires nonempty mono 16 kHz WAV at most 90 minutes')
        frames = w.getnframes()
    native = output / 'native.json'
    subprocess.run([str(exe), str(audio), str(native), str(model)], check=True)
    data = json.loads(native.read_text(encoding='utf-8'))
    ev = data.get('generationMetadata', [])
    if (data.get('runtime') != 'mlx-audio-swift-v0.1.3+nero-moss-whole-v2' or data.get('chunkingPolicy') != 'whole-recording-v1' or data.get('speakerScope') != 'recording' or data.get('generationCount') != 1 or len(ev) != 1 or data.get('inputAudioFrames') != frames or data.get('processedAudioFrames') != frames or data.get('hitTokenLimit') is not False or data.get('hasUnparsedText') is not False):
        fail('MOSS whole-input completion evidence is incomplete')
    e = ev[0]
    lengths = [375] * (frames // 480000)
    if frames % 480000:
        lengths.append(math.ceil((frames % 480000) / 1280))
    if (e.get('metadataSchemaVersion') != 2 or e.get('eosPolicy') != 'model-generation-config+tokenizer-v1' or e.get('samplingPolicy') != 'greedy-argmax-v1' or e.get('terminationReason') != 'eos' or e.get('terminalTokenId') != 151645 or e.get('tokenizerEosTokenId') != 151645 or e.get('configuredEosTokenIds') != [151645] or e.get('inputAudioSamples') != frames or e.get('featureChunkCount') != len(lengths) or e.get('featureTokenLengths') != lengths or e.get('audioTokenCount') != sum(lengths) or e.get('audioEmbeddingCount') != sum(lengths)):
        fail('MOSS EOS or feature coverage evidence is incomplete')
    chunks = data.get('chunks', [])
    prompt, context, effective, tokens = (e.get(k) for k in ('promptTokens', 'contextSize', 'effectiveMaxTokens', 'generationTokens'))
    if (type(prompt) is not int or type(context) is not int or type(effective) is not int or type(tokens) is not int or prompt <= 0 or context != 131072 or e.get('maxTokens') != 65536 or effective <= 0 or effective != min(65536, context - prompt - 1) or not 0 <= tokens < effective or len(chunks) != 1):
        fail('MOSS generation budget evidence is invalid')
    chunk = chunks[0]
    duration = frames / 16000
    fields = ('promptTokens', 'maxTokens', 'effectiveMaxTokens', 'contextSize', 'generationTokens', 'terminationReason', 'terminalTokenId', 'configuredEosTokenIds', 'tokenizerEosTokenId', 'inputAudioSamples', 'featureChunkCount', 'featureTokenLengths', 'audioTokenCount', 'audioEmbeddingCount', 'metadataSchemaVersion', 'eosPolicy', 'samplingPolicy')
    if (data.get('audioDurationSeconds') != duration or data.get('chunkDurationSeconds', 0) < duration or chunk.get('index') != 0 or chunk.get('start') != 0 or chunk.get('end') != duration or chunk.get('generationCount') != 1 or chunk.get('parsedCompletely') is not True or chunk.get('hitTokenLimit') is not False or chunk.get('generationMetadata') != ev or any(data.get(k) != e.get(k) or chunk.get(k) != e.get(k) for k in fields)):
        fail('MOSS whole-recording evidence disagrees across output')
    if not isinstance(data.get('text'), str) or not isinstance(data.get('segments'), list) or not isinstance(chunk.get('rawText'), str):
        fail('Invalid MOSS output')
    pattern = re.compile(r'\[(\d+(?:[.,]\d+)?)\]\[(S\d+)\](.*?)\[(\d+(?:[.,]\d+)?)\]', re.DOTALL)
    raw = chunk['rawText']
    if pattern.sub('', raw).strip():
        fail('MOSS raw output contains unparsed text')
    expected = []
    for match in pattern.finditer(raw):
        start = float(match.group(1).replace(',', '.'))
        end = float(match.group(4).replace(',', '.'))
        speaker = match.group(2)
        body = match.group(3).strip()
        if not math.isfinite(start) or not math.isfinite(end) or start < 0 or end < start or end > duration + .5:
            fail('MOSS segment timing is invalid')
        if body:
            expected.append((start, end, '[' + speaker + '] ' + body, speaker))
    segments = data['segments']
    if len(segments) != len(expected):
        fail('MOSS segment count disagrees with raw output')
    for segment, expected_segment in zip(segments, expected):
        if not isinstance(segment, dict) or segment.get('asr_chunk') != 0 or (segment.get('start'), segment.get('end'), segment.get('text'), segment.get('speaker_id')) != expected_segment:
            fail('MOSS segments disagree with raw output')
    if data['text'] != '\n'.join(segment['text'] for segment in segments):
        fail('MOSS text disagrees with segments')
    normalized = [{'start': start, 'end': end, 'speaker': speaker, 'text': labeled[len(speaker) + 3:]} for start, end, labeled, speaker in expected]
    return {'text': '\n'.join(segment['text'] for segment in normalized), 'segments': normalized, 'native_payload_path': str(native), 'completion': {'status': 'complete', 'evidence': 'native EOS and full input feature coverage'}, 'metadata': base(req)}


def qwen(req, output):
    offline()
    from qwen_asr import Qwen3ASRModel
    model = path(req['model']['path'], 'dir')
    audio = path(req['audio_path'], 'file')
    decoder = req.get('decoding', {})
    kwargs = {'local_files_only': True, 'max_inference_batch_size': 1, 'max_new_tokens': decoder.get('max_new_tokens', 1024)}
    if req.get('runtime', {}).get('device_map'):
        kwargs['device_map'] = req['runtime']['device_map']
    if req.get('runtime', {}).get('dtype'):
        import torch
        kwargs['dtype'] = getattr(torch, req['runtime']['dtype'])
    instance = Qwen3ASRModel.from_pretrained(str(model), **kwargs)
    terms = req.get('context', {}).get('terms', [])
    context = ', '.join(terms)
    native = output / 'native.json'
    meta = base(req)
    meta['context_used'] = bool(terms)
    if 'chunk_seconds' not in decoder:
        results = instance.transcribe(audio=str(audio), language='Korean', context=context)
        if len(results) != 1 or not isinstance(results[0].text, str):
            fail('Unexpected Qwen ASR output')
        write_json(native, {'text': results[0].text, 'language': results[0].language})
        return {'text': results[0].text, 'segments': [], 'native_payload_path': str(native), 'completion': {'status': 'returned', 'evidence': 'runtime returned one result; EOS and input coverage not exposed'}, 'metadata': meta}
    import numpy as np
    from qwen_asr.inference.utils import split_audio_into_chunks
    with wave.open(str(audio), 'rb') as w:
        if w.getnchannels() != 1 or w.getframerate() != 16000 or w.getsampwidth() != 2 or not w.getnframes():
            fail('Chunked Qwen ASR requires nonempty mono 16 kHz PCM16 WAV')
        frames = w.getnframes()
        pcm = np.frombuffer(w.readframes(frames), dtype='<i2').astype(np.float32) / 32768.0
    seconds = float(decoder['chunk_seconds'])
    boundary = decoder.get('chunk_boundary', 'energy')
    chunks = pause_split(pcm, 16000, seconds) if boundary == 'pause' else split_audio_into_chunks(pcm, 16000, seconds)
    padded = verify_chunk_coverage(pcm, chunks)
    pad = np.zeros(round(decoder.get('chunk_pad_seconds', 0) * 16000), dtype=np.float32)
    inputs = [np.concatenate([pad, c, pad]) for c, _ in chunks]
    if 'chunk_gain_db' in decoder:
        inputs = [window_gain(x, 16000, decoder['chunk_gain_db']) for x in inputs]
    results = instance.transcribe(audio=[(x, 16000) for x in inputs], language=['Korean'] * len(chunks), context=[context] * len(chunks))
    if len(results) != len(chunks) or not all(isinstance(r.text, str) for r in results):
        fail('Unexpected Qwen ASR output')
    pieces = [{'offset_seconds': offset, 'samples': len(c), 'text': r.text, 'language': r.language} for (c, offset), r in zip(chunks, results)]
    text = ' '.join(piece['text'].strip() for piece in pieces if piece['text'].strip())
    write_json(native, {'text': text, 'chunks': pieces})
    policy = 'longest pause boundaries' if boundary == 'pause' else 'qwen_asr split_audio_into_chunks low-energy boundaries'
    meta['chunking'] = {'policy': policy, 'boundary': boundary, 'target_chunk_seconds': decoder['chunk_seconds'], 'pad_seconds': decoder.get('chunk_pad_seconds', 0), 'gain_db': decoder.get('chunk_gain_db'), 'chunks': len(chunks), 'input_frames': frames, 'covered_frames': frames, 'zero_padded_chunks': padded}
    return {'text': text, 'segments': [], 'native_payload_path': str(native), 'completion': {'status': 'returned', 'evidence': 'chunks cover every input sample without overlap; per-chunk EOS not exposed'}, 'metadata': meta}


def pause_split(pcm, sr, target, fwd=3.0, frame_ms=20):
    import numpy as np
    hop = sr * frame_ms // 1000
    count = len(pcm) // hop
    energy = np.sqrt(np.mean(pcm[:count * hop].reshape(count, hop) ** 2, axis=1) + 1e-12)
    chunks, start = [], 0
    while len(pcm) - start > int(target * sr):
        lo = (start + int(target / 2 * sr)) // hop
        hi = min(count, (start + int((target + fwd) * sr)) // hop)
        window = energy[lo:hi]
        quiet = window < min(np.percentile(window, 20) * 1.5, np.median(window) * 10 ** (-12 / 20))
        best, middle, run = 0, None, 0
        for i, q in enumerate(np.append(quiet, False)):
            if q:
                run += 1
                continue
            if run > best:
                best, middle = run, i - run // 2 - 1
            run = 0
        cut = (lo + middle) * hop if middle is not None else start + int(target * sr)
        chunks.append((pcm[start:cut], start / sr))
        start = cut
    chunks.append((pcm[start:], start / sr))
    return chunks


def window_gain(x, sr, target_db, window=0.5, max_gain=10.0, floor_db=-55.0):
    # Quiet interjections were skipped by Qwen until each 0.5 s window was raised toward the target level.
    import numpy as np
    size = int(window * sr)
    out = np.empty_like(x)
    for i in range(0, len(x), size):
        part = x[i:i + size]
        rms = np.sqrt(np.mean(part ** 2)) + 1e-9
        out[i:i + size] = part * (min(10 ** (target_db / 20) / rms, max_gain) if rms > 10 ** (floor_db / 20) else 1.0)
    return np.clip(out, -1, 1)


def verify_chunk_coverage(pcm, chunks, sr=16000):
    starts = [round(offset * sr) for _, offset in chunks]
    ends = starts[1:] + [len(pcm)]
    if not chunks or starts[0] != 0 or any(b <= a for a, b in zip(starts, ends)):
        fail('Chunks do not cover the input exactly')
    padded = 0
    for (chunk, _), start, end in zip(chunks, starts, ends):
        size = end - start
        if len(chunk) < size or (chunk[:size] != pcm[start:end]).any() or (chunk[size:] != 0).any():
            fail('Chunks do not cover the input exactly')
        padded += len(chunk) > size
    return padded


def nemotron_has_markers(text):
    return bool(re.search(r'<\|[^<>]*\|>|<spk:[^<>]*>', text, re.IGNORECASE))


def nemotron(req, output):
    offline()
    model = path(req['model']['path'], 'dir')
    if model.name != 'fused-64e-w4a16-g64':
        fail('Nemotron requires the fused-64e-w4a16-g64 speech checkpoint, never a text backbone')
    exe = path(req['runtime']['executable'], 'file')
    audio = path(req['audio_path'], 'file')
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    key = secrets.token_urlsafe(32)
    env = os.environ.copy()
    env['VLLM_PLUGINS'] = 'nemo_speechlm'
    command = [str(exe), 'serve', str(model), '--host', '127.0.0.1', '--port', str(port), '--api-key', key, '--served-model-name', 'local-asr', '--dtype', 'bfloat16', '--gpu-memory-utilization', '0.9', '--max-model-len', '8192']
    with open(output / 'server.log', 'x', encoding='utf-8') as log:
        proc = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic() + req.get('runtime', {}).get('startup_timeout_seconds', 600)
        while True:
            if proc.poll() is not None:
                fail('Local vLLM exited before readiness; check CUDA, NeMo plugin and fused checkpoint compatibility')
            try:
                local_request(port, key, '/health')
                break
            except (OSError, urllib.error.URLError):
                if time.monotonic() >= deadline:
                    fail('Local vLLM readiness timed out')
                time.sleep(.2)
        audio_data = base64.b64encode(audio.read_bytes()).decode('ascii')
        body = {'model': 'local-asr', 'messages': [{'role': 'user', 'content': [{'type': 'text', 'text': 'Transcribe the following audio.'}, {'type': 'audio_url', 'audio_url': {'url': 'data:audio/wav;base64,' + audio_data}}]}], 'temperature': 0, 'chat_template_kwargs': {'enable_thinking': False}, 'skip_special_tokens': False, 'max_tokens': req.get('decoding', {}).get('max_new_tokens', 1024)}
        native = output / 'native.json'
        result = local_request(port, key, '/v1/chat/completions', body, native)
        choice = result['choices'][0]
        if choice.get('finish_reason') != 'stop' or not isinstance(choice['message']['content'], str):
            fail('Nemotron generation did not finish normally')
        text = choice['message']['content']
        meta = base(req)
        if nemotron_has_markers(text):
            meta['text_scoring_supported'] = False
            meta['text_scoring_reason'] = 'native control or speaker markers require a reviewed parser'
        return {'text': text, 'segments': [], 'native_payload_path': str(native), 'completion': {'status': 'returned', 'evidence': 'server finish_reason=stop; input coverage not exposed'}, 'metadata': meta}

    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()


def validate_candidates(text, candidates):
    if not isinstance(text, str) or not isinstance(candidates, list):
        fail('Expected text and candidate list')
    ids = set()
    for c in candidates:
        if not isinstance(c, dict) or not isinstance(c.get('id'), str) or c['id'] in ids:
            fail('Candidate IDs must be unique strings')
        ids.add(c['id'])
        start, end = c.get('start'), c.get('end')
        if type(start) is not int or type(end) is not int or not 0 <= start < end <= len(text) or text[start:end] != c.get('original') or not isinstance(c.get('replacement'), str) or not c['replacement']:
            fail('Invalid candidate span')
    ordered = sorted(candidates, key=lambda c: c['start'])
    if any(a['end'] > b['start'] for a, b in zip(ordered, ordered[1:])):
        fail('Overlapping candidates')


def apply_decisions(text, candidates, payload):
    decisions = payload.get('decisions') if isinstance(payload, dict) else None
    if not isinstance(decisions, list) or len(decisions) != len(candidates):
        fail('Expected one decision per candidate')
    mapping = {}
    for item in decisions:
        if not isinstance(item, dict) or item.get('id') in mapping or item.get('decision') not in ('accept', 'keep', 'uncertain'):
            fail('Invalid decision')
        mapping[item.get('id')] = item['decision']
    if set(mapping) != {c['id'] for c in candidates}:
        fail('Decision IDs do not match candidates')
    for c in sorted(candidates, key=lambda c: c['start'], reverse=True):
        if mapping[c['id']] == 'accept':
            text = text[:c['start']] + c['replacement'] + text[c['end']:]
    return text


def correction_server_args(exe, model, port, key):
    return [str(exe), '--model', str(model), '--host', '127.0.0.1', '--port', str(port), '--api-key', key, '--no-webui', '--parallel', '1', '--ctx-size', str(CORRECTION_CONTEXT), '--no-context-shift', '--reasoning', 'off', '--jinja', '--chat-template-kwargs', json.dumps({'enable_thinking': False})]


def correction_body(messages, candidates, decoding):
    schema = {'type': 'object', 'additionalProperties': False, 'required': ['decisions'], 'properties': {'decisions': {'type': 'array', 'minItems': len(candidates), 'maxItems': len(candidates), 'items': {'type': 'object', 'additionalProperties': False, 'required': ['id', 'decision'], 'properties': {'id': {'type': 'string', 'enum': [c['id'] for c in candidates]}, 'decision': {'type': 'string', 'enum': ['accept', 'keep', 'uncertain']}}}}}}
    return {'messages': messages, 'temperature': decoding.get('temperature', 0), 'top_k': decoding.get('top_k', 1), 'top_p': decoding.get('top_p', 1), 'min_p': decoding.get('min_p', 0), 'repeat_penalty': decoding.get('repeat_penalty', 1), 'presence_penalty': decoding.get('presence_penalty', 0), 'frequency_penalty': 0, 'max_tokens': decoding.get('max_new_tokens', CORRECTION_OUTPUT), 'seed': decoding.get('seed', 0), 'stream': False, 'cache_prompt': False, 'chat_template_kwargs': {'enable_thinking': False}, 'response_format': {'type': 'json_schema', 'json_schema': {'name': 'term_decisions', 'strict': True, 'schema': schema}}}


def local_request(port, key, route, body=None, native_path=None):
    headers = {'Authorization': 'Bearer ' + key}
    if body is not None:
        headers['Content-Type'] = 'application/json'
    r = urllib.request.Request('http://127.0.0.1:%d%s' % (port, route), data=None if body is None else json.dumps(body, ensure_ascii=False).encode(), headers=headers)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(r, timeout=5 if body is None else 600) as response:
            if body is None:
                return None
            raw = response.read()
    except urllib.error.HTTPError as error:
        if native_path is not None:
            with open(native_path, 'xb') as native:
                native.write(error.read())
        raise
    if native_path is not None:
        with open(native_path, 'xb') as native:
            native.write(raw)
    return json.loads(raw)



def correction(req, output):
    text, candidates = req['text'], req['candidates']
    validate_candidates(text, candidates)
    model = path(req['model']['path'], 'file')
    exe = path(req['runtime']['executable'], 'file')
    if model.suffix.lower() != '.gguf':
        fail('Correction requires a local GGUF model')
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    key = secrets.token_urlsafe(32)
    with open(output / 'server.log', 'x', encoding='utf-8') as log:
        proc = subprocess.Popen(correction_server_args(exe, model, port, key), stdout=log, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic() + req.get('runtime', {}).get('startup_timeout_seconds', 120)
        while True:
            if proc.poll() is not None:
                fail('Local llama-server exited before readiness')
            try:
                local_request(port, key, '/health')
                break
            except (OSError, urllib.error.URLError):
                if time.monotonic() >= deadline:
                    fail('Local llama-server readiness timed out')
                time.sleep(.2)
        content = json.dumps({'text': text, 'candidates': candidates, 'approved_terms': req.get('context', {}).get('terms', [])}, ensure_ascii=False)
        messages = [{'role': 'system', 'content': 'For each candidate decide accept, keep, or uncertain based only on the transcript and approved terms. Never rewrite text or invent candidates. Return JSON object {"decisions":[{"id":"...","decision":"accept|keep|uncertain"}]}, exactly one per candidate.'}, {'role': 'user', 'content': content}]
        dec = req.get('decoding', {})
        native = output / 'native.json'
        result = local_request(port, key, '/v1/chat/completions', correction_body(messages, candidates, dec), native)
        choice = result['choices'][0]
        if choice.get('finish_reason') != 'stop':
            fail('Correction generation did not finish normally')
        if not isinstance(choice['message'].get('content'), str) or not choice['message']['content'].strip():
            fail('Correction response has no content')
        answer = json.loads(choice['message']['content'])
        corrected = apply_decisions(text, candidates, answer)
        meta = base(req)
        meta['context_used'] = bool(req.get('context', {}).get('terms', []))
        meta['reasoning_content_chars'] = len(choice['message'].get('reasoning_content') or '')
        return {'text': corrected, 'segments': [], 'native_payload_path': str(native), 'completion': {'status': 'complete', 'evidence': 'server finish_reason=stop and validated candidate decisions'}, 'metadata': meta}
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()


def doctor(req):
    kind = req['adapter']
    model_path = path(req['model']['path'], 'file' if kind == 'correction' else 'dir')
    checks = {'model_exists': True}
    if kind == 'moss':
        checks['platform_compatible'] = sys.platform == 'darwin' and platform.machine().lower() in ('arm64', 'aarch64')
    elif kind == 'nemotron':
        checks['platform_compatible'] = sys.platform == 'linux'
    else:
        checks['platform_compatible'] = sys.platform in ('darwin', 'linux', 'win32')
    if kind in ('moss', 'correction'):
        exe = path(req['runtime']['executable'], 'file')
        checks['executable'] = os.access(exe, os.X_OK)
    elif kind == 'qwen':
        checks['qwen_asr_installed'] = importlib.util.find_spec('qwen_asr') is not None
    elif kind == 'nemotron':
        checks['selected_fused_variant'] = model_path.name == 'fused-64e-w4a16-g64'
        checks['vllm_installed'] = importlib.util.find_spec('vllm') is not None
        checks['executable'] = os.access(path(req['runtime']['executable'], 'file'), os.X_OK)
        checks['cuda_required'] = True
    else:
        fail('Unknown adapter')
    return checks


def validate_request(req):
    if not isinstance(req, dict) or req.get('schema_version') != 1:
        fail('Unsupported request schema')
    allowed = {'schema_version', 'adapter', 'task', 'model', 'runtime', 'output_dir', 'audio_path', 'text', 'candidates', 'context', 'decoding'}
    if set(req) - allowed:
        fail('Unexpected request fields')
    kind = req.get('adapter')
    if kind not in ('nemotron', 'moss', 'qwen', 'correction'):
        fail('Unknown adapter')
    if req.get('task') != ('postprocess' if kind == 'correction' else 'asr'):
        fail('Task does not match adapter')
    if kind == 'correction':
        if 'audio_path' in req:
            fail('Postprocess request cannot contain audio')
    elif 'text' in req or 'candidates' in req:
        fail('ASR request cannot contain postprocess input')
    if not isinstance(req.get('model'), dict) or not isinstance(req.get('runtime', {}), dict) or not isinstance(req.get('decoding', {}), dict):
        fail('Invalid model, runtime or decoding')
    context = req.get('context', {})
    if not isinstance(context, dict) or set(context) - {'terms'} or not isinstance(context.get('terms', []), list) or not all(isinstance(term, str) for term in context.get('terms', [])):
        fail('Invalid approved terms')
    if set(req['model']) - {'path', 'id', 'format', 'variant'}:
        fail('Unexpected model fields')
    runtime_fields = {'moss': {'python', 'executable'}, 'qwen': {'python', 'device_map', 'dtype'}, 'nemotron': {'python', 'executable', 'startup_timeout_seconds'}, 'correction': {'python', 'executable', 'startup_timeout_seconds'}}
    runtime = req.get('runtime', {})
    if set(runtime) - runtime_fields[kind]:
        fail('Unsupported runtime fields for adapter')
    if 'startup_timeout_seconds' in runtime and (type(runtime['startup_timeout_seconds']) not in (int, float) or not math.isfinite(runtime['startup_timeout_seconds']) or runtime['startup_timeout_seconds'] <= 0):
        fail('Invalid startup timeout')
    if 'dtype' in runtime and runtime['dtype'] not in QWEN_DTYPES:
        fail('Unsupported dtype')
    if kind in ('moss', 'nemotron') and context.get('terms'):
        fail('This adapter does not support dictionary context')
    accepted = {'moss': set(), 'qwen': {'max_new_tokens', *QWEN_CHUNK_FIELDS}, 'nemotron': {'max_new_tokens'}, 'correction': {'temperature', 'max_new_tokens', *SAMPLING_FIELDS}}
    decoding = req.get('decoding', {})
    if set(decoding) - accepted[kind]:
        fail('Unsupported decoding fields for adapter')
    if 'max_new_tokens' in decoding and (type(decoding['max_new_tokens']) is not int or decoding['max_new_tokens'] <= 0):
        fail('Invalid max_new_tokens')
    if kind == 'nemotron' and decoding.get('max_new_tokens', 1024) > 8192:
        fail('Nemotron max_new_tokens exceeds its model context')
    if kind == 'correction' and decoding.get('max_new_tokens', CORRECTION_OUTPUT) >= CORRECTION_CONTEXT:
        fail('Correction max_new_tokens exceeds its server context')
    if 'temperature' in decoding and (type(decoding['temperature']) not in (int, float) or not math.isfinite(decoding['temperature']) or decoding['temperature'] < 0):
        fail('Invalid temperature')
    for field in ('top_p', 'min_p', 'presence_penalty', 'repeat_penalty', 'chunk_seconds', 'chunk_pad_seconds'):
        if field in decoding and (type(decoding[field]) not in (int, float) or not math.isfinite(decoding[field]) or decoding[field] < 0):
            fail('Invalid ' + field)
    for field in ('top_k', 'seed'):
        if field in decoding and (type(decoding[field]) is not int or decoding[field] < 0):
            fail('Invalid ' + field)
    if 'chunk_seconds' in decoding and not 1 <= decoding['chunk_seconds'] <= 1200:
        fail('chunk_seconds must be between 1 and 1200')
    if set(decoding) & set(QWEN_CHUNK_FIELDS[1:]) and 'chunk_seconds' not in decoding:
        fail('chunk options require chunk_seconds')
    if 'chunk_boundary' in decoding and decoding['chunk_boundary'] not in QWEN_BOUNDARIES:
        fail('chunk_boundary must be energy or pause')
    if decoding.get('chunk_pad_seconds', 0) > 5:
        fail('chunk_pad_seconds must be between 0 and 5')
    if 'chunk_gain_db' in decoding and (type(decoding['chunk_gain_db']) not in (int, float) or not -40 <= decoding['chunk_gain_db'] <= -6):
        fail('chunk_gain_db must be between -40 and -6')



def terminate_on_signal(signum, frame):
    raise SystemExit(128 + signum)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('request')
    parser.add_argument('--doctor', action='store_true')
    args = parser.parse_args()
    req = json.loads(Path(args.request).read_text(encoding='utf-8'))
    validate_request(req)
    signal.signal(signal.SIGTERM, terminate_on_signal)
    checks = doctor(req)
    if args.doctor:
        print(json.dumps(checks))
        return
    output = path(req['output_dir'], 'dir')
    kind = req['adapter']
    if kind == 'nemotron':
        result = nemotron(req, output)
    elif kind == 'moss':
        result = moss(req, output)
    elif kind == 'qwen':
        result = qwen(req, output)
    elif kind == 'correction':
        result = correction(req, output)
    else:
        fail('Unknown adapter')
    write_json(output / 'result.json', result)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print('Adapter failed: ' + str(error), file=sys.stderr)
        sys.exit(1)
