#!/usr/bin/env python3
"""Create synthetic codec fixtures, never recordings. Requires macOS afconvert and
soundfile 0.13.1/numpy 2.3.4 in a fixture-only environment. No tool is an app dependency.
Usage: python make-fixtures.py <new-temporary-directory>
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import struct

import numpy as np
import soundfile as sf


def generate(output):
    output.mkdir(parents=True, exist_ok=False)
    frames, rate = 46337, 44100
    time = np.arange(frames) / rate
    signal = np.column_stack([0.3 * np.sin(2 * np.pi * (440 * time + 180 * time**2)),
                              0.1 * np.sin(2 * np.pi * 880 * time)])
    cases = []
    for name, container, codec in [
        ('source.wav', 'WAV', 'PCM_16'), ('pcm24.wav', 'WAV', 'PCM_24'),
        ('float.wav', 'WAV', 'FLOAT'), ('audio.aiff', 'AIFF', 'PCM_16'),
        ('audio.caf', 'CAF', 'PCM_16'), ('audio.flac', 'FLAC', 'PCM_16'),
        ('vorbis.ogg', 'OGG', 'VORBIS'), ('audio.mp3', 'MP3', 'MPEG_LAYER_III'),
    ]:
        sf.write(output / name, signal, rate, format=container, subtype=codec)
        cases.append(dict(file=name, frames=frames, rate=rate, codec=codec))
    # A VBR stream without Xing/LAME has no exact duration tag. Count this
    # synthetic MPEG-1 Layer III fixture's packets independently of the decoder.
    raw = (output / 'audio.mp3').read_bytes()
    offset, packets, first_end = 0, 0, None
    bitrates = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320]
    while offset < len(raw):
        header = int.from_bytes(raw[offset:offset+4], 'big')
        assert header >> 21 == 0x7ff and (header >> 17) & 15 == 13
        size = 144000 * bitrates[(header >> 12) & 15] // rate + ((header >> 9) & 1)
        assert size > 4 and offset + size <= len(raw)
        offset += size
        packets += 1
        if first_end is None:
            first_end = offset
    assert offset == len(raw) and b'Xing' in raw[:first_end]
    (output / 'untagged.mp3').write_bytes(raw[first_end:])
    cases.append(dict(file='untagged.mp3', frames=(packets-1)*1152, rate=rate,
                      codec='MPEG_LAYER_III', noLengthTag=True))
    for name, container, codec in [('aac.m4a', 'm4af', 'aac'),
                                    ('alac.m4a', 'm4af', 'alac'),
                                    ('adts.aac', 'adts', 'aac')]:
        subprocess.run(['/usr/bin/afconvert', str(output / 'source.wav'),
                        str(output / name), '-f', container, '-d', codec], check=True)
        count = frames
        if container == 'adts':
            raw, offset, count = (output / name).read_bytes(), 0, 0
            while offset < len(raw):
                assert raw[offset] == 255 and raw[offset + 1] & 0xF6 == 0xF0
                size = ((raw[offset+3] & 3) << 11) | (raw[offset+4] << 3) | (raw[offset+5] >> 5)
                assert size > 7 and offset + size <= len(raw)
                offset += size
                count += 1024
            assert offset == len(raw)
        cases.append(dict(file=name, frames=count, rate=rate, codec=codec))
    subprocess.run(['/usr/bin/afconvert', str(output / 'aac.m4a'),
                    str(output / 'aac-reference.wav'), '-f', 'WAVE', '-d', 'LEI16'], check=True)
    assert sf.info(output / 'aac-reference.wav').frames == frames
    next(case for case in cases if case['file'] == 'aac.m4a')['reference'] = 'aac-reference.wav'
    # The same encoded AAC with a standards-based, single normal-rate edit.
    raw = bytearray((output / 'aac.m4a').read_bytes())
    def atom(parent, name):
        offset, end = parent
        while offset + 8 <= end:
            size, kind = struct.unpack_from('>I4s', raw, offset)
            assert size >= 8 and offset + size <= end
            if kind == name:
                return offset, offset + size
            offset += size
        raise AssertionError(name)
    moov = atom((0, len(raw)), b'moov')
    trak = atom((moov[0]+8, moov[1]), b'trak')
    mvhd = atom((moov[0]+8, moov[1]), b'mvhd')
    tkhd = atom((trak[0]+8, trak[1]), b'tkhd')
    free = atom((moov[1], len(raw)), b'free')
    elst = struct.pack('>I4sIIIII', 28, b'elst', 0, 1, frames, 2112, 0x10000)
    edts = struct.pack('>I4s', len(elst)+8, b'edts') + elst
    struct.pack_into('>I', raw, mvhd[0]+8+16, frames)
    struct.pack_into('>I', raw, tkhd[0]+8+20, frames)
    struct.pack_into('>I', raw, moov[0], moov[1]-moov[0]+len(edts))
    struct.pack_into('>I', raw, trak[0], trak[1]-trak[0]+len(edts))
    struct.pack_into('>I', raw, free[0], free[1]-free[0]-len(edts))
    del raw[free[1]-len(edts):free[1]]
    raw[trak[1]:trak[1]] = edts
    raw = raw.replace(b'iTunSMPB', b'neroSMPB')
    (output / 'edit-aac.m4a').write_bytes(raw)
    cases.append(dict(file='edit-aac.m4a', frames=frames, rate=rate,
                      codec='aac', reference='aac-reference.wav'))
    bad = raw.replace(elst, struct.pack('>I4sIIIiI', 28, b'elst', 0, 1, frames, -1, 0x10000))
    (output / 'empty-edit.m4a').write_bytes(bad)
    bad = (output / 'aac.m4a').read_bytes().replace(b'00000840', b'FFFFFFFF')
    (output / 'invalid-gapless.m4a').write_bytes(bad)
    time = np.arange(48000) / 48000
    sf.write(output / 'opus.ogg', np.sin(time * 440 * 2 * np.pi) * 0.3,
             48000, format='OGG', subtype='OPUS')
    subprocess.run(['/usr/bin/afconvert', str(output / 'source.wav'),
                    str(output / 'he-aac.m4a'), '-f', 'm4af', '-d', 'aach', '-b', '48000'], check=True)
    subprocess.run(['/usr/bin/afconvert', str(output / 'source.wav'),
                    str(output / 'he-adts.aac'), '-f', 'adts', '-d', 'aach', '-b', '48000'], check=True)
    for case in cases:
        case['sha256'] = hashlib.sha256((output / case['file']).read_bytes()).hexdigest()
    (output / 'manifest.json').write_text(json.dumps(dict(cases=cases, unsupported=['opus.ogg', 'he-aac.m4a', 'empty-edit.m4a', 'invalid-gapless.m4a', 'he-adts.aac']), indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    generate(parser.parse_args().output)
