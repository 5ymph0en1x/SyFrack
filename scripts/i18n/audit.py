"""Validate the English/French catalog and audit Studio/scene diagnostic strings.

Run from any directory: python scripts/i18n/audit.py
Uses only Python's standard library. Does not rewrite sources or translations.
"""
import collections
import json
import re
import string
import sys
from pathlib import Path
sys.dont_write_bytecode = True
from rust_strings import strings, production_source

ROOT = Path(__file__).resolve().parents[2]
# These are stable constants / match keys. Their consumers translate at draw
# time, so changing language never freezes cached library or texture captions.
RAW_KEYS = {
    'graph_edit.rs': {'Transformations', 'Boucle et pièges', 'Primitives', 'Combinaisons'},
    'library.rs': {'Boîte bulbe', 'Cathédrale', 'Dentelle inversée', 'Évidement', 'Menger kaléido', 'Menger torsadé', 'Rosace plissée', 'Sphère'},
    'preview.rs': {'forme incomplète'},
    'ui/render_panel.rs': {'Étude', 'Haute qualité'},
    'ui/floating.rs': {'Rendre / Exporter', 'Enregistrer la forme', 'Prendre en main le Studio'},
    'ui/panels/material.rs': {'Piège 1', 'Piège 2', 'Piège 3', 'Piège 4', 'Itérations', 'Normale', 'Mélange des orbites', 'Aucun', 'Menger kaléido', 'Cathédrale', 'Boîte bulbe', 'Évidement', 'Sphère'},
    'ui/form_panel/inspector.rs': {'Norme', 'Plan Y', 'Norme logarithmique'},
    'ui/render_panel/layout.rs': {'Carré', 'Paysage'},
}


def placeholders(text):
    fields = []
    for _, name, spec, conversion in string.Formatter().parse(text):
        if name is not None:
            fields.append((name, spec, conversion))
    return collections.Counter(fields)


def main():
    errors, catalog = [], {}
    path = ROOT / 'crates/syfrack-i18n/messages.tsv'
    for line, row in enumerate(path.read_text(encoding='utf8').splitlines(), 1):
        if not row or row.startswith('#'):
            continue
        try:
            key, en, fr = (json.loads(c) for c in row.split('\t'))
            assert key and en and fr, 'empty field'
            assert key not in catalog, f'duplicate key: {key}'
            assert placeholders(en) == placeholders(fr), f'format fields differ: {key}'
            catalog[key] = (en, fr)
        except (ValueError, AssertionError) as error:
            errors.append(f'{path.name}:{line}: {error}')
    app = ROOT / 'crates/syfrack-app/src'
    core = ROOT / 'crates/syfrack-core/src'
    for path in sorted(list(app.rglob('*.rs')) + list(core.rglob('*.rs'))):
        if path.name == 'tests.rs':
            continue
        source = production_source(path.read_text(encoding='utf8'))
        is_app = path.is_relative_to(app)
        relative = path.relative_to(app if is_app else core).as_posix()
        for start, _, key in strings(source):
            translated = re.search(r'\b(?:tr|tr_format)!\(\s*$', source[:start])
            location = f'{path.relative_to(ROOT)}:{source.count(chr(10), 0, start) + 1}'
            if translated and key not in catalog:
                errors.append(f'{location}: key absent from catalog: {key}')
            elif not translated and (re.search(r'[éèêàçîôùûœÉÀÇ]', key)
                    or (key in catalog and catalog[key][0] != catalog[key][1])):
                # French aliases in the parser and stable palette captions are
                # data, not UI prose. Both parser vocabularies remain readable.
                legacy = not is_app and relative == 'scene/graph/language.rs'
                palette_key = not is_app and relative == 'scene/palette.rs' and key in {'Aurore', 'Acide'}
                if legacy or palette_key:
                    continue
                if not is_app or key not in RAW_KEYS.get(relative, set()):
                    errors.append(f'{location}: untranslated French literal: {key}')
                elif key not in catalog:
                    errors.append(f'{location}: raw display key absent from catalog: {key}')
    if errors:
        raise SystemExit('\n'.join(errors))
    print(f'PASS: {len(catalog)} complete English/French messages; format fields match; Studio string audit clean.')


if __name__ == '__main__':
    main()
