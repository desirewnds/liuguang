# SPDX-License-Identifier: Apache-2.0
"""Offline package boundary tests; optional NSIS compile creates a never-run fixture."""
import argparse
import contextlib
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

PROJECT = Path(__file__).resolve().parents[1]
MAKENSIS = None


def load_module():
    spec = importlib.util.spec_from_file_location('package_native', PROJECT/'scripts/package-native.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class NativePackageTests(unittest.TestCase):
    def setUp(self):
        self.module = load_module()
        scratch = PROJECT/'test-artifacts/native-package'
        scratch.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix='fixture-', dir=scratch)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        assert self.root.is_relative_to(scratch.resolve())
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        for name, value in {'ROOT': self.root, 'STAGE': self.root/'target/native-package',
                            'PAYLOAD': self.root/'target/native-package/payload',
                            'MANIFEST': self.root/'target/native-package/package-manifest.json'}.items():
            stack.enter_context(patch.object(self.module, name, value))
        self.files = []
        for name in ['yt-dlp.exe', 'ffmpeg.exe', 'ffprobe.exe', 'node.exe', 'avcodec-62.dll',
                     'downloads.lock.json', 'licenses/notice.txt', 'webview2/browser.exe',
                     'licenses/WebView2-Fixed-Runtime-LICENSE.txt',
                     'licenses/application-dependencies.json', 'licenses/application-dependencies/legacy.txt']:
            path = self.write('engine/'+name, b'MZtiny test fixture' if name.endswith(('.exe', '.dll')) else b'fixture')
            self.files.append({'path': name, 'bytes': path.stat().st_size, 'sha256': self.module.digest(path)})
        self.write('engine/manifest.json', json.dumps({'schema': 1, 'platform': 'windows-x86_64',
                   'versions': {'yt-dlp.exe': 'test', 'webview2': 'test'}, 'files': self.files}).encode())
        for name in ['flow-cli.exe', 'flow-desktop.exe']:
            self.write('target/release/'+name, b'MZfixture NEVER RUN')
        self.write('packaging/flow.cmd', b'@echo off\nexit /b 99\n')
        self.write('packaging/installer.nsi', (PROJECT/'packaging/installer.nsi').read_bytes())
        self.write('packaging/icon.ico', (PROJECT/'packaging/icon.ico').read_bytes())
        self.write('THIRD_PARTY_NOTICES.md', b'Test fixture only')
        self.write('docs/third-party-source-status.md', b'Test fixture only')
        self.write('Cargo.lock', b'native test lock')
        notice = self.write('packaging/notices/application-dependencies/native-LICENSE.txt', b'native test license')
        self.notice_index = {'schema': 1, 'platform': 'windows-x86_64-gnu',
            'cargo_lock_sha256': self.module.digest(self.root/'Cargo.lock'),
            'packages': [{'name': 'fixture', 'version': '1', 'status': 'notice-archived',
                          'notices': [{'file': notice.name, 'sha256': self.module.digest(notice)}]}]}
        self.write('packaging/notices/application-dependencies.json', json.dumps(self.notice_index).encode())
        self.module.prepare_manifest()

    def stage(self):
        return self.module.stage_bundle(verify_binary=False)

    def write(self, relative, data):
        path = self.root/relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def test_native_resources_remove_browser_but_keep_media(self):
        resources = self.module.native_resources()
        self.assertNotIn('webview2', resources['versions'])
        names = {entry['path'] for entry in resources['files']}
        self.assertNotIn('webview2/browser.exe', names)
        self.assertNotIn('licenses/WebView2-Fixed-Runtime-LICENSE.txt', names)
        self.assertIn('node.exe', names)
        self.assertIn('avcodec-62.dll', names)

    def test_stage_audit_and_clean_preserve_originals(self):
        original = (self.root/'engine/node.exe').read_bytes()
        self.stage()
        result = self.module.audit_bundle()
        self.assertTrue(result['runtime_verified'])
        self.assertFalse(result['webview2'])
        self.module.clean_stage()
        self.assertFalse(self.module.STAGE.exists())
        self.assertEqual((self.root/'engine/node.exe').read_bytes(), original)

    def test_clean_native_checkout_needs_no_browser_or_legacy_notice_files(self):
        for row in self.files:
            if self.module.is_browser(row['path']) or self.module.is_old_application_notice(row['path']):
                (self.root/'engine'/row['path']).unlink()
        self.module.prepare_manifest()
        self.stage()
        self.assertTrue(self.module.audit_bundle()['runtime_verified'])

    def test_tampered_canonical_dependency_fails_before_staging(self):
        (self.root/'engine/node.exe').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'Pinned resource mismatch'):
            self.stage()
        self.assertFalse(self.module.STAGE.exists())

    def test_unknown_stage_file_is_not_deleted(self):
        self.stage()
        unknown = self.write('target/native-package/personal.txt', b'keep')
        with self.assertRaisesRegex(ValueError, 'Untracked package stage file'):
            self.module.clean_stage()
        self.assertEqual(unknown.read_bytes(), b'keep')

    def test_modified_installer_script_is_rejected_and_preserved(self):
        self.stage()
        script = self.write('target/native-package/installer.nsi', b'changed script')
        with self.assertRaisesRegex(ValueError, 'script integrity mismatch'):
            self.module.audit_bundle()
        with self.assertRaisesRegex(ValueError, 'Modified generated stage file retained'):
            self.module.clean_stage()
        self.assertEqual(script.read_bytes(), b'changed script')

    def test_audit_rejects_unlisted_payload(self):
        self.stage()
        self.write('target/native-package/payload/personal.txt', b'keep')
        with self.assertRaisesRegex(ValueError, 'Unlisted or missing'):
            self.module.audit_bundle()

    def test_path_and_nsis_injection_rejected(self):
        for name in ['../outside', 'C:/outside', '/outside', 'engine/../../outside']:
            with self.assertRaises(ValueError):
                self.module.safe_child(self.root, name)
        with self.assertRaises(ValueError):
            self.module.nsis('file\nDelete everything')
        self.assertEqual(self.module.nsis('$TEMP/"name"'), '$$TEMP/$\\"name$\\"')

    def test_upgrade_and_cleanup_contract(self):
        self.stage()
        installer = (self.module.STAGE/'installer.nsi').read_text(encoding='utf-8')
        self.assertNotIn('RMDir /r', installer)
        self.assertNotIn('nsis_tauri_utils', installer)
        self.assertNotIn('EdgeUpdate', installer)
        self.assertIn('/S /UPDATE _?=$OldRoot', installer)
        self.assertLess(installer.index('--flow-check-install-root "$OldRoot"'), installer.index('ExecWait'))
        uninstall = installer[installer.index('Section "Uninstall"'):]
        self.assertLess(uninstall.index('--flow-uninstall-cleanup'), uninstall.index('DeletePayloadResources'))
        self.assertIn('${If} $UpdateMode != 1', uninstall)
        flags = installer[installer.index('Function un.onInit'):installer.index('Function un.DataPage')]
        self.assertLess(flags.index('${GetOptions} $CMDLINE "/DELETEAPPDATA"'), flags.index('${GetOptions} $CMDLINE "/KEEPDATA"'))

    def test_real_nsis_compile_never_runs_installer(self):
        if not MAKENSIS:
            self.skipTest('explicit --makensis not supplied')
        self.stage()
        # Only NSIS syntax is tested here. Fixture payloads are deliberately not
        # executable PE files; the real compile command always verifies imports.
        result = self.module.compile_installer(Path(MAKENSIS), verify_imports=False)
        output = self.root/'release'/result['installer']['file']
        self.assertEqual(output.read_bytes()[:2], b'MZ')
        self.assertEqual(self.module.digest(output), result['installer']['sha256'])

    def test_stale_lock_or_missing_notice_blocks_release(self):
        self.write('Cargo.lock', b'changed lock')
        with self.assertRaisesRegex(ValueError, 'do not match Cargo.lock'):
            self.stage()

    def test_development_manifest_cannot_bypass_missing_notice(self):
        self.notice_index['packages'].append({'name': 'missing', 'version': '1', 'status': 'needs-review', 'notices': []})
        self.write('packaging/notices/application-dependencies.json', json.dumps(self.notice_index).encode())
        self.module.prepare_manifest(allow_incomplete_notices=True)
        with self.assertRaisesRegex(ValueError, 'requires review'):
            self.stage()

    def test_changed_prepared_manifest_blocks_release(self):
        manifest = self.root/'packaging/native-engine-manifest.json'
        value = json.loads(manifest.read_text())
        value['versions']['unexpected'] = 'tampered'
        self.write('packaging/native-engine-manifest.json', json.dumps(value).encode())
        with self.assertRaisesRegex(ValueError, 'Prepare the native resource manifest'):
            self.stage()

    def test_compiled_helper_manifest_mismatch_blocks_release(self):
        with patch.object(self.module.subprocess, 'run') as run:
            run.return_value.stdout = '{}'
            with self.assertRaisesRegex(ValueError, 'compiled with different resources'):
                self.module.stage_bundle()
        self.assertFalse(self.module.STAGE.exists())


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--makensis')
    args, unittest_args = parser.parse_known_args()
    MAKENSIS = args.makensis
    unittest.main(argv=[sys.argv[0], *unittest_args])
