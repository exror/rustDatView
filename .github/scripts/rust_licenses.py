"""Write the licenses of every Rust crate linked into datview.exe to one text file.

Usage: python .github/scripts/rust_licenses.py [OUTPUT]
Reads `cargo metadata` for the x86_64-pc-windows-msvc target, so only crates
that can end up in the Windows executable are listed. License texts are taken
from each crate's published sources.
"""
import json
from pathlib import Path
import subprocess
import sys

LICENSE_NAMES = ('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'UNLICENSE')


def main():
    repo = Path(__file__).resolve().parents[2]
    output = Path(sys.argv[1]) if len(sys.argv) > 1 else repo / 'licenses' / 'RUST-DEPENDENCIES.txt'
    metadata = json.loads(subprocess.run(
        ['cargo', 'metadata', '--format-version', '1', '--locked',
         '--filter-platform', 'x86_64-pc-windows-msvc'],
        cwd=repo, check=True, capture_output=True, text=True).stdout)
    packages = {package['id']: package for package in metadata['packages']}
    nodes = {node['id']: node for node in metadata['resolve']['nodes']}
    root = next(package['id'] for package in metadata['packages'] if package['name'] == 'datview')
    # Follow normal (non-dev, non-build) dependencies from the application.
    seen, stack = set(), [root]
    while stack:
        current = stack.pop()
        if current in seen:
            continue
        seen.add(current)
        for dependency in nodes[current]['deps']:
            if any(kind['kind'] is None for kind in dependency['dep_kinds']):
                stack.append(dependency['pkg'])
    lines = [
        'Licenses of the Rust crates compiled into rustDatView.exe',
        '==========================================================',
        '',
    ]
    for package in sorted((packages[i] for i in seen if packages[i]['source']), key=lambda p: (p['name'], p['version'])):
        directory = Path(package['manifest_path']).parent
        texts = sorted(path for path in directory.iterdir()
                       if path.is_file() and path.name.upper().startswith(LICENSE_NAMES))
        lines.append('-' * 78)
        lines.append(f"{package['name']} {package['version']} — {package.get('license') or 'see license file'}")
        if package.get('repository'):
            lines.append(package['repository'])
        lines.append('')
        if texts:
            for text in texts:
                lines.append(f'[{text.name}]')
                lines.append(text.read_text(encoding='utf-8', errors='replace').strip())
                lines.append('')
        else:
            lines.append('(No license file is published with this crate; terms are given by the SPDX expression above.)')
            lines.append('')
    output.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    print(f'Wrote {output} ({len(seen) - 1} crates)')


if __name__ == '__main__':
    main()
