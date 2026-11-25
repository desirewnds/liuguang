# SPDX-License-Identifier: Apache-2.0
"""Restore only pinned native media/JavaScript dependencies, never WebView2.

No downloaded program is run. Downloads stay in engine/sources, and extraction
reuses the original checksum/path checks. The canonical lock is never rewritten.
"""
from pathlib import Path
import argparse
import importlib.util
import json
import shutil
import tempfile
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT/'engine'


def native_packages(lock):
    selected = []
    for package in lock['packages']:
        outputs = package['outputs']
        if not outputs:
            raise ValueError('Dependency has no locked outputs')
        browser = [item['path'].replace('\\', '/').lower().startswith('webview2/') for item in outputs]
        if any(browser):
            if not all(browser):
                raise ValueError('Mixed browser/media dependency must be reviewed')
            continue
        if package.get('kind') == 'cab' or package.get('runtime_dir', '').lower() == 'webview2':
            raise ValueError('Unexpected browser/CAB dependency in native lock selection')
        selected.append(package)
    return selected


def load_checks():
    spec = importlib.util.spec_from_file_location('flow_locked_engine_checks', ROOT/'scripts/prepare-engines.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def prepare(packages, *, offline_dir=None, verify_only=False):
    checks = load_checks()
    for package in packages:
        if all(checks.matches(checks.safe_path(ENGINE, row['path']), row['sha256']) for row in package['outputs']):
            print('Verified '+package['name'])
            continue
        if verify_only:
            raise ValueError('Missing or mismatched native dependency: '+package['name'])
        cache = checks.safe_path(ENGINE/'sources', package['file'])
        if offline_dir:
            source = checks.safe_path(offline_dir, package['file'])
            if not checks.matches(source, package['sha256']):
                raise ValueError('Missing or mismatched explicit archive: '+str(source))
        else:
            if not checks.matches(cache, package['sha256']):
                url = urllib.parse.urlparse(package['url'])
                # Deliberately narrower than the legacy helper: no Microsoft
                # browser host, browser CAB or system-runtime installation.
                if url.scheme != 'https' or url.hostname not in {'github.com', 'nodejs.org'}:
                    raise ValueError('Untrusted native dependency URL')
                cache.parent.mkdir(parents=True, exist_ok=True)
                with tempfile.NamedTemporaryFile(dir=cache.parent, suffix='.partial', delete=False) as temporary:
                    staged = Path(temporary.name)
                    try:
                        request = urllib.request.Request(package['url'], headers={'User-Agent': 'Liuguang-build/2.0'})
                        with urllib.request.urlopen(request, timeout=60) as response:
                            shutil.copyfileobj(response, temporary)
                    except Exception:
                        temporary.close()
                        staged.unlink(missing_ok=True)
                        raise
                if not checks.matches(staged, package['sha256']):
                    staged.unlink(missing_ok=True)
                    raise ValueError('Downloaded native dependency checksum mismatch')
                staged.replace(cache)
            source = cache
        checks.restore(package, source)
        print('Prepared '+package['name'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--offline-dir', type=Path, help='Explicit archive directory; disables downloads')
    parser.add_argument('--verify-only', action='store_true', help='Only read/hash existing native dependencies')
    parser.add_argument('--list', action='store_true', help='List locked selection without downloads or writes')
    args = parser.parse_args()
    lock = json.loads((ENGINE/'downloads.lock.json').read_text(encoding='utf-8'))
    packages = native_packages(lock)
    if args.list:
        print(json.dumps({'packages': [p['name'] for p in packages],
                          'files': [r['path'] for p in packages for r in p['outputs']],
                          'webview2': False}, ensure_ascii=False, indent=2))
        return
    prepare(packages, offline_dir=args.offline_dir, verify_only=args.verify_only)
    print('All native engine outputs match their pinned hashes; WebView2 was not selected.')


if __name__ == '__main__':
    main()
