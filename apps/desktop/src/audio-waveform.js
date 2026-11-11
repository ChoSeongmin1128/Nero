// A sampled preview of the local playback WAV. The seek input remains the control.
const HEADER_BYTES = 64 * 1024, WINDOW_BYTES = 8 * 1024, BARS = 64;
const MAX_REQUEST_BYTES = 256 * 1024, MAX_TOTAL_BYTES = 1024 * 1024;
const fourCC = (view, offset) => String.fromCharCode(...new Uint8Array(view.buffer, view.byteOffset + offset, 4));

export function localAudioUrl(source, base) {
  try {
    const url = new URL(source, base);
    if (url.protocol === 'asset:' || (['http:', 'https:', 'tauri:'].includes(url.protocol) && ['localhost', '127.0.0.1', '[::1]', 'asset.localhost', 'tauri.localhost'].includes(url.hostname))) return url.href;
  } catch { /* Unsupported source keeps the ordinary seek slider. */ }
  return null;
}

export function parseWaveHeader(bytes, total) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.byteLength < 12 || fourCC(view, 0) !== 'RIFF' || fourCC(view, 8) !== 'WAVE') throw new Error('Unsupported WAV');
  const declared = view.getUint32(4, true) + 8;
  if (declared > total || declared < 12) throw new Error('Invalid RIFF bounds');
  let format;
  for (let at = 12; at + 8 <= Math.min(bytes.length, declared);) {
    const kind = fourCC(view, at), size = view.getUint32(at + 4, true), start = at + 8;
    if (size > declared - start) throw new Error('Invalid WAV chunk');
    if (kind === 'fmt ') {
      if (size < 16 || start + 16 > bytes.length) throw new Error('Incomplete WAV format');
      let encoding = view.getUint16(start, true);
      const channels = view.getUint16(start + 2, true), rate = view.getUint32(start + 4, true), byteRate = view.getUint32(start + 8, true), align = view.getUint16(start + 12, true), bits = view.getUint16(start + 14, true);
      // Hound writes Nero's float playback WAV using WAVE_FORMAT_EXTENSIBLE.
      if (encoding === 0xfffe) {
        const guidTail = [0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113];
        if (size < 40 || start + 40 > bytes.length || view.getUint16(start + 16, true) < 22 || view.getUint16(start + 18, true) !== bits || !guidTail.every((value, index) => view.getUint8(start + 28 + index) === value)) throw new Error('Unsupported extensible WAV');
        encoding = view.getUint32(start + 24, true);
      }
      if (!((encoding === 1 && [16, 24, 32].includes(bits)) || (encoding === 3 && bits === 32)) || channels < 1 || channels > 32 || rate < 1 || rate > 384000 || align !== channels * bits / 8 || byteRate !== rate * align) throw new Error('Unsupported WAV format');
      format = { encoding, channels, rate, align, bits };
    }
    if (kind === 'data') {
      if (!format || size < format.align || start + size > total || size % format.align) throw new Error('Invalid WAV data');
      return { ...format, start, size, frames: size / format.align };
    }
    at = start + size + (size & 1);
  }
  throw new Error('WAV header exceeds preview limit');
}

export async function fetchAudioRange(fetcher, url, start, end, signal, budget) {
  if (!localAudioUrl(url)) throw new Error('Waveform requires a local audio source');
  const requested = end - start + 1;
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || requested < 1 || requested > MAX_REQUEST_BYTES || budget.bytes + requested > MAX_TOTAL_BYTES) throw new Error('Waveform request limit');
  budget.bytes += requested;
  const response = await fetcher(url, { headers: { Range: `bytes=${start}-${end}` }, signal, credentials: 'omit', redirect: 'error' });
  const fail = async () => { try { await response.body?.cancel(); } catch { /* Reader may already own the stream. */ } throw new Error('Invalid partial audio response'); };
  if (response.status !== 206) return fail();
  const match = /^bytes (\d+)-(\d+)\/(\d+)$/.exec(response.headers.get('Content-Range') || '');
  if (!match) return fail();
  const from = Number(match[1]), to = Number(match[2]), total = Number(match[3]), length = to - from + 1;
  if (![from, to, total].every(Number.isSafeInteger) || from !== start || to !== Math.min(end, total - 1) || total < 1 || to < from || length > requested) return fail();
  const advertised = response.headers.get('Content-Length');
  if (advertised !== null && Number(advertised) !== length) return fail();
  if (!response.body?.getReader) return fail();
  const reader = response.body.getReader(), chunks = [];
  let count = 0;
  try {
    while (true) {
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
      const { value, done } = await reader.read();
      if (done) break;
      count += value.byteLength;
      if (count > length) throw new Error('Audio response exceeds range');
      chunks.push(value);
    }
    if (count !== length) throw new Error('Incomplete audio range');
  } catch (error) { await reader.cancel().catch(() => {}); throw error; }
  finally { reader.releaseLock(); }
  const bytes = new Uint8Array(count);
  let at = 0;
  for (const chunk of chunks) { bytes.set(chunk, at); at += chunk.byteLength; }
  return { bytes, total };
}

