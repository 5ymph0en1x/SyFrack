"""Publish verified build artifacts with gh. Never replace a published release."""
import argparse
import hashlib
import json
import os
import re
import subprocess
import zipfile
from pathlib import Path
from urllib.parse import quote


def gh(*args):
    return subprocess.check_output(['gh', *args], text=True, encoding='utf8', stderr=subprocess.PIPE)


def remote_revision(repo, tag):
    ref = json.loads(gh('api', f'repos/{repo}/git/ref/tags/{quote(tag, safe="")}'))['object']
    for _ in range(10):
        if ref['type'] == 'commit':
            return ref['sha']
        if ref['type'] != 'tag':
            break
        ref = json.loads(gh('api', f'repos/{repo}/git/tags/{ref["sha"]}'))['object']
    raise ValueError('Release tag does not resolve to a commit.')


def publish(tag, revision, assets, repo):
    if not re.fullmatch(r'v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', tag):
        raise ValueError('Invalid version tag.')
    if not re.fullmatch(r'[0-9a-f]{40}', revision) or not re.fullmatch(r'[\w.-]+/[\w.-]+', repo):
        raise ValueError('Invalid commit or repository.')
    archive = assets / f'SyFrack-{tag}-windows-x64.zip'
    checksum = assets / 'SHA256SUMS.txt'
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if checksum.read_text(encoding='utf8').strip() != f'{digest}  {archive.name}':
        raise ValueError('Refusing to publish: download checksum mismatch.')
    with zipfile.ZipFile(archive) as bundle:
        info = json.loads(bundle.read(archive.stem + '/BUILD_INFO.json'))
    if (info['tag'], info['revision'], info['target'], info['profile'], info['c_runtime']) != (
            tag, revision, 'x86_64-pc-windows-msvc', 'release', 'static'):
        raise ValueError('Artifact metadata does not match the requested release.')
    if remote_revision(repo, tag) != revision:
        raise ValueError('The remote tag moved after compilation; publication refused.')
    try:
        release = json.loads(gh('api', f'repos/{repo}/releases/tags/{quote(tag, safe="")}'))
    except subprocess.CalledProcessError as error:
        if 'HTTP 404' not in error.stderr:
            raise
        release = None
    if release is not None and not release['draft']:
        raise ValueError('This release is already published. Preserve its assets and use a new version tag.')
    if release is None:
        args = ['release', 'create', tag, '--repo', repo, '--verify-tag', '--draft',
                '--title', f'SyFrack Studio {tag}', '--notes-file', str(assets / 'RELEASE_NOTES.md')]
        if '-' in tag:
            args.append('--prerelease')
        gh(*args)
    # Drafts can be retried after a partial upload. Published assets are never clobbered.
    gh('release', 'upload', tag, str(archive), str(checksum), '--repo', repo, '--clobber')
    gh('release', 'edit', tag, '--repo', repo, '--draft=false')
    print(f'Published {tag}: {archive.name} and SHA256SUMS.txt')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--assets', type=Path, required=True)
    args = parser.parse_args()
    publish(args.tag, args.revision, args.assets, os.environ['GITHUB_REPOSITORY'])


if __name__ == '__main__':
    main()
