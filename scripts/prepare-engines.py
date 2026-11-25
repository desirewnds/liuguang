# SPDX-License-Identifier: Apache-2.0
"""Restore SHA-256-pinned Windows dependencies from official sources or archives.

No downloaded program is executed. Windows expand.exe only extracts the pinned
Microsoft CAB. Authenticode checks run before promoting the fixed WebView2 files.
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import stat
import subprocess
import tempfile
import urllib.request
import urllib.parse
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / 'engine'
OFFICIAL_HOSTS = {'github.com', 'nodejs.org', 'msedge.sf.dl.delivery.mp.microsoft.com'}


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def matches(path, expected):
    return path.is_file() and digest(path) == expected


def is_reparse(path):
    try:
        return path.is_symlink() or (
            os.name == 'nt' and bool(path.lstat().st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT)
        )
    except FileNotFoundError:
        return False


def safe_path(base, relative):
    if is_reparse(base):
        raise ValueError('Refusing dependency base reparse point: ' + str(base))
    base = base.resolve()
    path = base / relative
    if Path(relative).is_absolute() or '..' in Path(relative).parts:
        raise ValueError('Unsafe dependency path: ' + relative)
    if not path.resolve().is_relative_to(base):
        raise ValueError('Dependency escapes its directory: ' + relative)
    for parent in [path, *path.parents]:
        if parent == base:
            break
        if is_reparse(parent):
            raise ValueError('Refusing dependency reparse point: ' + str(parent))
    return path


def verify_signatures(package, runtime_root):
    if not package.get('authenticode'):
        return
    if os.name != 'nt':
        raise RuntimeError('Fixed WebView2 Authenticode verification requires Windows')
    powershell = Path(os.environ['SystemRoot']) / 'System32/WindowsPowerShell/v1.0/powershell.exe'
    subprocess.run([
        str(powershell), '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', str(ROOT / 'scripts/verify-webview2.ps1'),
        '-RuntimePath', str(runtime_root),
        '-LockPath', str(ENGINE / 'downloads.lock.json'),
    ], check=True)


def verify_runtime_members(package, allow_missing=False):
    if package.get('kind') != 'cab':
        return
    runtime = safe_path(ENGINE, package['runtime_dir'])
    actual = set()
    pending = [runtime] if runtime.exists() else []
    while pending:
        for path in pending.pop().iterdir():
            if is_reparse(path):
                raise RuntimeError('Fixed Runtime contains a reparse point: ' + str(path))
            if path.is_dir():
                pending.append(path)
            elif path.is_file():
                actual.add(path.relative_to(runtime).as_posix())
    expected = {item['member'].replace('\\', '/') for item in package['outputs']}
    if actual - expected or (not allow_missing and actual != expected):
        raise RuntimeError('Fixed Runtime contains missing or unexpected files; review the directory before rebuilding')


def restore(package, source):
    outputs = package['outputs']
    verify_runtime_members(package, allow_missing=True)
    with tempfile.TemporaryDirectory(dir=ENGINE, prefix='.prepare-') as temp:
        staging = Path(temp)
        if package.get('archive_runtime_dir'):
            prefix = package['archive_runtime_dir'].rstrip('/') + '/'
            with zipfile.ZipFile(source) as archive:
                actual = [name.removeprefix(prefix) for name in archive.namelist()
                          if name.startswith(prefix) and not name.endswith('/')]
            expected = package['archive_runtime_members']
            if len(actual) != len(set(actual)) or set(actual) != set(expected):
                raise RuntimeError('ZIP runtime contents differ from the locked build: ' + package['name'])
        if package.get('kind') == 'cab':
            if os.name != 'nt':
                raise RuntimeError('Preparing fixed WebView2 requires Windows expand.exe')
            expand = Path(os.environ['SystemRoot']) / 'System32/expand.exe'
            subprocess.run([str(expand), str(source), '-F:*', str(staging)],
                           capture_output=True, check=True)
            extracted = safe_path(staging, package['archive_root'])
            expected = {item['member'].replace('\\', '/') for item in outputs}
            actual = {p.relative_to(extracted).as_posix() for p in extracted.rglob('*') if p.is_file()}
            if actual != expected:
                raise RuntimeError('CAB contents differ from the locked fixed runtime')
        staged = []
        for index, item in enumerate(outputs):
            target = safe_path(ENGINE, item['path'])
            if package.get('kind') == 'cab':
                stage = safe_path(extracted, item['member'])
            else:
                stage = staging / str(index)
                if item.get('member'):
                    with zipfile.ZipFile(source) as archive:
                        with archive.open(item['member']) as src, stage.open('wb') as dst:
                            shutil.copyfileobj(src, dst)
                else:
                    shutil.copy2(source, stage)
            if not matches(stage, item['sha256']):
                raise RuntimeError('Extracted dependency checksum mismatch: ' + item['path'])
            if 'bytes' in item and stage.stat().st_size != item['bytes']:
                raise RuntimeError('Extracted dependency size mismatch: ' + item['path'])
            staged.append((stage, target))
        if package.get('kind') == 'cab':
            verify_signatures(package, extracted)
        for stage, target in staged:
            target.parent.mkdir(parents=True, exist_ok=True)
            stage.replace(target)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--offline-dir', type=Path, help='Exact locked package filenames; disables network')
    parser.add_argument('--verify-only', action='store_true', help='Verify files and signatures without changing files')
    args = parser.parse_args()
    lock = json.loads((ENGINE / 'downloads.lock.json').read_text(encoding='utf-8'))
    for package in lock['packages']:
        outputs = package['outputs']
        if all(matches(safe_path(ENGINE, item['path']), item['sha256']) for item in outputs):
            if package.get('kind') == 'cab':
                verify_runtime_members(package)
                verify_signatures(package, safe_path(ENGINE, package['runtime_dir']))
            print('Verified ' + package['name'])
            continue
        if args.verify_only:
            raise SystemExit('Missing or mismatched dependency: ' + package['name'])
        cache = safe_path(ENGINE / 'sources', package['file'])
        if args.offline_dir:
            source = safe_path(args.offline_dir, package['file'])
            if not matches(source, package['sha256']):
                raise SystemExit('Missing or mismatched explicit archive: ' + str(source))
        else:
            if not matches(cache, package['sha256']):
                url = urllib.parse.urlparse(package['url'])
                if url.scheme != 'https' or url.hostname not in OFFICIAL_HOSTS:
                    raise SystemExit('Untrusted download URL in lock')
                cache.parent.mkdir(parents=True, exist_ok=True)
                with tempfile.NamedTemporaryFile(dir=cache.parent, suffix='.partial', delete=False) as temp:
                    temp_path = Path(temp.name)
                    try:
                        request = urllib.request.Request(package['url'], headers={'User-Agent': 'Liuguang-build/1.0'})
                        with urllib.request.urlopen(request, timeout=60) as response:
                            shutil.copyfileobj(response, temp)
                    except Exception:
                        temp.close()
                        temp_path.unlink(missing_ok=True)
                        raise
                if not matches(temp_path, package['sha256']):
                    temp_path.unlink(missing_ok=True)
                    raise SystemExit('Downloaded package checksum mismatch: ' + package['name'])
                temp_path.replace(cache)
            source = cache
        restore(package, source)
        print('Prepared ' + package['name'])
    print('All engine binaries and private WebView2 resources match the pinned releases.')


if __name__ == '__main__':
    main()
