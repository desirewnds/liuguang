# SPDX-License-Identifier: Apache-2.0
"""Collect actual native workspace dependency notices without modifying engines.

Cargo must use the project-local cache. Output is a reviewable packaging input,
not an assertion that every license obligation or source requirement is complete.
"""
from pathlib import Path
import hashlib
import json
import os
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT/'packaging/notices'
CACHE = ROOT/'.build/cargo'


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def previous_notices(index_path):
    """Normalize legacy list indexes and the promoted native schema without I/O writes."""
    if not index_path.exists():
        return {}
    data = json.loads(index_path.read_text(encoding='utf-8'))
    legacy = isinstance(data, list)
    if legacy:
        packages = data
    elif isinstance(data, dict) and data.get('schema') == 1 and isinstance(data.get('packages'), list):
        packages = data['packages']
    else:
        raise ValueError('Unsupported archived dependency notice index')
    result = {}
    for package in packages:
        if not isinstance(package, dict):
            raise ValueError('Invalid archived dependency notice package')
        if package.get('ecosystem') != 'cargo':
            continue
        name, version = package.get('name'), package.get('version')
        if not isinstance(name, str) or not isinstance(version, str) or (name, version) in result:
            raise ValueError('Invalid or duplicate archived dependency notice identity')
        entries = package.get('notices', [])
        if not isinstance(entries, list):
            raise ValueError('Invalid archived dependency notice list')
        normalized = []
        for entry in entries:
            if legacy and isinstance(entry, str):
                record = {'file': entry}
            elif isinstance(entry, dict):
                record = {'file': entry.get('file'), 'sha256': entry.get('sha256')}
                expected = record['sha256']
                if not isinstance(expected, str) or not re.fullmatch(r'[0-9a-fA-F]{64}', expected):
                    raise ValueError('Missing or invalid archived native notice hash')
            else:
                raise ValueError('Invalid archived dependency notice entry')
            filename = record['file']
            if (not isinstance(filename, str) or not filename or filename in {'.', '..'}
                    or filename.endswith((' ', '.')) or any(c in filename for c in '\\/:\r\n\0')):
                raise ValueError('Unsafe archived dependency notice filename')
            normalized.append(record)
        result[(name, version)] = {**package, 'notices': normalized}
    return result


def verified_previous_notice(directory, notice, engine_manifest):
    """Read exact old notice bytes; old string entries use the engine SHA pin."""
    filename = notice['file']
    source = (directory/filename).resolve()
    if not source.is_relative_to(directory.resolve()) or not source.is_file():
        raise ValueError('Missing or unsafe archived dependency notice: '+filename)
    expected = notice.get('sha256')
    if expected is None:
        manifest = json.loads(engine_manifest.read_text(encoding='utf-8'))
        relative = 'licenses/application-dependencies/'+filename
        matches = [row for row in manifest['files'] if row.get('path') == relative]
        if len(matches) != 1:
            raise ValueError('Legacy dependency notice has no unique engine SHA pin: '+filename)
        expected = matches[0].get('sha256')
    if not isinstance(expected, str) or not re.fullmatch(r'[0-9a-fA-F]{64}', expected):
        raise ValueError('Missing or invalid archived dependency notice hash: '+filename)
    data = source.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected.lower():
        raise ValueError('Archived dependency notice checksum mismatch: '+filename)
    return data


