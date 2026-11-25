# SPDX-License-Identifier: Apache-2.0
"""Allowlisted native source distribution; no binaries, browser or private data.

--list only reports files and bytes. Archive creation is an explicit separate
action and never replaces an existing release or third-party materials archive.
"""
from pathlib import Path, PurePosixPath
import argparse
import hashlib
import json
import os
import stat
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = '2.0.0'
CRATES = ['flow-core', 'flow-app', 'flow-platform', 'flow-cli', 'flow-gui']
ROOT_FILES = ['Cargo.toml', 'Cargo.lock', 'README.md', 'CONTRIBUTING.md', 'CONTRACT.md',
              'THIRD_PARTY_NOTICES.md', 'LICENSE', 'NOTICE', '.gitignore', '.gitattributes']
SCRIPTS = ['build-native.ps1', 'package-native.py', 'package-native-source.py',
           'prepare-native-engines.py', 'prepare-engines.py', 'collect-native-notices.py',
           'verify-ffmpeg-shared.py', 'catalog-audit.py', 'catalog-enrich.py',
           'audit-source-materials.py', 'collect-cargo-sources.py', 'collect-ffmpeg-sources.py',
           'collect-ffmpeg-notices.py', 'collect-nested-sources.py', 'collect-svn-sources.py',
           'macos_common.py', 'prepare-macos-engines.py', 'collect-macos-notices.py',
           'package-macos.py', 'build-macos.py']
DOCS = ['architecture.md', 'user-guide.md', 'repository-layout.md', 'release-checklist.md', 'validation.md', 'verification-1.0.2.md', 'verification-2.0.0.md',
        'license-options.md', 'third-party-source-status.md', 'native-rewrite-status.md',
        'native-packaging-verification.md', 'capability-matrix.csv', 'catalog-audit.json',
        'macos.md', 'verification-1.0.1.md']
PACKAGING = ['README.md', 'installer.nsi', 'flow.cmd', 'icon.ico', 'native-engine-manifest.json',
             'test_native_package.py', 'test_native_source.py']
EXCLUDED = {'.git', '.build', 'node_modules', 'target', 'dist', 'test-artifacts', 'release',
            'frontend', 'src-tauri', '__pycache__', '.codex', '.agents', 'sources'}
FORBIDDEN_SUFFIXES = {'.exe', '.dll', '.pdb', '.zip', '.7z', '.cab', '.pyc', '.log', '.partial',
                      '.tmp', '.bak', '.sqlite', '.sqlite3', '.db', '.mp4', '.mkv', '.webm',
                      '.dmg', '.pkg', '.dylib', '.app', '.dsym'}


