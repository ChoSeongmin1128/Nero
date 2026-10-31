"""Source-built, isolated Vulkan executable plus a loader-safe capability probe."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def apply_patch(source, pin, prepare):
    patch_path = Path(__file__).parent / pin['file']
    changed = source / pin['path']
    if (patch_path.stat().st_size != pin['bytes'] or prepare.sha(patch_path) != pin['sha256']
            or prepare.sha(changed) != pin['baseSha256']):
        raise ValueError('Whisper Vulkan execution-evidence patch differs from its pin')
    lines = patch_path.read_bytes().splitlines(keepends=True)
    if (lines[:2] != [f"--- a/{pin['path']}\n".encode(), f"+++ b/{pin['path']}\n".encode()]
            or not lines[2].startswith(b'@@ ') or any(line.startswith(b'@@ ') for line in lines[3:])
            or any(line[:1] not in (b' ', b'+', b'-') for line in lines[3:])):
        raise ValueError('Unexpected execution-evidence patch layout')
    old = b''.join(line[1:] for line in lines[3:] if line[:1] in (b' ', b'-'))
    new = b''.join(line[1:] for line in lines[3:] if line[:1] in (b' ', b'+'))
    raw = changed.read_bytes()
    if raw.count(old) != 1:
        raise ValueError('Execution-evidence patch context is not unique')
    result = raw.replace(old, new, 1)
    import hashlib
    if hashlib.sha256(result).hexdigest() != pin['resultSha256']:
        raise ValueError('Patched Vulkan source differs from its result pin')
    changed.write_bytes(result)


def build(profile, source_pin, cache, stage, tools, prepare):
    if os.name != 'nt':
        raise ValueError('Whisper Vulkan build requires Windows x64 MSVC')
    tools = Path(tools).absolute()
    module = prepare.load_module('nero_whisper_tools', Path(__file__).with_name('toolchain.py'))
    module.verify(tools, prepare.sha)
    pin = profile['vulkan']
    for name, expected in [('toolchain.json', pin['toolchainSha256']),
                           ('vulkan_probe.cpp', pin['probeSourceSha256'])]:
        if prepare.sha(Path(__file__).with_name(name)) != expected:
            raise ValueError('Whisper Windows build input differs from its pin: ' + name)
    sdk = tools / 'sdk'
    cmake = tools / 'cmake/cmake-4.1.2-windows-x86_64/bin/cmake.exe'
    ninja = tools / 'ninja/ninja.exe'
    source_archive = prepare.download(cache, f"whisper.cpp-{source_pin['commit']}.tar.gz", source_pin,
        f"https://codeload.github.com/{source_pin['repository']}/tar.gz/{source_pin['commit']}")
    source_module = prepare.load_module('nero_whisper_source', Path(__file__).with_name('macos.py'))
    # Only a child process sees build tool search paths. No machine/user PATH or SDK settings.
    env = dict(os.environ, VULKAN_SDK=str(sdk), CMAKE_BUILD_PARALLEL_LEVEL='2',
               CC='cl.exe', CXX='cl.exe', VSLANG='1033')
    for name in ('CL', '_CL_', 'CFLAGS', 'CXXFLAGS', 'LDFLAGS', 'CMAKE_TOOLCHAIN_FILE'):
        env.pop(name, None)
    env['PATH'] = str(ninja.parent) + os.pathsep + str(sdk / 'Bin') + os.pathsep + env.get('PATH', '')
    with tempfile.TemporaryDirectory(prefix='whisper-vulkan-build-', dir=cache.parent) as folder:
        folder = Path(folder)
        source = source_module.unpack(source_archive, folder / 'source')
        apply_patch(source, pin['patch'], prepare)
        apply_patch(source, pin['buildPatch'], prepare)
        dependencies = {}
        for item in pin['dependencies']:
            name = item['repository'].split('/')[-1]
            archive = prepare.download(cache, f"{name}-{item['commit']}.tar.gz", item,
                f"https://codeload.github.com/{item['repository']}/tar.gz/{item['commit']}")
            dependencies[name] = source_module.unpack(archive, folder / name)
            for license in item['licenses']:
                destination = stage / 'share/licenses' / name / license
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(dependencies[name] / license, destination)
        # Upstream b5454 finds SPIRV-Headers but does not link its interface target.
        # A combined include root matches its <spirv/...> and <vulkan/...> includes
        # while keeping both header sources pinned independently of the SDK layout.
        includes = folder / 'includes'
        shutil.copytree(dependencies['Vulkan-Headers'] / 'include', includes)
        shutil.copytree(dependencies['SPIRV-Headers'] / 'include', includes, dirs_exist_ok=True)
        spirv_config = folder / 'spirv-config'
        spirv_config.mkdir()
        (spirv_config / 'SPIRV-HeadersConfig.cmake').write_text(
            'add_library(SPIRV-Headers::SPIRV-Headers INTERFACE IMPORTED)\n'
            'set_target_properties(SPIRV-Headers::SPIRV-Headers PROPERTIES '
            'INTERFACE_INCLUDE_DIRECTORIES "' + includes.as_posix() + '")\n',
            encoding='utf-8')
        output = folder / 'build'
        configure = [str(cmake), '-S', str(source), '-B', str(output), *pin['cmakeFlags'],
                     '-DCMAKE_MAKE_PROGRAM:FILEPATH=' + str(ninja),
                     '-DVulkan_INCLUDE_DIR:PATH=' + str(includes),
                     '-DVulkan_LIBRARY:FILEPATH=' + str(sdk / 'Lib/vulkan-1.lib'),
                     '-DVulkan_GLSLC_EXECUTABLE:FILEPATH=' + str(sdk / 'Bin/glslc.exe'),
                     '-DSPIRV-Headers_DIR:PATH=' + str(spirv_config)]
        subprocess.run(configure, env=env, check=True)
        actual = dict(line.split('=', 1) for line in (output / 'CMakeCache.txt').read_text().splitlines()
                      if '=' in line and not line.startswith(('#', '//')))
        for flag in pin['cmakeFlags']:
            if flag.startswith('-D'):
                name, expected = flag[2:].split('=', 1)
                if actual.get(name) != expected:
                    raise ValueError('Whisper source build option differs: ' + name)
        subprocess.run([str(cmake), '--build', str(output), '--target', 'whisper-cli', '-j', '2'],
                       env=env, check=True)
        destination = stage / 'vulkan/bin'
        destination.mkdir(parents=True)
        shutil.copyfile(output / 'bin/whisper-cli.exe', destination / 'whisper-cli.exe')
        # Probe has no Vulkan import: missing drivers never prevent the CPU route.
        subprocess.run(['cl.exe', '/nologo', '/std:c++17', '/O2', '/MT', '/EHsc', '/utf-8',
                        '/I' + str(includes), str(Path(__file__).with_name('vulkan_probe.cpp')),
                        '/Fe:' + str(stage / 'bin/nero-vulkan-probe.exe'),
                        '/Fo:' + str(folder / 'vulkan_probe.obj')], env=env, cwd=folder, check=True)
        # Check dependency closure from PE metadata, without executing GPU code.
        verify = prepare.load_module('nero_whisper_pe', Path(__file__).with_name('windows_pe.py'))
        verify.verify_closure(stage)
        licenses = stage / 'share/licenses/vulkan-sdk'
        licenses.mkdir(parents=True)
        shutil.copyfile(sdk / 'Licenses/LICENSE.txt', licenses / 'LICENSE.txt')
        return {'toolchainReceiptSha256': prepare.sha(tools / 'nero-toolchain.json'),
                'compiler': {'path': shutil.which('cl.exe'), 'sha256': prepare.sha(Path(shutil.which('cl.exe'))),
                             'msvcVersion': env.get('VCToolsVersion'), 'windowsSdkVersion': env.get('WindowsSDKVersion')},
                'flags': pin['cmakeFlags'], 'cmakeCacheSha256': prepare.sha(output / 'CMakeCache.txt')}
