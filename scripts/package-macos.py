# SPDX-License-Identifier: Apache-2.0
"""Assemble and audit a private .app, then create a DMG/ZIP without launching it.

Requires already prepared/signed runtime inputs and matching native Rust outputs.
Does not install, mount the DMG, publish a release or modify Windows artifacts.
"""
from pathlib import Path
import argparse
import json
import os
import plistlib
import shutil
import tempfile
from macos_common import (ROOT, TARGETS, OWNER, VERSION, area, audit_macho_tree, child,
                          clean_owned, file_rows, load_lock, require_mac, run, sha,
                          sign, verify_engine, write_json)


def icon(base: Path, destination: Path):
    with tempfile.TemporaryDirectory(prefix='flow-icon-', dir=base) as temporary:
        folder = Path(temporary) / 'flow.iconset'
        folder.mkdir()
        original = ROOT / 'crates/flow-platform/assets/icon.png'
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                name = f'icon_{size}x{size}' + ('@2x' if scale == 2 else '') + '.png'
                run(['/usr/bin/sips', '-z', str(size * scale), str(size * scale), original, '--out', folder / name])
        run(['/usr/bin/iconutil', '--convert', 'icns', '--output', destination, folder])


def package(target: str, *, identity='-', notarize_profile=None, archive='dmg'):
    require_mac(target)
    if notarize_profile and identity == '-':
        raise ValueError('Notarization requires an explicitly supplied Developer ID identity')
    base = area(target)
    manifest_path = base / 'engine-manifest.json'
    manifest_bytes = manifest_path.read_bytes()
    manifest = json.loads(manifest_bytes)
    if manifest['target'] != target or manifest['cargo_lock_sha256'] != sha(ROOT / 'Cargo.lock'):
        raise ValueError('The prepared runtime target or Cargo lock is stale')
    runtime = base / 'engine'
    verify_engine(runtime, manifest_path)
    runtime_provenance = json.loads((runtime / 'licenses/macos-runtime-provenance.json').read_text(encoding='utf-8'))
    if runtime_provenance['downloads_lock_sha256'] != sha(ROOT / 'packaging/macos/downloads.lock.json'):
        raise ValueError('Prepared runtimes belong to an older download lock; prepare and rebuild')
    expected_signing = 'ad-hoc' if identity == '-' else 'Developer ID'
    if runtime_provenance['signing'] != expected_signing:
        raise ValueError('Prepare engines with the same signing mode before compiling')
    provenance = json.loads((base / 'engine-verification.json').read_text(encoding='utf-8'))
    executables = {name: ROOT / 'target' / target / 'release' / name for name in ('flow-desktop', 'flow-cli')}
    for name, path in executables.items():
        if not path.is_file():
            raise ValueError('Missing native Rust output: ' + name)
    if manifest_bytes not in executables['flow-cli'].read_bytes():
        raise ValueError('CLI was not compiled with the exact prepared runtime manifest')
    stage, ownership = base / 'bundle', base / 'bundle-owner.json'
    if stage.exists():
        if ownership.exists():
            clean_owned(stage, ownership)
        else:
            raise ValueError('Unowned bundle stage retained: ' + str(stage))
    stage.mkdir()
    app = stage / '流光.app'
    contents = app / 'Contents'
    macos, resources = contents / 'MacOS', contents / 'Resources'
    macos.mkdir(parents=True)
    resources.mkdir()
    try:
        for name, source in executables.items():
            shutil.copyfile(source, macos / name)
            (macos / name).chmod(0o755)
        shutil.copytree(runtime, resources / 'engine', symlinks=False)
        # Keep the source runtime hashes exact; never codesign --deep here.
        verify_engine(resources / 'engine', manifest_path)
        (resources / 'engine-manifest.json').write_bytes(manifest_bytes)
        template = plistlib.loads((ROOT / 'packaging/macos/Info.plist').read_bytes())
        template['CFBundleShortVersionString'] = VERSION
        template['CFBundleVersion'] = VERSION
        template['LSMinimumSystemVersion'] = load_lock()['minimum_macos']
        (contents / 'Info.plist').write_bytes(plistlib.dumps(template, sort_keys=True))
        icon(base, resources / 'flow.icns')
        binaries = [macos / name for name in executables] + [resources / 'engine' / name for name in ('yt-dlp', 'node', 'ffmpeg', 'ffprobe')]
        imports = audit_macho_tree(binaries, target, app, template['LSMinimumSystemVersion'])
        for path in (macos / 'flow-cli', macos / 'flow-desktop'):
            sign(path, identity)
        # Nested private runtimes have already been individually signed and
        # hashed. Signing the outer bundle seals, but must not rewrite, them.
        sign(app, identity)
        run(['/usr/bin/codesign', '--verify', '--deep', '--strict', app])
        verify_engine(resources / 'engine', manifest_path)
        release = child(ROOT, 'release')
        release.mkdir(exist_ok=True)
        stem = f'liuguang-{VERSION}-macos-' + ('arm64' if target.startswith('aarch64') else 'x64')
        destination = release / (stem + ('.dmg' if archive == 'dmg' else '.app.zip'))
        report_path = release / (stem + '-manifest.json')
        sums_path = release / (stem + '-SHA256SUMS.txt')
        if any(path.exists() for path in (destination, report_path, sums_path)):
            raise ValueError('Existing release is retained; use a new version or review it explicitly before replacing')
        with tempfile.TemporaryDirectory(prefix='flow-package-', dir=base) as temporary:
            candidate = Path(temporary) / destination.name
            if archive == 'dmg':
                run(['/usr/bin/hdiutil', 'create', '-quiet', '-volname', '流光', '-srcfolder', stage,
                     '-format', 'UDZO', candidate])
                if identity != '-':
                    sign(candidate, identity)
            else:
                run(['/usr/bin/ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', app, candidate])
            notarized = False
            if notarize_profile:
                # This is never enabled implicitly. The profile refers to an
                # existing keychain entry; credentials never enter arguments.
                run(['/usr/bin/xcrun', 'notarytool', 'submit', candidate,
                     '--keychain-profile', notarize_profile, '--wait'])
                if archive == 'dmg':
                    run(['/usr/bin/xcrun', 'stapler', 'staple', candidate])
                    run(['/usr/bin/xcrun', 'stapler', 'validate', candidate])
                else:
                    run(['/usr/bin/xcrun', 'stapler', 'staple', app])
                    run(['/usr/bin/xcrun', 'stapler', 'validate', app])
                    candidate.unlink()
                    run(['/usr/bin/ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', app, candidate])
                notarized = True
            if archive == 'dmg':
                run(['/usr/bin/hdiutil', 'verify', '-quiet', candidate])
            # Both paths are on the workspace volume. Hard-link publication is
            # exclusive, so a newly appeared release is never overwritten.
            os.link(candidate, destination)
        payload = file_rows(app)
        record = {
            'schema': 1, 'version': VERSION, 'target': target,
            'minimum_macos': template['LSMinimumSystemVersion'],
            'installer': destination.name, 'bytes': destination.stat().st_size, 'sha256': sha(destination),
            'payload_bytes': sum(row['bytes'] for row in payload), 'payload_files': payload,
            'runtime_manifest_sha256': sha(manifest_path), 'cargo_lock_sha256': sha(ROOT / 'Cargo.lock'),
            'signing': expected_signing, 'notarized': notarized, 'macho': imports,
            'capability_checks': provenance['capabilities']['required'],
            'runtime_versions': manifest['versions'],
            'webview': False, 'global_runtime_required': False,
            'ui_verified': False, 'installation_verified': False,
            'complete_corresponding_source_verified': False, 'complete_licensing_review': False,
        }
        write_json(report_path, record)
        sums_path.write_text(sha(destination) + '  ' + destination.name + '\n', encoding='utf-8')
        return {'file': str(destination), 'bytes': record['bytes'], 'sha256': record['sha256'],
                'target': target, 'signing': expected_signing, 'notarized': notarized,
                'ui_verified': False}
    finally:
        write_json(ownership, {'owner': OWNER, 'files': file_rows(stage)})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True, choices=TARGETS)
    parser.add_argument('--sign-identity', default='-')
    parser.add_argument('--notarize-profile')
    parser.add_argument('--archive', choices=['dmg', 'zip'], default='dmg')
    parser.add_argument('--clean-stage', action='store_true')
    args = parser.parse_args()
    if args.clean_stage:
        clean_owned(area(args.target) / 'bundle', area(args.target) / 'bundle-owner.json')
    else:
        print(json.dumps(package(args.target, identity=args.sign_identity,
                                 notarize_profile=args.notarize_profile, archive=args.archive), ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
