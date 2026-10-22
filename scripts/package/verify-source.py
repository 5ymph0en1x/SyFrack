"""Verify a prepared source export without changing it. Python standard library only."""
import hashlib
import json
import re
import sys
from pathlib import Path, PurePosixPath
from source_layout import EXPLICIT_INPUTS, documentation_files


def main():
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    manifest = json.loads((root / 'SOURCE_MANIFEST.json').read_text(encoding='utf8'))
    assert manifest['format'] == 1, 'Unsupported inventory format'
    expected = {row['path']: row for row in manifest['files']}
    assert len(expected) == len(manifest['files']), 'Duplicate inventory paths'
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()}
    assert actual == set(expected) | {'SOURCE_MANIFEST.json'}, 'Missing or additional files'
    for name, record in expected.items():
        relative = PurePosixPath(name)
        assert not relative.is_absolute() and '..' not in relative.parts and '\\' not in name and ':' not in name, f'Unsafe inventory path: {name}'
        path = root / name
        assert not path.is_symlink() and path.resolve().is_relative_to(root), f'Path escapes tree: {name}'
        assert path.stat().st_size == record['bytes'], f'Size mismatch: {name}'
        with path.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        assert digest == record['sha256'], f'SHA-256 mismatch: {name}'
    assert not (root / '.git').exists(), 'Source preparation must not create a Git history'
    assert manifest['file_count'] == len(expected)
    assert manifest['total_bytes'] == sum(record['bytes'] for record in expected.values())
    forbidden = {'.git', 'target', 'runtime', '__pycache__', '.codex', '.agents', '.remember', '.superpowers', 'node_modules'}
    for path in root.rglob('*'):
        assert not path.is_symlink() and path.resolve().is_relative_to(root), 'Unsafe path in export'
        assert not (set(path.relative_to(root).parts) & forbidden), 'Local cache/state in export'
        relative = path.relative_to(root).as_posix()
        assert path.suffix.lower() not in {'.exe', '.dll', '.pdb', '.zip', '.exr', '.lnk', '.pyc'} or relative in EXPLICIT_INPUTS, 'Build/render artifact in source export'
    documentation_files(root)
    # Only local relative links in documentation; ordinary web links are left alone.
    for path in root.rglob('*.md'):
        for link in re.findall(r'\]\(([^)]+)\)', path.read_text(encoding='utf8')):
            if '://' in link or link.startswith(('#', 'mailto:')):
                continue
            target = link.split('#', 1)[0]
            if target:
                assert (path.parent / target).exists(), f'Broken documentation link in {path.relative_to(root)}: {link}'
    print(f'PASS: {len(expected)} files, all SHA-256 hashes and local documentation links verified.')


if __name__ == '__main__':
    main()
