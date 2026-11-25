# SPDX-License-Identifier: Apache-2.0
"""Small archive, path and manifest fixtures. No Mac tool, network or media run."""
from pathlib import Path
import importlib.util
import io
import json
import os
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import macos_common as common
spec = importlib.util.spec_from_file_location('flow_mac_notice_tests', ROOT / 'scripts/collect-macos-notices.py')
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


class PackageChecks(unittest.TestCase):
    def setUp(self):
        self.parent = ROOT / 'test-artifacts/macos-package'
        self.parent.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix='case-', dir=self.parent)
        self.base = Path(self.temporary.name).resolve()

    def tearDown(self):
        self.temporary.cleanup()
        try:
            self.parent.rmdir()
        except OSError:
            pass

    def test_fixed_locks_cover_both_architectures(self):
        for target in common.TARGETS:
            rows = common.packages(target)
            self.assertEqual({r['name'] for r in rows}, {'yt-dlp', 'node', 'ffmpeg', 'ffprobe'})
            self.assertTrue(all('/latest/' not in r['url'] for r in rows))

    def test_notice_collection_uses_only_resolved_target_ids_and_unlicense(self):
        package = self.base / '.build/cargo/registry/src/example'
        package.mkdir(parents=True)
        (package / 'Cargo.toml').write_text('[package]\nname="example"\n', encoding='utf-8')
        (package / 'UNLICENSE').write_text('Exact package license text', encoding='utf-8')
        common.write_json(self.base / 'packaging/notices/application-dependencies.json', {'packages': []})
        common.write_json(self.base / 'packaging/upstream-notices/index.json', {'records': []})
        (self.base / 'Cargo.lock').write_text('lock fixture', encoding='utf-8')
        selected = {'id': 'selected', 'name': 'example', 'version': '1.0', 'source': 'registry',
                    'manifest_path': str(package / 'Cargo.toml'), 'license': 'Unlicense'}
        unrelated = {**selected, 'id': 'other-platform', 'name': 'excluded',
                     'manifest_path': str(self.base / 'does-not-exist/Cargo.toml')}
        metadata = {'packages': [selected, unrelated], 'resolve': {'nodes': [{'id': 'selected'}]}}
        output = self.base / '.build/macos/notice-review'
        with patch.object(notices, 'ROOT', self.base), patch.object(notices, 'run', return_value=json.dumps(metadata)):
            self.assertEqual(notices.collect('aarch64-apple-darwin', output), 1)
        index = json.loads((output / 'licenses/application-dependencies.json').read_text(encoding='utf-8'))
        self.assertEqual([p['name'] for p in index['packages']], ['example'])
        self.assertEqual(index['packages'][0]['status'], 'notice-archived')

    def test_notice_review_does_not_create_or_enter_engine_stage(self):
        with patch.object(common, 'ROOT', self.base), patch.object(notices, 'ROOT', self.base):
            output = notices.review_directory('aarch64-apple-darwin')
            self.assertEqual(output.name, 'notice-review')
            self.assertFalse((common.area('aarch64-apple-darwin') / 'engine').exists())
            for target in common.TARGETS:
                with self.assertRaises(ValueError):
                    notices.review_directory('aarch64-apple-darwin', common.area(target) / 'engine/review')

    def test_paths_reject_traversal_absolute_windows_and_nul(self):
        for name in ('../video.mp4', '/etc/profile', 'C:/video', 'x\\y', 'a/../../b', '\0bad'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                common.child(self.base, name)

    def test_zip_accepts_unique_regular_selected_binary(self):
        archive = self.base / 'test.zip'
        with zipfile.ZipFile(archive, 'w') as stream:
            stream.writestr('tools/ffmpeg', b'pinned-binary')
            stream.writestr('../ignored-unselected', b'never-extracted')
        data = common.read_output(archive, {'kind': 'zip'}, {'member_basename': 'ffmpeg'})
        self.assertEqual(data, b'pinned-binary')
        self.assertFalse((self.base.parent / 'ignored-unselected').exists())

    def test_zip_rejects_duplicate_names(self):
        archive = self.base / 'test.zip'
        with zipfile.ZipFile(archive, 'w') as stream:
            stream.writestr('first/ffmpeg', b'a')
            stream.writestr('second/ffmpeg', b'b')
        with self.assertRaises(ValueError):
            common.read_output(archive, {'kind': 'zip'}, {'member_basename': 'ffmpeg'})

    def test_zip_rejects_selected_link(self):
        archive = self.base / 'test.zip'
        info = zipfile.ZipInfo('ffmpeg')
        info.create_system = 3
        info.external_attr = 0o120777 << 16
        with zipfile.ZipFile(archive, 'w') as stream:
            stream.writestr(info, '../outside')
        with self.assertRaises(ValueError):
            common.read_output(archive, {'kind': 'zip'}, {'member_basename': 'ffmpeg'})

    def test_tar_rejects_selected_link(self):
        archive = self.base / 'test.tar'
        with tarfile.open(archive, 'w') as stream:
            info = tarfile.TarInfo('bin/node')
            info.type = tarfile.SYMTYPE
            info.linkname = '/usr/local/bin/node'
            stream.addfile(info)
        with self.assertRaises(ValueError):
            common.read_output(archive, {'kind': 'tar'}, {'member': 'bin/node'})

    def test_offline_download_requires_exact_hash(self):
        (self.base / 'node.tar').write_bytes(b'wrong')
        with self.assertRaises(ValueError):
            common.acquire({'file': 'node.tar', 'sha256': '0' * 64}, self.base)

    def test_deployment_commands_parse_both_macho_versions(self):
        text = 'cmd LC_BUILD_VERSION\n cmdsize 32\n platform 1\n minos 13.5\n sdk 15.0\n'
        self.assertEqual(common.minimum_versions(text), ['13.5'])
        text = 'cmd LC_VERSION_MIN_MACOSX\n cmdsize 16\n version 11.0\n sdk 12.0\n'
        self.assertEqual(common.minimum_versions(text), ['11.0'])
        with self.assertRaises(ValueError):
            common.minimum_versions('cmd LC_LOAD_DYLIB')

    def test_macho_rejects_wrong_architecture(self):
        path = self.base / 'ffmpeg'
        path.write_bytes(b'fixture')
        with patch.object(common, 'run', return_value='x86_64\n'), self.assertRaises(ValueError):
            common.inspect_macho(path, 'aarch64-apple-darwin', bundle_root=self.base,
                                 executable_dir=self.base, minimum='13.5')

    def test_macho_rejects_newer_system_requirement(self):
        output = ['arm64', 'cmd LC_BUILD_VERSION\n minos 15.0\n']
        with patch.object(common, 'run', side_effect=output), self.assertRaises(ValueError):
            common.inspect_macho(self.base / 'node', 'aarch64-apple-darwin', bundle_root=self.base,
                                 executable_dir=self.base, minimum='13.5')

    def test_macho_rejects_external_homebrew_dependency(self):
        output = ['arm64', 'cmd LC_BUILD_VERSION\n minos 13.5\n',
                  'binary:\n\t/opt/homebrew/lib/libfoo.dylib (compatibility version 1.0.0, current version 1.0.0)\n']
        with patch.object(common, 'run', side_effect=output), self.assertRaises(ValueError):
            common.inspect_macho(self.base / 'node', 'aarch64-apple-darwin', bundle_root=self.base,
                                 executable_dir=self.base, minimum='13.5')

    def test_engine_manifest_is_exact_and_rejects_unknown_files(self):
        engine = self.base / 'engine'
        engine.mkdir()
        (engine / 'node').write_bytes(b'node')
        manifest = self.base / 'engine-manifest.json'
        common.write_json(manifest, {'files': common.file_rows(engine)})
        (engine / 'manifest.json').write_bytes(manifest.read_bytes())
        common.verify_engine(engine, manifest)
        (engine / 'foreign-video.mp4').write_bytes(b'keep')
        with self.assertRaises(ValueError):
            common.verify_engine(engine, manifest)
        self.assertTrue((engine / 'foreign-video.mp4').is_file())

    def test_owned_cleanup_refuses_changed_files(self):
        with patch.object(common, 'ROOT', self.base):
            stage = self.base / '.build/macos/target/engine'
            stage.mkdir(parents=True)
            path = stage / 'node'
            path.write_bytes(b'original')
            manifest = stage.parent / 'owner.json'
            common.write_json(manifest, {'owner': common.OWNER, 'files': common.file_rows(stage)})
            path.write_bytes(b'changed')
            with self.assertRaises(ValueError):
                common.clean_owned(stage, manifest)
            self.assertEqual(path.read_bytes(), b'changed')

    def test_owned_cleanup_removes_only_exact_snapshot(self):
        with patch.object(common, 'ROOT', self.base):
            stage = self.base / '.build/macos/target/engine'
            stage.mkdir(parents=True)
            (stage / 'node').write_bytes(b'original')
            manifest = stage.parent / 'owner.json'
            common.write_json(manifest, {'owner': common.OWNER, 'files': common.file_rows(stage)})
            common.clean_owned(stage, manifest)
            self.assertFalse(stage.exists())
            self.assertFalse(manifest.exists())


if __name__ == '__main__':
    unittest.main(verbosity=2)
