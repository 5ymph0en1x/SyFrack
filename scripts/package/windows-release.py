"""Build/verify the portable Windows release. Python 3.11+, Cargo, no pip packages."""
import argparse
import datetime
import hashlib
import json
import os
import posixpath
import re
import struct
import subprocess
import tempfile
import tomllib
import zipfile
from pathlib import Path, PurePosixPath
from source_layout import documentation_files

ROOT = Path(__file__).resolve().parents[2]
TARGET = 'x86_64-pc-windows-msvc'
VERSION_TAG = re.compile(r'v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)')
EXES = {'syfrack-studio.exe': 2, 'syfrack.exe': 3}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def validate_tag(tag, root=ROOT):
    match = VERSION_TAG.fullmatch(tag)
    if not match:
        raise ValueError('Use a version tag such as v0.2.0 or v0.2.0-beta.1.')
    version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf8'))['workspace']['package']['version']
    if match[1] != version:
        raise ValueError(f'Tag {tag} must match workspace.package.version ({version}).')
    return version


def inspect_exe(data, subsystem):
    """Reject wrong architectures/subsystems and a missing external MSVC runtime."""
    if data[:2] != b'MZ':
        raise ValueError('Not a Windows executable.')
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if data[pe:pe + 4] != b'PE\0\0' or struct.unpack_from('<H', data, pe + 4)[0] != 0x8664:
        raise ValueError('A Windows x64 executable is required.')
    optional = pe + 24
    if struct.unpack_from('<H', data, optional)[0] != 0x20b:
        raise ValueError('Expected PE32+ format.')
    if struct.unpack_from('<H', data, optional + 68)[0] != subsystem:
        raise ValueError('Wrong Windows executable subsystem.')
    section_count = struct.unpack_from('<H', data, pe + 6)[0]
    section_start = optional + struct.unpack_from('<H', data, pe + 20)[0]
    sections = []
    for i in range(section_count):
        size, rva, raw_size, offset = struct.unpack_from('<IIII', data, section_start + i * 40 + 8)
        sections.append((rva, max(size, raw_size), offset))

    def file_offset(rva):
        for start, size, offset in sections:
            if start <= rva < start + size:
                return offset + rva - start
        raise ValueError('Invalid PE import address.')

    imports_rva = struct.unpack_from('<I', data, optional + 120)[0]
    imports = []
    if imports_rva:
        offset = file_offset(imports_rva)
        while any(data[offset:offset + 20]):
            name_rva = struct.unpack_from('<I', data, offset + 12)[0]
            name_offset = file_offset(name_rva)
            name = data[name_offset:data.index(b'\0', name_offset)].decode('ascii').lower()
            if name.startswith(('vcruntime', 'msvcp', 'concrt', 'msvcr1')):
                raise ValueError(f'External MSVC runtime import: {name}; build with +crt-static.')
            imports.append(name)
            offset += 20
    return sorted(imports)


def dependency_notices(metadata, root, files):
    """Retain license/notice files, font attribution and a dependency inventory."""
    records = []
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if package['source'] is None:
            continue
        folder = Path(package['manifest_path']).parent
        notices = []
        for path in sorted(folder.rglob('*')):
            name = path.name.lower()
            legal = name.startswith(('license', 'licence', 'copying', 'notice', 'copyright', 'unlicense'))
            font_legal = 'fonts' in path.parts and path.suffix == '.txt'
            if path.is_file() and (legal or font_legal):
                relative = path.relative_to(folder).as_posix()
                destination = f'licenses/{package["name"]}-{package["version"]}/{relative}'
                files[destination] = path.read_bytes()
                notices.append(destination)
        selected_license = package.get('license')
        if not notices or package['name'] == 'epaint_default_fonts':
            if 'Apache-2.0' in (selected_license or ''):
                destination = 'licenses/shared/Apache-2.0.txt'
                files[destination] = (root / 'assets/licenses/Apache-2.0.txt').read_bytes()
                notices.append(destination)
                selected_license = 'Apache-2.0' + (' AND bundled font licenses' if package['name'] == 'epaint_default_fonts' else '')
            else:
                fallback = root / 'assets/licenses' / f'{package["name"]}-{package["version"]}.txt'
                if not fallback.is_file():
                    raise ValueError(f'Missing license text for {package["name"]} {package["version"]}.')
                destination = f'licenses/{fallback.name}'
                files[destination] = fallback.read_bytes()
                notices.append(destination)
        records.append({
            'name': package['name'], 'version': package['version'],
            'license': package.get('license'), 'selected_license': selected_license,
            'authors': package.get('authors', []), 'repository': package.get('repository'),
            'notices': notices,
        })
    files['licenses/DEPENDENCIES.json'] = (json.dumps(records, indent=2) + '\n').encode()