function peakOf(bytes, format) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), step = format.bits / 8;
  let peak = 0;
  for (let at = 0; at + step <= bytes.length; at += step) {
    let sample;
    if (format.encoding === 3) sample = view.getFloat32(at, true);
    else if (step === 2) sample = view.getInt16(at, true) / 32768;
    else if (step === 3) { const raw = view.getUint8(at) | view.getUint8(at + 1) << 8 | view.getUint8(at + 2) << 16; sample = ((raw << 8) >> 8) / 8388608; }
    else sample = view.getInt32(at, true) / 2147483648;
    if (Number.isFinite(sample)) peak = Math.max(peak, Math.min(1, Math.abs(sample)));
  }
  return peak;
}

export async function sampleAudioWaveform(url, { fetcher = globalThis.fetch, signal } = {}) {
  const budget = { bytes: 0 }, header = await fetchAudioRange(fetcher, url, 0, HEADER_BYTES - 1, signal, budget);
  const format = parseWaveHeader(header.bytes, header.total), peaks = new Array(BARS);
  let cursor = 0;
  const worker = async () => {
    while (cursor < BARS) {
      const index = cursor++, first = Math.floor(index * format.frames / BARS), next = Math.floor((index + 1) * format.frames / BARS);
      const frames = Math.min(Math.max(1, next - first), Math.floor(WINDOW_BYTES / format.align));
      const startFrame = Math.min(format.frames - frames, first + Math.floor(Math.max(0, next - first - frames) / 2));
      const start = format.start + startFrame * format.align, end = start + frames * format.align - 1;
      let bytes;
      if (end < header.bytes.length) bytes = header.bytes.subarray(start, end + 1);
      else {
        const range = await fetchAudioRange(fetcher, url, start, end, signal, budget);
        if (range.total !== header.total) throw new Error('Audio changed during preview');
        bytes = range.bytes;
      }
      peaks[index] = peakOf(bytes, format);
    }
  };
  await Promise.all([worker(), worker(), worker()]);
  return peaks;
}

export function waveformBarHeights(peaks) {
  const maximum = Math.max(0, ...peaks.filter(Number.isFinite));
  return peaks.map(value => Math.max(2, Math.round(maximum > 0 && Number.isFinite(value) ? Math.max(0, value) / maximum * 30 : 0)));
}

export function connectAudioWaveform(audio, host, seek) {
  if (!host || !seek) return { dispose() {} };
  const track = host.parentElement, document = host.ownerDocument;
  let controller, source = null, disposed = false;
  const clear = () => { controller?.abort(); controller = null; source = null; host.hidden = true; host.replaceChildren(); track?.removeAttribute('data-waveform-ready'); };
  const progress = () => {
    const duration = Number(audio.duration), current = Number(audio.currentTime), ratio = duration > 0 && Number.isFinite(duration) ? Math.max(0, Math.min(1, current / duration)) : 0;
    [...host.children].forEach((bar, index) => { bar.dataset.played = String(index / BARS < ratio); });
  };
  const load = async () => {
    const raw = audio.getAttribute('src'), url = raw && localAudioUrl(raw, document.baseURI);
    if (!url) { clear(); return; }
    if (source === raw) return;
    clear(); source = raw;
    const request = controller = new AbortController();
    try {
      const peaks = await sampleAudioWaveform(url, { signal: request.signal });
      if (disposed || request.signal.aborted || audio.getAttribute('src') !== raw || controller !== request) return;
      const fragment = document.createDocumentFragment();
      waveformBarHeights(peaks).forEach(height => { const bar = document.createElement('span'); bar.style.height = `${height}px`; fragment.append(bar); });
      host.replaceChildren(fragment); host.hidden = false; track?.setAttribute('data-waveform-ready', 'true'); progress();
    } catch { if (controller === request) clear(); }
  };
  const reset = () => { clear(); void load(); };
  const listeners = [['loadstart', reset], ['loadedmetadata', load], ['emptied', clear], ['error', clear], ['timeupdate', progress], ['seeking', progress]];
  listeners.forEach(([name, handler]) => audio.addEventListener(name, handler));
  const Observer = document.defaultView?.MutationObserver;
  const observer = Observer && new Observer(() => { if (source !== audio.getAttribute('src')) reset(); });
  observer?.observe(audio, { attributes: true, attributeFilter: ['src'] });
  void load();
  return { dispose() { disposed = true; clear(); observer?.disconnect(); listeners.forEach(([name, handler]) => audio.removeEventListener(name, handler)); } };
}
