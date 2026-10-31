"""Read PE32+ dependency metadata. No executable/DLL is loaded for validation."""
import re
import struct

SYSTEM = {'kernel32.dll', 'user32.dll', 'advapi32.dll', 'shell32.dll', 'ole32.dll',
          'oleaut32.dll', 'bcrypt.dll', 'ntdll.dll', 'ws2_32.dll', 'version.dll',
          'shlwapi.dll', 'ucrtbase.dll', 'msvcrt.dll'}


def imports(path):
    data = path.read_bytes()
    def u32(offset):
        return struct.unpack_from('<I', data, offset)[0]
    if data[:2] != b'MZ':
        raise ValueError('Missing PE header')
    pe = u32(60)
    if data[pe:pe + 4] != b'PE\0\0' or data[pe + 4:pe + 6] != b'\x64\x86':
        raise ValueError('Whisper requires a Windows AMD64 executable')
    optional = pe + 24
    if data[optional:optional + 2] != b'\x0b\x02':
        raise ValueError('Whisper requires PE32+')
    sections = []
    cursor = optional + struct.unpack_from('<H', data, pe + 20)[0]
    for _ in range(struct.unpack_from('<H', data, pe + 6)[0]):
        sections.append((u32(cursor + 12), max(u32(cursor + 8), u32(cursor + 16)), u32(cursor + 20)))
        cursor += 40
    def offset(rva):
        for base, length, raw in sections:
            if base <= rva < base + length:
                return raw + rva - base
        raise ValueError('PE dependency address is outside its sections')
    def name(rva):
        start = offset(rva)
        end = data.find(b'\0', start, start + 256)
        if end < 0:
            raise ValueError('Invalid PE dependency string')
        value = data[start:end].decode('ascii').lower()
        if not re.fullmatch(r'[a-z0-9_.-]+\.dll', value):
            raise ValueError('Noncanonical PE dependency')
        return value
    result = set()
    # IMAGE_IMPORT_DESCRIPTOR and IMAGE_DELAYLOAD_DESCRIPTOR.
    for index, stride, name_offset in ((1, 20, 12), (13, 32, 4)):
        rva, size = struct.unpack_from('<II', data, optional + 112 + index * 8)
        if not rva:
            continue
        start = offset(rva)
        for i in range(min(size // stride + 1, 4096)):
            entry = start + i * stride
            if data[entry:entry + stride] == b'\0' * stride:
                break
            if index == 13 and u32(entry) != 1:
                raise ValueError('Unexpected non-RVA delay import')
            result.add(name(u32(entry + name_offset)))
        else:
            raise ValueError('Unterminated PE dependency table')
    return sorted(result)


def verify_closure(root):
    result = {}
    for path in sorted(root.rglob('*')):
        if path.suffix.lower() not in ('.dll', '.exe'):
            continue
        deps = imports(path)
        for dependency in deps:
            if (dependency in SYSTEM or dependency.startswith(('api-ms-win-', 'ext-ms-win-'))
                    or (dependency == 'vulkan-1.dll' and path == root / 'vulkan/bin/whisper-cli.exe')
                    or (path.parent / dependency).is_file()):
                continue
            raise ValueError('Unpackaged Whisper dependency: ' + dependency)
        if path.name == 'nero-vulkan-probe.exe' and 'vulkan-1.dll' in deps:
            raise ValueError('Capability probe must not import the Vulkan loader')
        if path == root / 'vulkan/bin/whisper-cli.exe' and 'vulkan-1.dll' not in deps:
            raise ValueError('Vulkan executable does not link its pinned backend')
        result[path.relative_to(root).as_posix()] = deps
    return result