def build(tag, revision, target_dir, output, root=ROOT):
    version = validate_tag(tag, root)
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        raise ValueError('A full Git commit SHA is required.')
    files = {}
    imports = {}
    for name, subsystem in EXES.items():
        data = (target_dir / TARGET / 'release' / name).read_bytes()
        imports[name] = inspect_exe(data, subsystem)
        files[name] = data
    for name in ['LICENSE', 'THIRD_PARTY_NOTICES.md', 'Cargo.lock']:
        files[name] = (root / name).read_bytes()
    for folder in ['scenes', 'graphs', 'assets']:
        for path in sorted((root / folder).rglob('*')):
            if path.is_symlink():
                raise ValueError(f'Symlink refused: {path}')
            if path.is_file() and '__pycache__' not in path.parts:
                # The source export contains curated content only. Include all its assets.
                files[path.relative_to(root).as_posix()] = path.read_bytes()
    for name, path in documentation_files(root).items():
        files[name] = path.read_bytes()
    files['scripts/install-oidn.py'] = (root / 'scripts/install-oidn.py').read_bytes()
    files['scripts/with-vulkan-cpu.ps1'] = (root / 'scripts/with-vulkan-cpu.ps1').read_bytes()
    metadata = json.loads(subprocess.check_output([
        'cargo', 'metadata', '--offline', '--locked', '--format-version', '1',
        '--filter-platform', TARGET, '--manifest-path', str(root / 'Cargo.toml'),
    ], text=True, encoding='utf8'))
    dependency_notices(metadata, root, files)
    rustc = subprocess.check_output(['rustc', '--version'], text=True).strip()
    files['BUILD_INFO.json'] = (json.dumps({
        'version': version, 'tag': tag, 'revision': revision, 'target': TARGET,
        'profile': 'release', 'rustc': rustc, 'c_runtime': 'static',
        'cpu': 'generic x86-64; no target-cpu=native', 'imports': imports,
        'built_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    }, indent=2) + '\n').encode()
    files['README.txt'] = f'''SyFrack Studio {tag} - experimental Windows x64 release

1. Extract the ENTIRE ZIP into a writable folder.
2. Double-click syfrack-studio.exe. No Rust or Python installation is needed.
3. Change language under View > Language > English / Francais.
4. Keep scenes/, graphs/ and assets/ beside the executables.
5. Choose Interface > Simple / Advanced; the interface choice is saved locally.

The CLI is syfrack.exe. From this folder in PowerShell:
  .\\syfrack.exe --help
  .\\syfrack.exe probe
  .\\syfrack.exe render scenes/mandelbulb.ron --out renders/example --width 1280 --height 720 --samples 64 --bounces 4 --steps 512 --denoise

A compatible Vulkan or Direct3D 12 GPU/driver is required for Studio.
The CLI additionally tries OpenGL. Update your GPU driver if startup fails.
The C runtime is linked statically; no separate VC++ runtime is bundled.
The native denoiser works without an external runtime. OIDN is optional and
is not included. See docs/BUILDING.md for a separate OIDN installation.

Unsigned experimental executables may trigger a Windows reputation prompt.
No code-signing certificate is used. SHA256SUMS.txt checks download integrity;
it is not a code signature. See LICENSE and licenses/ for third-party notices.
GPU performance and compatibility vary. See docs/USER_GUIDE.md to get started.

Source tag: {tag}
Source commit: {revision}
'''.encode()
    records = [{'path': name, 'bytes': len(data), 'sha256': sha(data)} for name, data in sorted(files.items())]
    files['PACKAGE_MANIFEST.json'] = (json.dumps({'files': records}, indent=2) + '\n').encode()
    stem = f'SyFrack-{tag}-windows-x64'
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f'{stem}.zip'
    if archive.exists():
        raise FileExistsError(f'Preserve the existing archive before rebuilding: {archive}')
    with zipfile.ZipFile(archive, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as bundle:
        for name, data in sorted(files.items()):
            bundle.writestr(f'{stem}/{name}', data)
    (output / 'SHA256SUMS.txt').write_bytes(f'{sha(archive.read_bytes())}  {archive.name}\n'.encode())
    (output / 'RELEASE_NOTES.md').write_text(f'''Experimental SyFrack Studio {tag}.

Download **{archive.name}**, extract the entire archive and launch
**syfrack-studio.exe**. Studio, the CLI, example scenes, graph library, HDRI
assets and third-party notices are included. Rust and Python are not needed.

Windows x64; Studio requires a compatible Vulkan or Direct3D 12 GPU/driver.
English and French are selectable under **View → Language**.
The build uses the optimized release profile, generic x86-64 instructions
and a statically linked C runtime. OIDN is optional and is not bundled.
The executables are unsigned. GPU performance/compatibility vary.

**SHA256SUMS.txt** contains the ZIP download checksum. The ZIP also includes
**BUILD_INFO.json** and a per-file **PACKAGE_MANIFEST.json**.

Source commit: `{revision}`.
''', encoding='utf8')
    print(f'Packaged {archive.name}: {len(files)} files, {archive.stat().st_size / 1048576:.2f} MiB')
    return archive


def verify(archive, smoke_test=False):
    checksum = (archive.parent / 'SHA256SUMS.txt').read_text(encoding='utf8').strip()
    if checksum != f'{sha(archive.read_bytes())}  {archive.name}':
        raise ValueError('Release ZIP SHA-256 mismatch.')
    with zipfile.ZipFile(archive) as bundle:
        names = bundle.namelist()
        if len(names) != len(set(names)):
            raise ValueError('Duplicate archive entries.')
        prefix = archive.stem + '/'
        for name in names:
            if not name.startswith(prefix) or '..' in PurePosixPath(name).parts or '\\' in name:
                raise ValueError(f'Unsafe archive entry: {name}')
        manifest = json.loads(bundle.read(prefix + 'PACKAGE_MANIFEST.json'))
        expected = {prefix + r['path'] for r in manifest['files']} | {prefix + 'PACKAGE_MANIFEST.json'}
        if set(names) != expected:
            raise ValueError('Missing or additional archive files.')
        for row in manifest['files']:
            data = bundle.read(prefix + row['path'])
            if len(data) != row['bytes'] or sha(data) != row['sha256']:
                raise ValueError(f'Archive inventory mismatch: {row["path"]}')
        for name, subsystem in EXES.items():
            inspect_exe(bundle.read(prefix + name), subsystem)
        info = json.loads(bundle.read(prefix + 'BUILD_INFO.json'))
        if info['target'] != TARGET or info['profile'] != 'release' or info['c_runtime'] != 'static':
            raise ValueError('Incorrect release build configuration.')
        for folder in ['scenes/', 'graphs/', 'assets/', 'licenses/']:
            if not any(name.startswith(prefix + folder) for name in names):
                raise ValueError(f'Content folder missing: {folder}')
        for name in names:
            if name.endswith('.md'):
                for link in re.findall(r'\]\(([^)]+)\)', bundle.read(name).decode('utf8')):
                    if '://' in link or link.startswith(('#', 'mailto:')):
                        continue
                    target = posixpath.normpath(posixpath.join(posixpath.dirname(name), link.split('#', 1)[0]))
                    if target not in names:
                        raise ValueError(f'Broken packaged documentation link: {name}: {link}')
        if smoke_test:
            if os.name != 'nt':
                raise ValueError('Executable smoke test requires Windows.')
            with tempfile.TemporaryDirectory(prefix='syfrack-release-') as temp:
                bundle.extractall(temp)
                unpacked = Path(temp) / archive.stem
                result = subprocess.run([str(unpacked / 'syfrack.exe'), '--help'], cwd=unpacked,
                                        capture_output=True, text=True, timeout=30, check=True)
                if '--lang' not in result.stdout or 'render' not in result.stdout:
                    raise ValueError('CLI help smoke test failed.')
    print(f'PASS: archive SHA-256, full inventory, x64 binaries and bundled content ({len(names)} files).')
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    validate = commands.add_parser('validate-tag')
    validate.add_argument('--tag', required=True)
    create = commands.add_parser('build')
    create.add_argument('--tag', required=True)
    create.add_argument('--revision', required=True)
    create.add_argument('--target-dir', type=Path, default=ROOT / 'target')
    create.add_argument('--output-dir', type=Path, default=ROOT / 'dist')
    check = commands.add_parser('verify')
    check.add_argument('--archive', type=Path, required=True)
    check.add_argument('--smoke-test', action='store_true')
    args = parser.parse_args()
    if args.command == 'validate-tag':
        print(f'Version OK: {validate_tag(args.tag)}')
    elif args.command == 'build':
        build(args.tag, args.revision, args.target_dir, args.output_dir)
    else:
        verify(args.archive, args.smoke_test)


if __name__ == '__main__':
    main()
