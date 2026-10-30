#!/usr/bin/env python3
"""Build-time only: pin, verify and bundle native llama.cpp. Never fetch model weights."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT=Path(__file__).resolve().parents[2]
SPEC=json.loads(Path(__file__).with_name('runtime.json').read_text())
MIN_WINDOWS_CRT=(14,44,35211,0)
CRT_NAMES={'concrt140.dll','msvcp140.dll','msvcp140_1.dll','msvcp140_2.dll',
           'msvcp140_atomic_wait.dll','msvcp140_codecvt_ids.dll','vcomp140.dll',
           'vcruntime140.dll','vcruntime140_1.dll'}

def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for data in iter(lambda:f.read(1024*1024),b''):h.update(data)
    return h.hexdigest()

def crt_version(path):
    # VS_FIXEDFILEINFO is PE version metadata, not publisher authentication.
    with path.open('rb') as stream:data=stream.read(4*1024*1024+1)
    if len(data)>4*1024*1024 or not data.startswith(b'MZ'):
        raise ValueError('Invalid Microsoft C++ runtime DLL metadata')
    marker=struct.pack('<II',0xFEEF04BD,0x00010000)
    offset=data.find(marker)
    if offset<0 or offset!=data.rfind(marker) or offset+52>len(data):
        raise ValueError('Ambiguous Microsoft C++ runtime DLL version')
    major_minor,build_revision=struct.unpack_from('<II',data,offset+8)
    return major_minor>>16,major_minor&65535,build_revision>>16,build_revision&65535

def checked_crt(files):
    required={'msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll'}
    if not required<=files.keys():return None
    if any(crt_version(files[name])<MIN_WINDOWS_CRT for name in files):
        raise ValueError('Microsoft C++ runtime is older than the qualified 14.44.35211.0; provide a current licensed x64 redistributable with --msvc-redist')
    return files

def windows_crt(explicit):
    if explicit is not None: roots=[explicit]
    elif os.environ.get('VCToolsRedistDir'): roots=[Path(os.environ['VCToolsRedistDir'])]
    elif os.name=='nt':
        locator=Path(os.environ.get('ProgramFiles(x86)','C:/Program Files (x86)'))/'Microsoft Visual Studio/Installer/vswhere.exe'
        installation=subprocess.check_output([str(locator),'-latest','-products','*','-property','installationPath'],text=True).strip()
        roots=sorted((Path(installation)/'VC/Redist/MSVC').glob('*'),reverse=True)
    else: raise ValueError('Windows packaging requires --msvc-redist from a licensed Windows build toolchain')
    for root in roots:
        candidates=[root]+list(root.glob('x64/Microsoft.VC*.CRT'))
        for folder in candidates:
            files={p.name.lower():p for p in folder.glob('*.dll') if p.name.lower() in CRT_NAMES}
            checked=checked_crt(files)
            if checked:return checked
    raise ValueError('App-local Microsoft C++ runtime DLLs are missing; do not ship this bundle')

def prepare(target, cache, output, redist=None):
    crt=windows_crt(redist) if target.startswith('windows') else {}
    cache.mkdir(parents=True,exist_ok=True)
    if output.exists():
        raise SystemExit('Destination exists; use a new destination. Existing workers are never replaced.')
    output.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='summary-',dir=output.parent) as temporary:
        stage=Path(temporary)/'summary';stage.mkdir()
        for item in SPEC['assets'][target]:
            archive=cache/item['name']
            if not archive.is_file() or archive.stat().st_size!=item['bytes'] or sha(archive)!=item['sha256']:
                partial=cache/(item['name']+'.part')
                request=urllib.request.Request(f"https://github.com/{SPEC['repository']}/releases/download/{SPEC['version']}/{item['name']}",headers={'User-Agent':'Nero-build'})
                with urllib.request.urlopen(request,timeout=60) as r,partial.open('wb') as f:
                    total=0
                    while data:=r.read(1024*1024):
                        total+=len(data)
                        if total>item['bytes']:raise ValueError('Runtime archive exceeded pinned size')
                        f.write(data)
                if partial.stat().st_size!=item['bytes'] or sha(partial)!=item['sha256']:raise ValueError('Runtime archive checksum mismatch')
                partial.replace(archive)
            destination=stage/item['backend'];destination.mkdir()
            # Flatten only runtime files, rejecting duplicate names and archive links.
            def write(name,data):
                base=Path(name).name
                wanted=base in ('llama-server','llama-server.exe') or base.endswith(('.dylib','.dll','.metallib','.metal')) or base.upper().startswith(('LICENSE','COPYING'))
                if not wanted:return
                path=destination/base
                if path.exists():raise ValueError('Duplicate runtime file: '+base)
                with path.open('xb') as f:shutil.copyfileobj(data,f)
                if base=='llama-server':path.chmod(0o755)
            if archive.suffix=='.zip':
                with zipfile.ZipFile(archive) as z:
                    for info in z.infolist():
                        if info.is_dir():continue
                        if (info.external_attr>>16)&0o170000==0o120000:raise ValueError('Linked archive entry')
                        with z.open(info) as f:write(info.filename,f)
            else:
                with tarfile.open(archive) as t:
                    for info in t:
                        # Official macOS tar contains dylib aliases: preserve their bytes under each alias.
                        if info.isfile() or info.issym() or info.islnk():
                            f=t.extractfile(info)
                            if f is not None:
                                with f:write(info.name,f)
            for name,source in crt.items():
                shutil.copyfile(source,destination/name)
            binary=destination/('llama-server.exe' if target.startswith('windows') else 'llama-server')
            if not binary.is_file():raise ValueError('Missing llama-server')
        license_url=f"https://raw.githubusercontent.com/{SPEC['repository']}/{SPEC['commit']}/LICENSE"
        with urllib.request.urlopen(license_url,timeout=30) as r:(stage/'LICENSE-llama.cpp.txt').write_bytes(r.read(65536))
        receipt={'target':target,'runtime':SPEC,'source_archive_hashes_verified':True,'file_hash_scope':'before Developer ID code signing','files':[{'path':str(p.relative_to(stage)),'bytes':p.stat().st_size,'sha256':sha(p)} for p in sorted(stage.rglob('*')) if p.is_file()]}
        (stage/'runtime.json').write_text(json.dumps(receipt,indent=2)+'\n')
        os.rename(stage,output)
    print('Prepared native summary runtime:',target)

def verify_staged(output,target,execute=False):
    if output.is_symlink() or not output.is_dir():raise ValueError('Invalid summary runtime directory')
    receipt_path=output/'runtime.json'
    if receipt_path.is_symlink() or not receipt_path.is_file() or receipt_path.stat().st_size>1024*1024:
        raise ValueError('Invalid summary runtime receipt')
    for path in output.rglob('*'):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError('Linked or special summary runtime entry')
    receipt=json.loads(receipt_path.read_text())
    if receipt.get('target')!=target or receipt.get('runtime')!=SPEC or receipt.get('source_archive_hashes_verified') is not True:
        raise ValueError('Summary runtime receipt differs from the pinned recipe')
    expected=set()
    for item in receipt.get('files',[]):
        relative=Path(item['path'])
        if relative.anchor or '..' in relative.parts or not relative.parts:
            raise ValueError('Invalid summary runtime receipt path')
        path=output/relative
        cursor=output
        for part in relative.parts:
            cursor=cursor/part
            if cursor.is_symlink():raise ValueError('Linked summary runtime file')
        if path in expected or not path.is_file() or path.stat().st_size!=item['bytes'] or sha(path)!=item['sha256']:
            raise ValueError('Summary runtime file differs from its receipt')
        expected.add(path)
    actual={p for p in output.rglob('*') if p.is_file() and p!=output/'runtime.json'}
    if not expected or actual!=expected:raise ValueError('Summary runtime receipt is incomplete')
    for asset in SPEC['assets'][target]:
        directory=output/asset['backend']
        binary=directory/('llama-server.exe' if target.startswith('windows') else 'llama-server')
        if binary not in expected:raise ValueError('Summary runtime executable is missing')
        if target.startswith('windows'):
            crt=checked_crt({p.name.lower():p for p in directory.glob('*.dll') if p.name.lower() in CRT_NAMES})
            if not crt:raise ValueError('Summary runtime is missing its app-local C++ runtime')
        if execute:
            if target.startswith('windows')!=(os.name=='nt'):
                raise ValueError('Execute runtime checks on the matching target OS')
            kwargs={'creationflags':0x08000000} if os.name=='nt' else {}
            result=subprocess.run([str(binary.resolve()),'--version'],cwd=directory,
                                  stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE,timeout=20,**kwargs)
            if result.returncode!=0:raise ValueError(f'Summary {asset["backend"]} startup failed: {result.returncode}')
    print('Verified native summary runtime:',target)

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',required=True,choices=SPEC['assets']);p.add_argument('--msvc-redist',type=Path);p.add_argument('--cache',type=Path,default=ROOT/'.work/summary-runtime-downloads');p.add_argument('--output',type=Path,default=ROOT/'apps/desktop/.generated/native/summary')
    p.add_argument('--verify-staged',type=Path);p.add_argument('--optional',action='store_true');p.add_argument('--execute-runtime',action='store_true');a=p.parse_args()
    if a.verify_staged is not None:
        if a.optional and not a.verify_staged.exists() and not a.verify_staged.is_symlink():
            print('Native summary runtime absent: local summaries unavailable.')
        else:verify_staged(a.verify_staged,a.target,a.execute_runtime)
    else:prepare(a.target,a.cache,a.output,a.msvc_redist)