def safe(path):
    current = path.absolute()
    while True:
        if current.exists() or current.is_symlink():
            metadata = current.lstat()
            if current.is_symlink() or getattr(metadata, 'st_file_attributes', 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
                raise ValueError('Linked source path is excluded: '+str(current))
        if current.parent == current:
            break
        current = current.parent
    if not path.resolve().is_relative_to(ROOT.resolve()):
        raise ValueError('Source path escaped project')


def permitted(path):
    relative = path.relative_to(ROOT)
    name = path.name.lower()
    return (not any(part.lower() in EXCLUDED for part in relative.parts)
            and path.suffix.lower() not in FORBIDDEN_SUFFIXES
            and not (name == '.env' or name.startswith('.env.') or 'cookies' in name
                     or name.endswith(('.pem', '.key', '.pfx', '.p12'))))


def source_files():
    files = set()
    def add(path):
        safe(path)
        if path.is_file() and permitted(path):
            files.add(path)
    def tree(directory, suffixes=None):
        path = ROOT/directory
        safe(path)
        if not path.exists():
            return
        for current, directories, names in os.walk(path, followlinks=False):
            base = Path(current)
            for name in list(directories):
                safe(base/name)
                if name.lower() in EXCLUDED:
                    directories.remove(name)
            for name in names:
                child = base/name
                if suffixes is None or child.suffix.lower() in suffixes:
                    add(child)
    for name in ROOT_FILES:
        add(ROOT/name)
    for crate in CRATES:
        add(ROOT/'crates'/crate/'Cargo.toml')
        tree(f'crates/{crate}/src', {'.rs'})
        tree(f'crates/{crate}/assets', {'.json', '.png', '.ico'})
        tree(f'crates/{crate}/resources', {'.rc', '.manifest'})
        tree(f'crates/{crate}/examples', {'.rs', '.py'})
        build = ROOT/'crates'/crate/'build.rs'
        if build.exists():
            add(build)
    for name in SCRIPTS:
        add(ROOT/'scripts'/name)
    for name in DOCS:
        add(ROOT/'docs'/name)
    for name in PACKAGING:
        add(ROOT/'packaging'/name)
    add(ROOT/'.github/workflows/release-check.yml')
    add(ROOT/'.github/workflows/check.yml')
    add(ROOT/'.github/workflows/macos-check.yml')
    tree('.github/ISSUE_TEMPLATE', {'.yml', '.yaml', '.md'})
    tree('packaging/macos', {'.json', '.plist', '.py', '.md'})
    notice_index = ROOT/'packaging/notices/application-dependencies.json'
    if notice_index.is_file():
        add(notice_index)
        data = json.loads(notice_index.read_text(encoding='utf-8'))
        for package in data['packages']:
            for notice in package.get('notices', []):
                if '/' in notice['file'] or '\\' in notice['file']:
                    raise ValueError('Unsafe native notice source filename')
                path = notice_index.parent/'application-dependencies'/notice['file']
                if not path.is_file():
                    raise ValueError('Missing indexed native notice: '+notice['file'])
                add(path)
    upstream_index = ROOT/'packaging/upstream-notices/index.json'
    if upstream_index.is_file():
        add(upstream_index)
        for notice in json.loads(upstream_index.read_text(encoding='utf-8'))['records']:
            if '/' in notice['file'] or '\\' in notice['file']:
                raise ValueError('Unsafe upstream notice source filename')
            path = upstream_index.parent/notice['file']
            if not path.is_file():
                raise ValueError('Missing indexed upstream notice: '+notice['file'])
            add(path)
    # Only runtime-manifest entries are allowed from engine: no arbitrary files
    # discovered in the dependency directory, and no old UI dependency notices.
    manifest_path = ROOT/'engine/manifest.json'
    if manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
        for row in manifest['files']:
            relative = row['path'].replace('\\', '/')
            if PurePosixPath(relative).is_absolute() or any(part in ('.', '..') or ':' in part for part in PurePosixPath(relative).parts):
                raise ValueError('Unsafe engine source manifest path')
            lower = relative.lower()
            if (lower.startswith(('catalog/', 'plugins/', 'licenses/'))
                    and not lower.startswith(('licenses/webview2', 'licenses/application-dependencies'))):
                add(ROOT/'engine'/relative)
    for name in ['manifest.json', 'downloads.lock.json', 'sources-manifest.json']:
        add(ROOT/'engine'/name)
    # Integration harness is independent from the old frontend's package tests.
    for name in ['run-integration.py', 'site-probe.py', 'integration/Cargo.toml']:
        add(ROOT/'tests'/name)
    tree('tests/integration/src', {'.rs'})
    return sorted(files, key=lambda p: p.relative_to(ROOT).as_posix())


def report(files):
    return {'version': VERSION, 'count': len(files), 'bytes': sum(p.stat().st_size for p in files),
            'files': [p.relative_to(ROOT).as_posix() for p in files]}


def audit_sources(files):
    """Validate pinned source inputs without requiring the excluded binaries."""
    names = {p.relative_to(ROOT).as_posix() for p in files}
    # Mac runtime binaries are intentionally absent. Validate the fixed source
    # inputs and pins, not an imaginary successful Mac build.
    mac_lock = ROOT/'packaging/macos/downloads.lock.json'
    if mac_lock.exists():
        import re
        data = json.loads(mac_lock.read_text(encoding='utf-8'))
        if set(data.get('targets', {})) != {'aarch64-apple-darwin', 'x86_64-apple-darwin'}:
            raise ValueError('Mac lock must cover both native architectures')
        for target in data['targets'].values():
            for row in data['common'] + target['packages']:
                if not re.fullmatch('[a-f0-9]{64}', row.get('sha256', '')):
                    raise ValueError('Mac source lock contains an unpinned runtime')
        for required in ['scripts/macos_common.py', 'scripts/build-macos.py',
                         'scripts/prepare-macos-engines.py', 'scripts/package-macos.py',
                         'scripts/collect-macos-notices.py', 'packaging/macos/downloads.lock.json',
                         'packaging/macos/Info.plist', '.github/workflows/macos-check.yml',
                         'crates/flow-platform/build.rs', 'docs/macos.md']:
            if required not in names:
                raise ValueError('Mac build source missing: ' + required)
    manifest = json.loads((ROOT/'packaging/native-engine-manifest.json').read_text(encoding='utf-8'))
    with (ROOT/'Cargo.lock').open('rb') as stream:
        lock_hash = hashlib.file_digest(stream, 'sha256').hexdigest()
    if manifest.get('cargo_lock_sha256') != lock_hash:
        raise ValueError('Source manifest is stale relative to Cargo.lock; prepare it before archiving')
    download_lock = json.loads((ROOT/'engine/downloads.lock.json').read_text(encoding='utf-8'))
    binaries = {r['path'] for p in download_lock['packages'] for r in p['outputs']
                if not r['path'].lower().startswith('webview2/')}
    for row in manifest['files']:
        name = row['path']
        if name.lower().startswith('webview2/'):
            raise ValueError('Browser resource present in native source manifest')
        origin = ('packaging/notices/'+name.removeprefix('licenses/')
                  if name.startswith('licenses/application-dependencies') else 'engine/'+name)
        if name in binaries:
            continue
        if origin not in names:
            raise ValueError('Required pinned source input absent from allowlist: '+origin)
        path = ROOT/origin
        safe(path)
        with path.open('rb') as stream:
            actual = hashlib.file_digest(stream, 'sha256').hexdigest()
        if actual != row['sha256'] or path.stat().st_size != row['bytes']:
            raise ValueError('Pinned source input changed: '+origin)


def archive_source(files):
    required = {'Cargo.toml', 'Cargo.lock', 'packaging/native-engine-manifest.json',
                'packaging/notices/application-dependencies.json',
                *{f'crates/{crate}/Cargo.toml' for crate in CRATES}}
    present = {p.relative_to(ROOT).as_posix() for p in files}
    if not required <= present:
        raise ValueError('Incomplete native source distribution: '+', '.join(sorted(required-present)))
    audit_sources(files)
    destination = ROOT/'release'/f'liuguang-{VERSION}-native-source.zip'
    safe(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for path in files:
            safe(path)
            info = zipfile.ZipInfo(f'liuguang-{VERSION}-source/'+path.relative_to(ROOT).as_posix(), date_time=(2026, 11, 16, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, path.read_bytes())
    with zipfile.ZipFile(destination) as archive:
        bad = archive.testzip()
        if bad:
            raise ValueError('Source archive CRC verification failed: '+bad)
    with destination.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'file': destination.name, 'bytes': destination.stat().st_size, 'sha256': digest, 'source_files': len(files)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group(required=True)
    actions.add_argument('--list', action='store_true')
    actions.add_argument('--archive', action='store_true')
    args = parser.parse_args()
    files = source_files()
    print(json.dumps(report(files) if args.list else archive_source(files), ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
