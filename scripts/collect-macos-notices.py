# SPDX-License-Identifier: Apache-2.0
"""Collect the selected Mac Cargo graph's notices into its private engine tree.

Does not overwrite Windows notices. Missing texts fail the build explicitly.
"""
from pathlib import Path
import argparse
import importlib.util
import json
import os
import re
from macos_common import ROOT, TARGETS, area, child, run, sha, write_json


def collect(target: str, engine: Path):
    cache = ROOT / '.build/cargo'
    env = {**os.environ, 'CARGO_HOME': str(cache), 'CARGO_TARGET_DIR': str(ROOT / 'target')}
    metadata = json.loads(run(['cargo', 'metadata', '--locked', '--offline', '--all-features',
                              '--format-version', '1', '--filter-platform', target,
                              '--manifest-path', ROOT / 'Cargo.toml'], env=env))
    previous_root = ROOT / 'packaging/notices'
    previous_index = json.loads((previous_root / 'application-dependencies.json').read_text(encoding='utf-8'))
    previous = {(p['name'], p['version']): p for p in previous_index['packages']}
    upstream_root = ROOT / 'packaging/upstream-notices'
    upstream = json.loads((upstream_root / 'index.json').read_text(encoding='utf-8'))['records']
    destination = child(engine, 'licenses/application-dependencies')
    destination.mkdir(parents=True, exist_ok=True)
    rows, missing = [], []
    resolved = {node['id'] for node in metadata.get('resolve', {}).get('nodes', [])}
    if not resolved:
        raise ValueError('Cargo metadata did not provide the selected target resolve graph')
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if package['id'] not in resolved or not package.get('source'):
            continue
        directory = Path(package['manifest_path']).resolve().parent
        if not directory.is_relative_to(cache.resolve()):
            raise ValueError('Dependency source outside project Cargo cache')
        files = [p for p in directory.iterdir() if p.is_file() and
                 re.match(r'^(licen[cs]e|unlicense|copying|notice|copyright)([.\-_].*)?$', p.name, re.I)]
        if package.get('license_file'):
            path = (directory / package['license_file']).resolve()
            if path.is_relative_to(directory) and path.is_file() and path not in files:
                files.append(path)
        texts = [(p.name, p.read_bytes()) for p in sorted(files)]
        if not texts:
            for notice in previous.get((package['name'], package['version']), {}).get('notices', []):
                source = child(previous_root / 'application-dependencies', notice['file'])
                if sha(source) != notice['sha256']:
                    raise ValueError('Previously reviewed notice was modified')
                texts.append((notice['file'], source.read_bytes()))
        source_records = []
        if not texts:
            import hashlib
            for record in upstream:
                if f"{package['name']}@{package['version']}" not in record['packages']:
                    continue
                data = child(upstream_root, record['file']).read_bytes()
                if record.get('git_blob'):
                    valid = hashlib.sha1(f'blob {len(data)}\0'.encode() + data).hexdigest() == record['git_blob']
                else:
                    valid = bool(record.get('sha256')) and hashlib.sha256(data).hexdigest() == record['sha256']
                if not valid:
                    raise ValueError('Supplementary notice does not match its recorded content hash')
                texts.append((record['file'], data))
                source_records.append({k: record[k] for k in ('source', 'git_blob', 'sha256', 'license_choice',
                                                              'notice_kind', 'notice_limitation') if k in record})
        row = {'ecosystem': 'cargo', 'name': package['name'], 'version': package['version'],
               'license': package.get('license'), 'source': package['source'], 'notices': [],
               'scope': target + ' Cargo dependency; may include build/test dependencies'}
        for name, data in texts:
            name = re.sub(r'[^A-Za-z0-9_.-]', '_', f"cargo-{package['name']}-{package['version']}-{name}")
            path = child(destination, name)
            if path.exists() and path.read_bytes() != data:
                raise ValueError('Conflicting notice destination')
            path.write_bytes(data)
            row['notices'].append({'file': name, 'sha256': sha(path)})
        row['status'] = 'notice-archived' if texts else 'needs-review'
        if source_records:
            row['notice_source'] = source_records
        rows.append(row)
        if not texts:
            missing.append(package['name'] + '@' + package['version'])
    write_json(engine / 'licenses/application-dependencies.json', {
        'schema': 1, 'platform': target, 'cargo_lock_sha256': sha(ROOT / 'Cargo.lock'),
        'packages': rows, 'complete_licensing_review': False})
    if missing:
        raise ValueError('Mac dependency notices require review: ' + ', '.join(missing))
    return len(rows)


def review_directory(target: str, explicit: Path | None = None) -> Path:
    output = explicit.absolute() if explicit else area(target) / 'notice-review'
    if not output.resolve().is_relative_to((ROOT / '.build/macos').resolve()):
        raise ValueError('Notice review output must remain inside .build/macos')
    output = child(ROOT, output.relative_to(ROOT).as_posix())
    for platform_target in TARGETS:
        for reserved in ('engine', 'bundle'):
            if output.resolve().is_relative_to((area(platform_target) / reserved).resolve()):
                raise ValueError('Use a separate review directory; engine/bundle stages belong to the packager')
    return output


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True, choices=TARGETS)
    parser.add_argument('--output', type=Path, help='Separate review root inside the project; default is target-specific notice-review')
    args = parser.parse_args()
    output = review_directory(args.target, args.output)
    print(collect(args.target, output))
