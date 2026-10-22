"""Regression checks for release integrity and the remote publication boundary."""
import contextlib
import hashlib
import io
import importlib.util
import json
import struct
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


package = load('windows-release')
publisher = load('publish-release')
REVISION = 'a' * 40


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.archive = self.root / 'SyFrack-v0.2.0-windows-x64.zip'
        with zipfile.ZipFile(self.archive, 'w') as bundle:
            bundle.writestr(self.archive.stem + '/BUILD_INFO.json', json.dumps({
                'tag': 'v0.2.0', 'revision': REVISION, 'target': package.TARGET,
                'profile': 'release', 'c_runtime': 'static',
            }))
        self.write_checksum()
        (self.root / 'RELEASE_NOTES.md').write_text('Notes', encoding='utf8')

    def write_checksum(self):
        digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        (self.root / 'SHA256SUMS.txt').write_text(f'{digest}  {self.archive.name}\n', encoding='utf8')

    def publish(self):
        with contextlib.redirect_stdout(io.StringIO()):
            publisher.publish('v0.2.0', REVISION, self.root, 'owner/SyFrack')

    def test_version_tag_must_match_the_workspace(self):
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.2.0"\n')
        self.assertEqual(package.validate_tag('v0.2.0', self.root), '0.2.0')
        for tag in ['v0.3.0', '../v0.2.0', 'v0.2.0\nrevision=x', '--help']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                package.validate_tag(tag, self.root)

    def test_executable_requires_x64_correct_subsystem_and_static_runtime(self):
        data = bytearray(0x500)
        data[:2] = b'MZ'
        struct.pack_into('<I', data, 0x3c, 0x80)
        data[0x80:0x84] = b'PE\0\0'
        struct.pack_into('<HH', data, 0x84, 0x8664, 1)
        struct.pack_into('<H', data, 0x94, 0xf0)
        struct.pack_into('<H', data, 0x98, 0x20b)
        struct.pack_into('<H', data, 0x98 + 68, 3)
        self.assertEqual(package.inspect_exe(bytes(data), 3), [])
        with self.assertRaises(ValueError):
            package.inspect_exe(bytes(data), 2)
        struct.pack_into('<I', data, 0x98 + 120, 0x1000)
        struct.pack_into('<IIII', data, 0x98 + 0xf0 + 8, 0x100, 0x1000, 0x100, 0x300)
        struct.pack_into('<I', data, 0x300 + 12, 0x1040)
        data[0x340:0x34f] = b'vcruntime140.dll'
        with self.assertRaisesRegex(ValueError, 'External MSVC'):
            package.inspect_exe(bytes(data), 3)

    def test_corrupt_archive_never_calls_github(self):
        self.archive.write_bytes(self.archive.read_bytes() + b'changed')
        with patch.object(publisher, 'gh') as cli, self.assertRaisesRegex(ValueError, 'checksum'):
            self.publish()
        cli.assert_not_called()

    def test_moved_tag_prevents_publication(self):
        with patch.object(publisher, 'remote_revision', return_value='b' * 40), \
                patch.object(publisher, 'gh') as cli, self.assertRaisesRegex(ValueError, 'tag moved'):
            self.publish()
        cli.assert_not_called()

    def test_published_release_is_preserved(self):
        with patch.object(publisher, 'remote_revision', return_value=REVISION), \
                patch.object(publisher, 'gh', return_value='{"draft":false}') as cli, \
                self.assertRaisesRegex(ValueError, 'already published'):
            self.publish()
        self.assertEqual(cli.call_count, 1)

    def test_create_upload_then_publish_in_that_order(self):
        missing = subprocess.CalledProcessError(1, 'gh', stderr='gh: Not Found (HTTP 404)')
        with patch.object(publisher, 'remote_revision', return_value=REVISION), \
                patch.object(publisher, 'gh', side_effect=[missing, '', '', '']) as cli:
            self.publish()
        calls = [call.args for call in cli.call_args_list]
        self.assertEqual([args[:2] for args in calls[1:]],
                         [('release', 'create'), ('release', 'upload'), ('release', 'edit')])
        self.assertIn('--draft', calls[1])
        self.assertIn('--verify-tag', calls[1])
        self.assertIn('--draft=false', calls[3])

    def test_existing_draft_can_retry_without_recreation(self):
        with patch.object(publisher, 'remote_revision', return_value=REVISION), \
                patch.object(publisher, 'gh', side_effect=['{"draft":true}', '', '']) as cli:
            self.publish()
        self.assertEqual([call.args[:2] for call in cli.call_args_list[1:]],
                         [('release', 'upload'), ('release', 'edit')])

    def test_permission_errors_do_not_create_a_release(self):
        denied = subprocess.CalledProcessError(1, 'gh', stderr='gh: Forbidden (HTTP 403)')
        with patch.object(publisher, 'remote_revision', return_value=REVISION), \
                patch.object(publisher, 'gh', side_effect=denied) as cli, \
                self.assertRaises(subprocess.CalledProcessError):
            self.publish()
        self.assertEqual(cli.call_count, 1)

    def test_upload_failure_leaves_the_release_unpublished(self):
        failed = subprocess.CalledProcessError(1, 'gh', stderr='Upload failed')
        with patch.object(publisher, 'remote_revision', return_value=REVISION), \
                patch.object(publisher, 'gh', side_effect=['{"draft":true}', failed]) as cli, \
                self.assertRaises(subprocess.CalledProcessError):
            self.publish()
        self.assertFalse(any(call.args[:2] == ('release', 'edit') for call in cli.call_args_list))

    def test_annotated_tags_are_resolved_to_commits(self):
        with patch.object(publisher, 'gh', side_effect=[
                '{"object":{"type":"tag","sha":"tag-object"}}',
                json.dumps({'object': {'type': 'commit', 'sha': REVISION}})]):
            self.assertEqual(publisher.remote_revision('owner/SyFrack', 'v0.2.0'), REVISION)


if __name__ == '__main__':
    unittest.main()
