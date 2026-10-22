"""Source export synchronization and documentation-closure regressions."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from source_layout import documentation_files

spec = importlib.util.spec_from_file_location('prepare_source', Path(__file__).with_name('prepare-source.py'))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class SourceTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.destination = self.root / 'to_github'
        self.source = self.root / 'source.txt'
        self.source.write_bytes(b'first\r\n')
        self.selected = {'source.txt': self.source}

    def prepare(self):
        with patch.object(exporter, 'ROOT', self.root), patch.object(exporter, 'DEST', self.destination), \
             patch.object(exporter, 'inputs', return_value=self.selected), contextlib.redirect_stdout(io.StringIO()):
            exporter.main()

    def test_normalizes_lines_and_records_exact_bytes(self):
        self.prepare()
        target = self.destination / 'source.txt'
        self.assertEqual(target.read_bytes(), b'first\n')
        manifest = json.loads((self.destination / exporter.MANIFEST).read_text())
        self.assertEqual(manifest['files'][0]['sha256'], exporter.sha(target))
        self.assertEqual(manifest['total_bytes'], len(target.read_bytes()))

    def test_modified_export_aborts_before_copying_any_file(self):
        self.prepare()
        target = self.destination / 'source.txt'
        target.write_text('owner edit')
        extra = self.root / 'new.txt'
        extra.write_text('new')
        self.selected['new.txt'] = extra
        with self.assertRaisesRegex(RuntimeError, 'independently changed'):
            self.prepare()
        self.assertEqual(target.read_text(), 'owner edit')
        self.assertFalse((self.destination / 'new.txt').exists())

    def test_removes_only_unchanged_previously_managed_files(self):
        self.prepare()
        self.selected.clear()
        self.prepare()
        self.assertFalse((self.destination / 'source.txt').exists())
        manifest = json.loads((self.destination / exporter.MANIFEST).read_text())
        self.assertEqual(manifest['file_count'], 0)

    def test_modified_obsolete_file_is_preserved(self):
        self.prepare()
        target = self.destination / 'source.txt'
        target.write_text('keep this edit')
        self.selected.clear()
        with self.assertRaisesRegex(RuntimeError, 'unmanaged files'):
            self.prepare()
        self.assertEqual(target.read_text(), 'keep this edit')

    def test_unmanaged_file_is_not_deleted(self):
        self.prepare()
        extra = self.destination / 'owner.txt'
        extra.write_text('keep')
        with self.assertRaisesRegex(RuntimeError, 'unmanaged files'):
            self.prepare()
        self.assertEqual(extra.read_text(), 'keep')

    def test_previous_manifest_cannot_name_a_parent_path(self):
        self.prepare()
        manifest_path = self.destination / exporter.MANIFEST
        manifest = json.loads(manifest_path.read_text())
        manifest['files'].append({'path': '../source.txt', 'sha256': exporter.sha(self.source)})
        manifest_path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(RuntimeError, 'Unsafe previous'):
            self.prepare()
        self.assertEqual(self.source.read_bytes(), b'first\r\n')

    def test_documentation_closure_follows_nested_guides_and_images(self):
        (self.root / 'docs/images').mkdir(parents=True)
        (self.root / 'README.md').write_text('[guide](docs/guide.md#start) [site](https://example.org)')
        (self.root / 'docs/guide.md').write_text('![capture](images/studio.png) [home](../README.md)')
        (self.root / 'docs/images/studio.png').write_bytes(b'image')
        found = documentation_files(self.root, ['README.md'])
        self.assertEqual(set(found), {'README.md', 'docs/guide.md', 'docs/images/studio.png'})

    def test_documentation_closure_rejects_missing_dependencies(self):
        (self.root / 'README.md').write_text('[missing](docs/missing.md)')
        with self.assertRaisesRegex(ValueError, 'Missing documentation dependency'):
            documentation_files(self.root, ['README.md'])

    def test_documentation_closure_rejects_paths_outside_the_source(self):
        (self.root / 'README.md').write_text('[outside](../outside.md)')
        with self.assertRaisesRegex(ValueError, 'escapes the source tree'):
            documentation_files(self.root, ['README.md'])


if __name__ == '__main__':
    unittest.main()
