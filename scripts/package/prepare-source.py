"""Prepare a standalone GitHub source tree locally; never initializes Git or uploads.

Usage: python scripts/package/prepare-source.py
Only explicitly selected source files/assets are copied into <checkout>/to_github.
Reruns update managed files only when the previous exported copy is unchanged.
"""
import datetime
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEST = ROOT / 'to_github'
MANIFEST = 'SOURCE_MANIFEST.json'
ROOT_FILES = [
    'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.gitignore', '.gitattributes',
    'README.md', 'README.fr.md', 'LICENSE', 'CHANGELOG.md', 'CONTRIBUTING.md',
    'THIRD_PARTY_NOTICES.md',
]
DOCS = ['USER_GUIDE.md', 'BUILDING.md', 'ARCHITECTURE.md', 'LOCALIZATION.md',
        'PUBLISHING.md', 'RELEASE_VERIFICATION.md']
SCRIPTS = ['build.ps1', 'install-oidn.py', 'package/prepare-source.py',
           'package/verify-source.py', 'package/windows-release.py',
           'package/publish-release.py', 'package/test_release.py',
           'i18n/audit.py', 'i18n/rust_strings.py', 'i18n/test_rust_strings.py']
EXTENSIONS = {'.rs', '.wgsl', '.toml', '.lock', '.tsv', '.ttf', '.py', '.md',
              '.ron', '.syf', '.png', '.hdr', '.json', '.name', '.yml', '.yaml', '.txt'}
TEXT = {'.rs', '.wgsl', '.toml', '.lock', '.tsv', '.py', '.md', '.ron', '.syf',
        '.json', '.name', '.yml', '.yaml', '.ps1', '.txt'}
FORBIDDEN_PARTS = {'.git', 'target', 'old', 'runtime', 'dist', '__pycache__',
                   '.codex', '.agents', '.remember', '.superpowers', 'node_modules'}
PERSONAL_PATH = re.compile(r'(?i)[a-z]:[/\\]+(?:Users|Documents and Settings)[/\\]+')
TOKEN = re.compile(r'(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)')


def sha(path):
    return hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()


def exported_bytes(path):
    data = path.read_bytes()
    if path.suffix in TEXT or path.name in ROOT_FILES:
        data = data.replace(b'\r\n', b'\n')
    return data


def inputs():
    selected = {}
    def add(path, relative=None):
        path = Path(path)
        if not path.is_file():
            raise RuntimeError(f'Required source file missing: {path.relative_to(ROOT)}')
        if path.is_symlink() or not path.resolve().is_relative_to(ROOT.resolve()):
            raise RuntimeError(f'External/symlink source refused: {path}')
        relative = Path(relative) if relative is not None else path.relative_to(ROOT)
        if relative.is_absolute() or '..' in relative.parts or set(relative.parts) & FORBIDDEN_PARTS:
            raise RuntimeError(f'Unsafe export path: {relative}')
        if path.stat().st_size >= 99 * 1024 * 1024:
            raise RuntimeError(f'File exceeds source export limit: {relative}')
        if path.suffix in TEXT:
            text = path.read_text(encoding='utf-8-sig')
            local_paths = {str(ROOT), ROOT.as_posix(), str(ROOT).replace('\\', '\\\\')}
            if PERSONAL_PATH.search(text) or TOKEN.search(text) or any(p in text for p in local_paths):
                raise RuntimeError(f'Personal path or token pattern found: {relative}')
        selected[relative.as_posix()] = path
    for name in ROOT_FILES:
        add(ROOT / name)
    for folder in ['crates', 'graphs', 'assets', '.github']:
        for path in sorted((ROOT / folder).rglob('*')):
            if path.is_file() and path.suffix in EXTENSIONS and not set(path.relative_to(ROOT).parts) & FORBIDDEN_PARTS:
                add(path)
    for path in sorted((ROOT / 'scenes').glob('*')):
        if path.is_file() and path.suffix in {'.ron', '.syf', '.md'}:
            add(path)
    for folder in ['environments', 'materials', 'showcase']:
        for path in sorted((ROOT / 'scenes' / folder).rglob('*')):
            if path.is_file() and path.suffix in {'.ron', '.syf', '.md'}:
                add(path)
    for name in DOCS:
        add(ROOT / 'docs' / name)
    for name in ['breath-of-the-titans.jpg', 'studio-en.png', 'studio-fr.png', 'studio-2d-en.png']:
        add(ROOT / 'docs/images' / name)
    for name in SCRIPTS:
        add(ROOT / 'scripts' / name)
    # Check the assets against the provenance supplied with this checkout.
    for path in (ROOT / 'assets/hdri').glob('*.source.json'):
        provenance = json.loads(path.read_text(encoding='utf8'))
        asset = path.with_name(path.name.replace('.source.json', '.hdr'))
        if 'md5' in provenance:
            actual = hashlib.file_digest(asset.open('rb'), 'md5').hexdigest()
            if actual != provenance['md5']:
                raise RuntimeError(f'HDRI provenance checksum mismatch: {asset.name}')
    return selected


def main():
    root = ROOT.resolve()
    destination = DEST.resolve()
    if destination == root or not destination.is_relative_to(root) or DEST.is_symlink():
        raise RuntimeError('Export destination must be a child of the checkout')
    selected = inputs()
    manifest_path = DEST / MANIFEST
    if manifest_path.is_symlink() or not manifest_path.resolve().is_relative_to(destination):
        raise RuntimeError('Manifest path must remain inside the export')
    previous = {}
    if manifest_path.exists():
        previous = {row['path']: row for row in json.loads(manifest_path.read_text(encoding='utf8'))['files']}
    # Validate the entire destination before the first mutation.
    conflicts = []
    for name, source in selected.items():
        target = DEST / name
        if target.is_symlink() or not target.resolve().is_relative_to(destination):
            raise RuntimeError(f'Export path escapes destination: {name}')
        if target.exists():
            digest = sha(target)
            source_digest = hashlib.sha256(exported_bytes(source)).hexdigest()
            if digest not in {source_digest, sha(source), previous.get(name, {}).get('sha256')}:
                conflicts.append(name)
    unmanaged = [p.relative_to(DEST).as_posix() for p in DEST.rglob('*')
                 if p.is_file() and p.relative_to(DEST).as_posix() not in selected and p.name != MANIFEST]
    if conflicts or unmanaged:
        raise RuntimeError(f'Export contains independently changed or unmanaged files; nothing copied: {conflicts + unmanaged}')
    rows = []
    for name, source in sorted(selected.items()):
        target = DEST / name
        target.parent.mkdir(parents=True, exist_ok=True)
        data = exported_bytes(source)
        if not target.exists() or sha(target) != hashlib.sha256(data).hexdigest():
            target.write_bytes(data)
        rows.append({'path': name, 'bytes': target.stat().st_size, 'sha256': sha(target)})
    manifest = {
        'format': 1,
        'generated_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'purpose': 'Local source preparation; no Git initialization, remote or upload',
        'file_count': len(rows), 'total_bytes': sum(row['bytes'] for row in rows), 'files': rows,
    }
    manifest_path.write_bytes((json.dumps(manifest, indent=2) + '\n').encode('utf8'))
    print(f'Prepared {len(rows)} files, {manifest["total_bytes"] / 1048576:.2f} MiB in {DEST}')


if __name__ == '__main__':
    main()
