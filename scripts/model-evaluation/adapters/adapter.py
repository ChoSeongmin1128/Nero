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
    defaults = {'moss': {'max_new_tokens': 65536, 'temperature': 0, 'top_p': 1, 'min_chunk_duration': 0, 'repetition_context_size': 100}, 'qwen': {'max_new_tokens': 1024}, 'nemotron': {'max_new_tokens': 1024, 'temperature': 0, 'enable_thinking': False, 'skip_special_tokens': False}, 'correction': {'max_new_tokens': 1024, 'temperature': 0}}
    decoding = {**defaults[kind], **supplied}
    distributions = {'qwen': ('qwen-asr', 'transformers', 'torch'), 'nemotron': ('vllm', 'nemo-toolkit'), 'moss': (), 'correction': ()}
    versions = {}
    for name in distributions[kind]:
        try:
            versions[name] = importlib.metadata.version(name)
        except importlib.metadata.PackageNotFoundError:
            versions[name] = None
    runtime = {k: v for k, v in request.get('runtime', {}).items() if k in ('python', 'executable', 'device_map', 'startup_timeout_seconds')}
    runtime['python_version'] = platform.python_version()
    runtime['adapter_python_package_versions'] = versions
    if kind == 'qwen':
        runtime['max_inference_batch_size'] = 1
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
    instance = Qwen3ASRModel.from_pretrained(str(model), **kwargs)
    terms = req.get('context', {}).get('terms', [])
    results = instance.transcribe(audio=str(audio), language='Korean', context=', '.join(terms))
    if len(results) != 1 or not isinstance(results[0].text, str):
        fail('Unexpected Qwen ASR output')
    native = output / 'native.json'
    write_json(native, {'text': results[0].text, 'language': results[0].language})
    meta = base(req)
    meta['context_used'] = bool(terms)
    return {'text': results[0].text, 'segments': [], 'native_payload_path': str(native), 'completion': {'status': 'returned', 'evidence': 'runtime returned one result; EOS and input coverage not exposed'}, 'metadata': meta}


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
        proc = subprocess.Popen([str(exe), '--model', str(model), '--host', '127.0.0.1', '--port', str(port), '--api-key', key], stdout=log, stderr=subprocess.STDOUT)
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
        result = local_request(port, key, '/v1/chat/completions', {'messages': messages, 'temperature': dec.get('temperature', 0), 'max_tokens': dec.get('max_new_tokens', 1024), 'response_format': {'type': 'json_object'}}, native)
        choice = result['choices'][0]
        if choice.get('finish_reason') != 'stop':
            fail('Correction generation did not finish normally')
        answer = json.loads(choice['message']['content'])
        corrected = apply_decisions(text, candidates, answer)
        meta = base(req)
        meta['context_used'] = bool(req.get('context', {}).get('terms', []))
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
    runtime_fields = {'moss': {'python', 'executable'}, 'qwen': {'python', 'device_map'}, 'nemotron': {'python', 'executable', 'startup_timeout_seconds'}, 'correction': {'python', 'executable', 'startup_timeout_seconds'}}
    runtime = req.get('runtime', {})
    if set(runtime) - runtime_fields[kind]:
        fail('Unsupported runtime fields for adapter')
    if 'startup_timeout_seconds' in runtime and (type(runtime['startup_timeout_seconds']) not in (int, float) or not math.isfinite(runtime['startup_timeout_seconds']) or runtime['startup_timeout_seconds'] <= 0):
        fail('Invalid startup timeout')
    if kind in ('moss', 'nemotron') and context.get('terms'):
        fail('This adapter does not support dictionary context')
    accepted = {'moss': set(), 'qwen': {'max_new_tokens'}, 'nemotron': {'max_new_tokens'}, 'correction': {'temperature', 'max_new_tokens'}}
    decoding = req.get('decoding', {})
    if set(decoding) - accepted[kind]:
        fail('Unsupported decoding fields for adapter')
    if 'max_new_tokens' in decoding and (type(decoding['max_new_tokens']) is not int or decoding['max_new_tokens'] <= 0):
        fail('Invalid max_new_tokens')
    if kind == 'nemotron' and decoding.get('max_new_tokens', 1024) > 8192:
        fail('Nemotron max_new_tokens exceeds its model context')
    if 'temperature' in decoding and (type(decoding['temperature']) not in (int, float) or not math.isfinite(decoding['temperature']) or decoding['temperature'] < 0):
        fail('Invalid temperature')



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
