"""Resolve the local files needed by the public documentation."""
import re
from pathlib import Path
from urllib.parse import unquote, urlsplit

DOCS = (
    'USER_GUIDE.md', 'BUILDING.md', 'ARCHITECTURE.md', 'LOCALIZATION.md',
    'PUBLISHING.md', 'RELEASE_VERIFICATION.md', 'nature-cosmos.md',
    'ocean-surface.md', 'palettes-et-orbites.md',
)

# Explicit source assets and profiling/test inputs, rather than working renders.
EXPLICIT_INPUTS = tuple(
    f'crates/syfrack-render/tests/fixtures/parity/{name}.exr'
    for name in ('mandelbox', 'mandelbulb', 'menger')
) + tuple(
    f'docs/benchmarks/guiding-cost-2026-10-09/fixtures/{name}/{file}'
    for name in ('close', 'menger', 'room')
    for file in ('scene.ron', 'reference.exr')
) + (
    'docs/benchmarks/guiding-cost-2026-10-09/fixtures/menger/studio.exr',
    'scenes/environments/neon-night.exr',
)


def documentation_files(root, seeds=None):
    root = Path(root).resolve()
    pending = [root / name for name in (seeds if seeds is not None else
        ['README.md', 'README.fr.md', *('docs/' + name for name in DOCS)])]
    found = {}
    while pending:
        path = pending.pop()
        if path.is_symlink() or not path.resolve().is_relative_to(root):
            raise ValueError(f'Documentation link escapes the source tree: {path.name}')
        path = path.resolve()
        name = path.relative_to(root).as_posix()
        if name in found:
            continue
        if not path.is_file():
            raise ValueError(f'Missing documentation dependency: {name}')
        found[name] = path
        if path.suffix.lower() != '.md':
            continue
        for link in re.findall(r'\]\(([^)]+)\)', path.read_text(encoding='utf8')):
            link = link.strip()
            if link.startswith('<'):
                link = link[1:link.index('>')]
            else:
                link = re.split(r'\s+[\'\"]', link, maxsplit=1)[0]
            parts = urlsplit(link)
            if parts.scheme or parts.netloc or not parts.path:
                continue
            pending.append(path.parent / unquote(parts.path))
    return found
