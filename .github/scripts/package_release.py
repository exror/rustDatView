"""Package an explicit allowlist for Windows and smoke-test the executable being shipped."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import tomllib
import xml.etree.ElementTree as ET
import zipfile

EXAMPLES = (
    'cpu-gaming-session.csv',
    'cpu-benchmark-eco.csv',
    'cpu-benchmark-performance.csv',
    'motherboard-power.csv',
    'signals.csv',
    'nvidia-smi-two-gpus.csv',
    'field_lzma.vti',
)


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', default='x86_64-pc-windows-msvc')
    parser.add_argument('--binary', help='Executable to package (default: target/<target>/release/datview.exe)')
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    version = tomllib.loads((repo / 'Cargo.toml').read_text())['workspace']['package']['version']
    name = f'rustDatView-v{version}-windows-x86_64'
    assets = repo / 'target' / 'release-assets'
    assets.mkdir(parents=True, exist_ok=True)
    source = Path(args.binary) if args.binary else repo / 'target' / args.target / 'release' / 'datview.exe'
    with tempfile.TemporaryDirectory(prefix='rustdatview-release-') as tmp:
        root = Path(tmp) / name
        root.mkdir()
        binary = root / 'rustDatView.exe'
        shutil.copy2(source, binary)
        actual = run(str(binary), '--version').strip()
        if actual != f'rustDatView {version}':
            raise RuntimeError(f'Incorrect release version: {actual}')
        # The embedded manifest keeps long path support and stops Windows from
        # opening a console window next to the app when it is started from
        # Explorer (Windows 11 24H2 and later honour the console setting).
        data = binary.read_bytes()
        for setting in (b'longPathAware', b'consoleAllocationPolicy'):
            if setting not in data:
                raise RuntimeError(f'The executable manifest is missing {setting.decode()}')
        for filename in ('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'CHANGELOG.md'):
            shutil.copy2(repo / filename, root / filename)
        if not (repo / 'licenses' / 'RUST-DEPENDENCIES.txt').is_file():
            raise RuntimeError('Run .github/scripts/rust_licenses.py before packaging')
        shutil.copytree(repo / 'licenses', root / 'licenses')
        shutil.copytree(repo / 'docs', root / 'docs')
        examples = root / 'examples'
        examples.mkdir()
        for filename in EXAMPLES:
            shutil.copy2(repo / 'examples' / filename, examples / filename)
        # Real exports from the packaged copy, without Rust or Python installed.
        for extension in ('png', 'svg', 'pdf', 'py'):
            output = Path(tmp) / f'smoke.{extension}'
            run(str(binary), str(examples / 'motherboard-power.csv'), '--export', str(output))
            data = output.read_bytes()
            if extension == 'png':
                if data[:8] != b'\x89PNG\r\n\x1a\n' or struct.unpack('>II', data[16:24]) != (1600, 1000):
                    raise RuntimeError('Incorrect rendered PNG')
            elif extension == 'svg':
                if ET.fromstring(data).tag != '{http://www.w3.org/2000/svg}svg':
                    raise RuntimeError('Invalid SVG export')
            elif extension == 'pdf':
                if not data.startswith(b'%PDF-'):
                    raise RuntimeError('Invalid PDF export')
            elif b'import' not in data:
                raise RuntimeError('Invalid Python script export')
        (root / 'build-info.json').write_text(json.dumps({
            'version': version,
            'platform': 'windows-x86_64',
            'target': args.target,
            'commit': os.environ.get('GITHUB_SHA', ''),
            'rustc': run('rustc', '--version').strip(),
        }, indent=2) + '\n', encoding='utf-8')
        archive = assets / f'{name}.zip'
        with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
            for file in sorted(root.rglob('*')):
                if file.is_file():
                    output.write(file, file.relative_to(root.parent))
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        # LF endings, so `sha256sum -c SHA256SUMS.txt` works everywhere.
        (assets / 'SHA256SUMS.txt').write_text(f'{digest}  {archive.name}\n', encoding='utf-8', newline='\n')
        print(f'Packaged and smoke-tested {archive.name} ({archive.stat().st_size} bytes, sha256 {digest})')


if __name__ == '__main__':
    main()
