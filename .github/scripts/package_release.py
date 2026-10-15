"""Package an explicit allowlist; smoke-test the executable being distributed."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import shutil
import struct
import subprocess
import tempfile
import tomllib
import xml.etree.ElementTree as ET
import zipfile


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', required=True)
    parser.add_argument('--platform', required=True,
                        choices=['windows-x86_64', 'macos-aarch64', 'macos-x86_64'])
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    version = tomllib.loads((repo / 'Cargo.toml').read_text())['workspace']['package']['version']
    windows = args.platform.startswith('windows-')
    name = f'rustDatView-v{version}-{args.platform}'
    assets = repo / 'target' / 'release-assets'
    assets.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='rustdatview-release-') as tmp:
        root = Path(tmp) / name
        root.mkdir()
        source = repo / 'target' / args.target / 'release' / ('datview.exe' if windows else 'datview')
        if windows:
            binary = root / 'datview.exe'
        else:
            contents = root / 'rustDatView.app' / 'Contents'
            binary = contents / 'MacOS' / 'datview'
            binary.parent.mkdir(parents=True)
            with (contents / 'Info.plist').open('wb') as output:
                plistlib.dump({
                    'CFBundleExecutable': 'datview',
                    'CFBundleIdentifier': 'io.github.exror.rustdatview',
                    'CFBundleName': 'rustDatView',
                    'CFBundleDisplayName': 'rustDatView',
                    'CFBundlePackageType': 'APPL',
                    'CFBundleShortVersionString': version,
                    'CFBundleVersion': version,
                    'LSMinimumSystemVersion': '12.0',
                    'NSHighResolutionCapable': True,
                }, output)
        shutil.copy2(source, binary)
        if not windows:
            binary.chmod(0o755)
            dependencies = run('otool', '-L', str(binary))
            print(dependencies)
            for line in dependencies.splitlines()[1:]:
                library = line.strip().split(' (', 1)[0]
                if not library.startswith(('/usr/lib/', '/System/Library/')):
                    raise RuntimeError(f'Non-system runtime dependency: {library}')
            run('codesign', '--force', '--sign', '-', str(binary.parents[2]))
            run('codesign', '--verify', '--deep', '--strict', str(binary.parents[2]))
        actual = run(str(binary), '--version').strip()
        if actual != f'rustDatView {version}':
            raise RuntimeError(f'Incorrect release version: {actual}')
        for filename in ('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md'):
            shutil.copy2(repo / filename, root / filename)
        shutil.copytree(repo / 'licenses', root / 'licenses')
        examples = root / 'examples'
        examples.mkdir()
        for filename in ('signals.csv', 'motherboard-power.csv', 'field_lzma.vti'):
            shutil.copy2(repo / 'examples' / filename, examples / filename)
        # Real export smoke, executed from the packaged copy without Rust/Python bindings.
        for extension in ('png', 'svg'):
            output = Path(tmp) / f'smoke.{extension}'
            run(str(binary), str(examples / 'motherboard-power.csv'), '--y', '1,3,4', '--export', str(output))
            if extension == 'png':
                header = output.read_bytes()[:24]
                if header[:8] != b'\x89PNG\r\n\x1a\n' or struct.unpack('>II', header[16:24]) != (1200, 800):
                    raise RuntimeError('Incorrect rendered PNG dimensions')
            elif ET.parse(output).getroot().tag != '{http://www.w3.org/2000/svg}svg':
                raise RuntimeError('Invalid SVG export')
        (root / 'build-info.json').write_text(json.dumps({
            'version': version, 'platform': args.platform, 'target': args.target,
            'commit': os.environ.get('GITHUB_SHA', ''),
            'rustc': run('rustc', '--version').strip(),
            'apple_notarized': False if not windows else None,
        }, indent=2) + '\n', encoding='utf-8')
        archive = assets / f'{name}.zip'
        with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
            for file in sorted(root.rglob('*')):
                if file.is_file():
                    output.write(file, file.relative_to(root.parent))
        print(f'Packaged and smoke-tested {archive.name} ({archive.stat().st_size} bytes)')


if __name__ == '__main__':
    main()
