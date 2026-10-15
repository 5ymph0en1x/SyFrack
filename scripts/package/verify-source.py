"""Verify a prepared source export without changing it. Python standard library only."""
import hashlib
import json
import re
import sys
from pathlib import Path


def main():
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    manifest = json.loads((root / 'SOURCE_MANIFEST.json').read_text(encoding='utf8'))
    expected = {row['path']: row for row in manifest['files']}
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()}
    assert actual == set(expected) | {'SOURCE_MANIFEST.json'}, 'Missing or additional files'
    for name, record in expected.items():
        path = root / name
        assert path.resolve().is_relative_to(root), f'Path escapes tree: {name}'
        assert path.stat().st_size == record['bytes'], f'Size mismatch: {name}'
        with path.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        assert digest == record['sha256'], f'SHA-256 mismatch: {name}'
    assert not (root / '.git').exists(), 'Source preparation must not create a Git history'
    assert manifest['file_count'] == len(expected)
    assert manifest['total_bytes'] == sum(record['bytes'] for record in expected.values())
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