def main():
    env = {**os.environ, 'CARGO_HOME': str(CACHE), 'CARGO_TARGET_DIR': str(ROOT/'target')}
    result = subprocess.run(['cargo', 'metadata', '--locked', '--offline', '--format-version', '1',
                             '--filter-platform', 'x86_64-pc-windows-gnu', '--manifest-path', str(ROOT/'Cargo.toml')],
                            env=env, check=True, capture_output=True, encoding='utf-8')
    metadata = json.loads(result.stdout)
    previous_index = ROOT/'engine/licenses/application-dependencies.json'
    previous = previous_notices(previous_index)
    upstream_root = ROOT/'packaging/upstream-notices'
    upstream_index = upstream_root/'index.json'
    upstream = json.loads(upstream_index.read_text(encoding='utf-8'))['records'] if upstream_index.exists() else []
    notices = OUTPUT/'application-dependencies'
    notices.mkdir(parents=True, exist_ok=True)
    rows = []
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if not package.get('source'):
            continue
        directory = Path(package['manifest_path']).resolve().parent
        if not directory.is_relative_to(CACHE.resolve()):
            raise SystemExit('Dependency source must be within project-local Cargo cache')
        files = [p for p in directory.iterdir() if p.is_file() and re.match(r'^(licen[cs]e|copying|notice|copyright)([.\-_].*)?$', p.name, re.I)]
        if package.get('license_file'):
            path = (directory/package['license_file']).resolve()
            if path.is_relative_to(directory) and path.is_file() and path not in files:
                files.append(path)
        row = {'ecosystem': 'cargo', 'name': package['name'], 'version': package['version'],
               'license': package.get('license'), 'source': package['source'], 'notices': [],
               'scope': 'Windows GNU workspace dependency; may include build/test dependencies'}
        for path in sorted(files):
            name = re.sub(r'[^A-Za-z0-9_.-]', '_', f"cargo-{package['name']}-{package['version']}-{path.name}.txt")
            target = notices/name
            target.write_bytes(path.read_bytes())
            row['notices'].append({'file': name, 'sha256': sha(target)})
        if not row['notices']:
            sources = []
            for record in upstream:
                if f"{package['name']}@{package['version']}" not in record['packages']:
                    continue
                path = (upstream_root/record['file']).resolve()
                if not path.is_relative_to(upstream_root.resolve()) or not path.is_file():
                    raise SystemExit('Invalid upstream notice path: '+record['file'])
                data = path.read_bytes()
                blob = hashlib.sha1(f'blob {len(data)}\0'.encode()+data).hexdigest()
                if blob != record['git_blob']:
                    raise SystemExit('Upstream notice Git blob mismatch: '+record['file'])
                name = re.sub(r'[^A-Za-z0-9_.-]', '_', f"cargo-{package['name']}-{package['version']}-{record['file']}")
                target = notices/name
                target.write_bytes(data)
                row['notices'].append({'file': name, 'sha256': sha(target)})
                sources.append(record['source'])
                if record.get('declaration_source'):
                    row['license_declaration_source'] = record['declaration_source']
            if sources:
                row['notice_source'] = sources
        if not row['notices']:
            old = previous.get((package['name'], package['version']), {})
            for notice in old.get('notices', []):
                name = notice['file']
                data = verified_previous_notice(ROOT/'engine/licenses/application-dependencies', notice, ROOT/'engine/manifest.json')
                target = notices/name
                target.write_bytes(data)
                row['notices'].append({'file': name, 'sha256': sha(target)})
            if row['notices']:
                row['notice_source'] = old.get('notice_source', 'Previously archived notice for this exact package and version')
        row['status'] = 'notice-archived' if row['notices'] else 'needs-review'
        rows.append(row)
    index = {'schema': 1, 'platform': 'windows-x86_64-gnu', 'cargo_lock_sha256': sha(ROOT/'Cargo.lock'),
             'packages': rows, 'complete_licensing_review': False}
    (OUTPUT/'application-dependencies.json').write_text(json.dumps(index, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    expected = {n['file'] for row in rows for n in row['notices']}
    stale = [p.name for p in notices.iterdir() if p.is_file() and p.name not in expected]
    if stale:
        raise SystemExit('Stale generated notices require explicit review/cleanup: '+', '.join(stale))
    print(json.dumps({'packages': len(rows), 'notice_files': len(expected),
                      'needs_review': [f"{r['name']} {r['version']}" for r in rows if r['status']=='needs-review']}))


if __name__ == '__main__':
    main()
