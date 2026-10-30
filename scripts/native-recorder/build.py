from pathlib import Path
import argparse
import re
import subprocess


def build(root, output, target):
    output.mkdir(parents=True, exist_ok=True)
    css = (root / 'design-system/desktop-preview.css').read_text()
    typography = (root / 'design-system/typography.css').read_text()
    colors = []
    for name in ['surface', 'text', 'muted', 'edge', 'recording', 'warning']:
        match = re.search(r'--overlay-' + name + r':\s*#([0-9a-fA-F]{6});', css)
        if not match:
            raise ValueError('Missing overlay design token: ' + name)
        color = match[1]
        channels = [int(color[i:i+2], 16) / 255 for i in (0, 2, 4)]
        colors.append(f'    static let {name} = NSColor(srgbRed: {channels[0]}, green: {channels[1]}, blue: {channels[2]}, alpha: 1)')
    caption = re.search(r'--text-caption-size:\s*(\d+)px;', typography)
    if not caption:
        raise ValueError('Missing caption typography token')
    tokens = output / 'RecorderTokens.swift'
    tokens.write_text('import AppKit\nenum RecorderTokens {\n' + '\n'.join(colors) + f'\n    static let caption = CGFloat({caption[1]})\n}}\n')
    native = root / 'apps/desktop/src-tauri/native/recorder'
    sources = [native / 'RecorderPlacement.swift', native / 'RecorderPanel.swift', tokens]
    swift_target = {'aarch64-apple-darwin': 'arm64-apple-macos26.0', 'x86_64-apple-darwin': 'x86_64-apple-macos26.0'}[target]
    obj = output / 'RecorderPanel.o'
    subprocess.run(['xcrun', 'swiftc', '-swift-version', '5', '-O', '-whole-module-optimization', '-parse-as-library', '-emit-object', '-target', swift_target, *map(str, sources), '-o', str(obj)], check=True)
    subprocess.run(['xcrun', 'ar', 'rcs', str(output / 'libnero_recorder_panel.a'), str(obj)], check=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--target', required=True)
    args = parser.parse_args()
    build(Path(__file__).resolve().parents[2], args.out, args.target)
