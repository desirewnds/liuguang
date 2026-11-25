# SPDX-License-Identifier: Apache-2.0
"""Small source allowlist and native dependency selection tests; no archives/network."""
from pathlib import Path
import importlib.util
import hashlib
import json
import tempfile
import unittest
from unittest.mock import patch

PROJECT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), PROJECT/'scripts'/name)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class NativeSourceTests(unittest.TestCase):
    def setUp(self):
        base = PROJECT/'test-artifacts/native-source'
        base.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(dir=base, prefix='fixture-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        assert self.root.is_relative_to(base.resolve())
        self.source = load('package-native-source.py')
        patcher = patch.object(self.source, 'ROOT', self.root)
        patcher.start()
        self.addCleanup(patcher.stop)

    def write(self, name, content=b'fixture'):
        path = self.root/name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)

    def test_allowlist_excludes_old_ui_dependencies_and_user_material(self):
        wanted = ['Cargo.toml', 'Cargo.lock', 'crates/flow-gui/src/main.rs',
                  'packaging/notices/application-dependencies.json',
                  'packaging/upstream-notices/iced-LICENSE.txt', 'engine/catalog/options.json',
                  'engine/plugins/extractor.py', 'engine/licenses/ffmpeg.txt']
        unwanted = ['frontend/src/App.tsx', 'src-tauri/src/main.rs', '.build/secret',
                    'crates/flow-core/target/generated.rs', 'crates/flow-core/src/cookies.txt',
                    'packaging/notices/.env', 'packaging/notices/video.mp4', 'packaging/notices/personal.json',
                    'engine/licenses/application-dependencies/old.txt',
                    'engine/licenses/WebView2-LICENSE.txt', 'engine/node.exe',
                    'engine/catalog/untracked-personal.json', 'release/old.exe', 'my-video.mp4']
        for name in wanted+unwanted:
            self.write(name)
        self.write('packaging/notices/application-dependencies.json', b'{"packages": []}')
        self.write('packaging/upstream-notices/index.json', b'{"records": [{"file":"iced-LICENSE.txt"}]}')
        entries = ['catalog/options.json', 'plugins/extractor.py', 'licenses/ffmpeg.txt',
                   'licenses/application-dependencies/old.txt', 'licenses/WebView2-LICENSE.txt', 'node.exe']
        self.write('engine/manifest.json', json.dumps({'files': [{'path': name} for name in entries]}).encode())
        names = set(self.source.report(self.source.source_files())['files'])
        self.assertTrue(set(wanted) <= names)
        self.assertFalse(set(unwanted) & names)

    def test_engine_source_traversal_is_rejected(self):
        self.write('engine/manifest.json', json.dumps({'files': [{'path': 'licenses/../../private.txt'}]}).encode())
        with self.assertRaisesRegex(ValueError, 'Unsafe engine source manifest path'):
            self.source.source_files()

    def test_native_selection_never_includes_browser_package(self):
        module = load('prepare-native-engines.py')
        media = {'name': 'media', 'outputs': [{'path': 'ffmpeg.exe'}]}
        browser = {'name': 'browser', 'kind': 'cab', 'outputs': [{'path': 'webview2/msedgewebview2.exe'}]}
        self.assertEqual(module.native_packages({'packages': [media, browser]}), [media])

    def test_mixed_package_fails_instead_of_restoring_browser(self):
        module = load('prepare-native-engines.py')
        with self.assertRaisesRegex(ValueError, 'Mixed browser/media'):
            module.native_packages({'packages': [{'outputs': [{'path': 'node.exe'}, {'path': 'webview2/browser.exe'}]}]})

    def test_readonly_native_verification_never_downloads_or_restores(self):
        module = load('prepare-native-engines.py')
        with patch.object(module, 'load_checks') as checks:
            checks.return_value.matches.return_value = False
            with self.assertRaisesRegex(ValueError, 'Missing or mismatched'):
                module.prepare([{'name': 'missing', 'outputs': [{'path': 'node.exe', 'sha256': 'hash'}]}], verify_only=True)
            checks.return_value.restore.assert_not_called()

    def test_source_archive_audit_rejects_stale_workspace_lock(self):
        self.write('Cargo.lock', b'new lock')
        self.write('packaging/native-engine-manifest.json', b'{"cargo_lock_sha256":"old lock","files":[]}')
        with self.assertRaisesRegex(ValueError, 'stale relative to Cargo.lock'):
            self.source.audit_sources(self.source.source_files())

    def notice_fixture(self, native):
        module = load('collect-native-notices.py')
        content = b'archived fixture license'
        digest = hashlib.sha256(content).hexdigest()
        self.write('engine/licenses/application-dependencies/fixture.txt', content)
        package = {'name': 'fixture', 'version': '1', 'ecosystem': 'cargo',
                   'notices': [{'file': 'fixture.txt', 'sha256': digest}] if native else ['fixture.txt']}
        index = {'schema': 1, 'packages': [package]} if native else [package]
        self.write('engine/licenses/application-dependencies.json', json.dumps(index).encode())
        self.write('engine/manifest.json', json.dumps({'files': [{'path': 'licenses/application-dependencies/fixture.txt', 'sha256': digest}]}).encode())
        previous = module.previous_notices(self.root/'engine/licenses/application-dependencies.json')
        return module, previous[('fixture', '1')]['notices'][0], content

    def test_legacy_notice_index_uses_original_engine_hash(self):
        module, notice, expected = self.notice_fixture(native=False)
        actual = module.verified_previous_notice(self.root/'engine/licenses/application-dependencies', notice, self.root/'engine/manifest.json')
        self.assertEqual(actual, expected)

    def test_promoted_native_notice_index_is_accepted(self):
        module, notice, expected = self.notice_fixture(native=True)
        actual = module.verified_previous_notice(self.root/'engine/licenses/application-dependencies', notice, self.root/'no-old-manifest-required.json')
        self.assertEqual(actual, expected)

    def test_modified_notice_is_rejected_for_both_index_schemas(self):
        for native in [False, True]:
            module, notice, _ = self.notice_fixture(native)
            self.write('engine/licenses/application-dependencies/fixture.txt', b'changed')
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                module.verified_previous_notice(self.root/'engine/licenses/application-dependencies', notice, self.root/'engine/manifest.json')

    def test_missing_notice_and_missing_native_hash_are_rejected(self):
        module, notice, _ = self.notice_fixture(native=True)
        (self.root/'engine/licenses/application-dependencies/fixture.txt').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing or unsafe'):
            module.verified_previous_notice(self.root/'engine/licenses/application-dependencies', notice, self.root/'engine/manifest.json')
        self.write('engine/licenses/application-dependencies.json', json.dumps({'schema':1, 'packages':[{'name':'fixture','version':'1','ecosystem':'cargo','notices':[{'file':'fixture.txt'}]}]}).encode())
        with self.assertRaisesRegex(ValueError, 'invalid archived native notice hash'):
            module.previous_notices(self.root/'engine/licenses/application-dependencies.json')

    def test_missing_expanded_ffmpeg_inputs_preserve_existing_notice_index(self):
        module = load('collect-ffmpeg-notices.py')
        source = self.root/'engine/sources/dependencies'
        destination = self.root/'engine/licenses/ffmpeg-dependencies'
        index = self.root/'engine/licenses/ffmpeg-dependencies.json'
        self.write('engine/licenses/ffmpeg-dependencies.json', b'{"existing":"verified index"}')
        before = index.read_bytes()
        with patch.object(module, 'SOURCE', source), patch.object(module, 'DEST', destination):
            for state in ['missing-directory', 'empty-directory', 'unsupported-files']:
                with self.subTest(state=state):
                    if state != 'missing-directory':
                        source.mkdir(parents=True, exist_ok=True)
                    if state == 'unsupported-files':
                        (source/'README.txt').write_bytes(b'no source archives')
                    with self.assertRaisesRegex(SystemExit, 'No expanded FFmpeg source archives'):
                        module.main()
                    self.assertEqual(index.read_bytes(), before)
                    self.assertFalse(destination.exists())


if __name__ == '__main__':
    unittest.main()
