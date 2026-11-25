# SPDX-License-Identifier: Apache-2.0
"""Shared, project-local macOS packaging checks. No system installation."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import stat
import subprocess
import tarfile
import tempfile
import urllib.parse
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {'aarch64-apple-darwin': 'arm64', 'x86_64-apple-darwin': 'x86_64'}
OWNER = 'org.flowdown.desktop.macos-build.v1'
VERSION = '2.0.0'
MAX_ARCHIVE = 512 * 1024 * 1024


def sha(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def relative_parts(name: str) -> tuple[str, ...]:
    if not name or '\\' in name or '\0' in name:
        raise ValueError('Invalid archive/resource path')
    parts = PurePosixPath(name).parts
    if name.startswith('/') or any(p in ('.', '..') or ':' in p for p in parts):
        raise ValueError('Invalid archive/resource path: ' + name)
    return parts


def child(base: Path, name: str) -> Path:
    """Reject links inside our root; normal system ancestors such as /var may link."""
    path = base
    for part in relative_parts(name):
        path = path / part
        if path.is_symlink() or (path.exists() and getattr(path.lstat(), 'st_file_attributes', 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT):
            raise ValueError('Linked build resource: ' + str(path))
    if not path.resolve().is_relative_to(base.resolve()):
        raise ValueError('Path escaped build directory')
    return path


def area(target: str) -> Path:
    if target not in TARGETS:
        raise ValueError('Unsupported macOS target')
    return child(ROOT, '.build/macos/' + target)


def load_lock():
    path = ROOT / 'packaging/macos/downloads.lock.json'
    data = json.loads(path.read_text(encoding='utf-8'))
    if data.get('schema') != 1 or set(data.get('targets', {})) != set(TARGETS):
        raise ValueError('Invalid macOS download lock')
    return data


def packages(target: str) -> list[dict]:
    lock = load_lock()
    if target not in TARGETS:
        raise ValueError('Unsupported target')
    rows = lock['common'] + lock['targets'][target]['packages']
    output_names = set()
    for row in rows:
        if not re.fullmatch('[a-f0-9]{64}', row.get('sha256', '')):
            raise ValueError('Missing fixed download checksum: ' + row['name'])
        if len(relative_parts(row['file'])) != 1:
            raise ValueError('Archive cache filename must be flat')
        validate_url(row['url'])
        for output in row['outputs']:
            relative_parts(output['path'])
            if output['path'] in output_names:
                raise ValueError('Repeated output path')
            output_names.add(output['path'])
    return rows


def validate_url(url: str, *, redirected=False):
    parsed = urllib.parse.urlparse(url)
    hosts = {'github.com', 'nodejs.org', 'ffmpeg.martin-riedl.de'}
    if redirected:
        hosts |= {'release-assets.githubusercontent.com', 'objects.githubusercontent.com'}
    if parsed.scheme != 'https' or parsed.hostname not in hosts or parsed.username or parsed.password:
        raise ValueError('Unapproved dependency URL: ' + url)


def acquire(row: dict, offline_dir: Path | None = None) -> Path:
    if offline_dir is not None:
        source = child(offline_dir.resolve(), row['file'])
        if not source.is_file() or sha(source) != row['sha256']:
            raise ValueError('Offline archive does not match its pin: ' + row['file'])
        return source
    cache = child(ROOT, '.build/macos/downloads/' + row['file'])
    if cache.exists():
        if sha(cache) == row['sha256']:
            return cache
        raise ValueError('Existing download cache is modified; retained: ' + str(cache))
    cache.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(row['url'], headers={'User-Agent': 'Liuguang-native-build/2.0'})
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=cache.parent, suffix='.partial', delete=False) as stream:
            temporary = Path(stream.name)
            with urllib.request.urlopen(request, timeout=45) as response:
                validate_url(response.url, redirected=True)
                count = 0
                while chunk := response.read(1024 * 1024):
                    count += len(chunk)
                    if count > MAX_ARCHIVE:
                        raise ValueError('Dependency exceeds download size limit')
                    stream.write(chunk)
        if sha(temporary) != row['sha256']:
            raise ValueError('Downloaded checksum mismatch: ' + row['name'])
        temporary.replace(cache)
        return cache
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def read_output(source: Path, row: dict, output: dict) -> bytes:
    """Read selected regular members only; never extract archives wholesale."""
    if row['kind'] == 'binary':
        if source.stat().st_size > MAX_ARCHIVE:
            raise ValueError('Binary too large')
        return source.read_bytes()
    if row['kind'] == 'tar':
        with tarfile.open(source, 'r:*') as archive:
            members = [m for m in archive.getmembers() if m.name == output['member']]
            if len(members) != 1 or not members[0].isfile():
                raise ValueError('Archive member missing, repeated or not regular')
            member = members[0]
            relative_parts(member.name)
            if member.size > MAX_ARCHIVE:
                raise ValueError('Archive member too large')
            stream = archive.extractfile(member)
            if stream is None:
                raise ValueError('Cannot read archive member')
            return stream.read(MAX_ARCHIVE + 1)
    if row['kind'] == 'zip':
        with zipfile.ZipFile(source) as archive:
            members = [m for m in archive.infolist() if not m.is_dir() and
                       ((m.filename == output.get('member')) or
                        ('member_basename' in output and PurePosixPath(m.filename).name == output['member_basename']))]
            if len(members) != 1:
                raise ValueError('ZIP executable must have exactly one matching member')
            member = members[0]
            relative_parts(member.filename)
            mode = member.external_attr >> 16
            if stat.S_ISLNK(mode) or (stat.S_IFMT(mode) not in (0, stat.S_IFREG)):
                raise ValueError('ZIP executable is not a regular file')
            if member.file_size > MAX_ARCHIVE:
                raise ValueError('ZIP executable too large')
            return archive.read(member)
    raise ValueError('Unsupported archive type')


def run(args, *, env=None, capture=True) -> str:
    result = subprocess.run([str(a) for a in args], cwd=ROOT, env=env, check=True,
                            text=True, encoding='utf-8', errors='replace',
                            stdout=subprocess.PIPE if capture else None,
                            stderr=subprocess.PIPE if capture else None)
    return result.stdout or ''


def require_mac(target: str):
    if platform.system() != 'Darwin':
        raise ValueError('Mach-O verification, signing and packaging require macOS; no cross-build success is assumed')
    machine = platform.machine()
    if machine != TARGETS[target]:
        raise ValueError('Use a native ' + TARGETS[target] + ' runner; Rosetta is not a validation substitute')


def version_tuple(value: str):
    values = value.split('.')
    if not values or not all(v.isdigit() for v in values):
        raise ValueError('Invalid deployment version: ' + value)
    return tuple(int(v) for v in values) + (0,) * (3 - len(values))


def minimum_versions(load_commands: str) -> list[str]:
    versions = []
    in_version = False
    for line in load_commands.splitlines():
        words = line.split()
        if words[:1] == ['cmd']:
            in_version = len(words) == 2 and words[1] in ('LC_BUILD_VERSION', 'LC_VERSION_MIN_MACOSX')
        if in_version and len(words) >= 2 and words[0] in ('minos', 'version'):
            version_tuple(words[1])
            versions.append(words[1])
    if not versions:
        raise ValueError('Mach-O lacks a recorded minimum macOS version')
    return versions


def inspect_macho(path: Path, target: str, *, bundle_root: Path, executable_dir: Path, minimum: str) -> dict:
    arch = TARGETS[target]
    arches = run(['/usr/bin/lipo', '-archs', path]).strip().split()
    if arch not in arches:
        raise ValueError('Missing native architecture: ' + str(path))
    load = run(['/usr/bin/otool', '-arch', arch, '-l', path])
    for value in minimum_versions(load):
        if version_tuple(value) > version_tuple(minimum):
            raise ValueError(f'{path.name} requires macOS {value}, above declared {minimum}')
    # Each checked architecture has its own dependency commands/rpaths.
    rpaths = re.findall(r'cmd LC_RPATH\s+cmdsize \d+\s+path (.+?) \(offset', load)
    deps = []
    text = run(['/usr/bin/otool', '-arch', arch, '-L', path])
    for line in text.splitlines():
        if ' (compatibility version ' not in line:
            continue
        name = line.strip().split(' (compatibility version ', 1)[0]
        if name.startswith(('/usr/lib/', '/System/Library/Frameworks/', '/System/Library/PrivateFrameworks/')):
            deps.append({'name': name, 'kind': 'system'})
            continue
        def expand(value):
            if value.startswith('@loader_path/'):
                return path.parent / value.removeprefix('@loader_path/')
            if value.startswith('@executable_path/'):
                return executable_dir / value.removeprefix('@executable_path/')
            if value.startswith('/'):
                return Path(value)
            raise ValueError('Unsupported dynamic-library path: ' + value)
        if name.startswith('@rpath/'):
            candidates = [expand(rp) / name.removeprefix('@rpath/') for rp in rpaths]
        else:
            candidates = [expand(name)]
        matches = [p for p in candidates if p.is_file() and p.resolve().is_relative_to(bundle_root.resolve())]
        if not matches:
            raise ValueError('Non-bundled dynamic library (Homebrew/system-installed libraries are not allowed): ' + name)
        resolved = matches[0].resolve()
        deps.append({'name': name, 'kind': 'bundled', 'path': str(resolved)})
    return {'architectures': arches, 'checked_architecture': arch,
            'minimum_macos': minimum_versions(load), 'dependencies': deps}


def audit_macho_tree(entries: list[Path], target: str, bundle_root: Path, minimum: str) -> dict:
    pending = [(p, p.parent) for p in entries]
    result = {}
    while pending:
        path, executable_dir = pending.pop()
        key = str(path.resolve()) + '|' + str(executable_dir.resolve())
        if key in result:
            continue
        row = inspect_macho(path, target, bundle_root=bundle_root, executable_dir=executable_dir, minimum=minimum)
        result[key] = row
        pending.extend((Path(d['path']), executable_dir) for d in row['dependencies'] if d['kind'] == 'bundled')
    return result


def sign(path: Path, identity: str, entitlements: Path | None = None):
    args = ['/usr/bin/codesign', '--force', '--sign', identity]
    if identity == '-':
        args += ['--timestamp=none']
    else:
        args += ['--options', 'runtime', '--timestamp']
    if entitlements is not None:
        args += ['--entitlements', entitlements]
    run([*args, path])
    run(['/usr/bin/codesign', '--verify', '--strict', path])


def file_rows(base: Path) -> list[dict]:
    rows = []
    for path in sorted(base.rglob('*')):
        if path.is_symlink():
            raise ValueError('Unexpected linked package member: ' + str(path))
        if path.is_file():
            rows.append({'path': path.relative_to(base).as_posix(), 'bytes': path.stat().st_size, 'sha256': sha(path)})
    return rows


def verify_rows(base: Path, rows: list[dict], *, exact=True):
    expected = set()
    for row in rows:
        path = child(base, row['path'])
        if row['path'] in expected or not path.is_file() or path.stat().st_size != row['bytes'] or sha(path) != row['sha256']:
            raise ValueError('Modified, missing or repeated package resource: ' + row['path'])
        expected.add(row['path'])
    if exact and {row['path'] for row in file_rows(base)} != expected:
        raise ValueError('Untracked files in owned build directory; retained')


def verify_engine(base: Path, manifest_path: Path) -> dict:
    data = manifest_path.read_bytes()
    manifest = json.loads(data)
    verify_rows(base, manifest['files'], exact=False)
    expected = {row['path'] for row in manifest['files']} | {'manifest.json'}
    if {row['path'] for row in file_rows(base)} != expected:
        raise ValueError('Untracked or missing engine files')
    if child(base, 'manifest.json').read_bytes() != data:
        raise ValueError('On-disk and compile-time engine manifests differ')
    return manifest


def clean_owned(base: Path, manifest: Path):
    if not base.exists():
        return
    if not base.resolve().is_relative_to((ROOT / '.build/macos').resolve()):
        raise ValueError('Cleanup outside macOS build area')
    value = json.loads(manifest.read_text(encoding='utf-8'))
    if value.get('owner') != OWNER:
        raise ValueError('Unowned build area; retained')
    verify_rows(base, value['files'])
    # Remove only the verified snapshot. Unknown new files are never swept.
    for row in value['files']:
        child(base, row['path']).unlink()
    for directory in sorted([p for p in base.rglob('*') if p.is_dir()], key=lambda p: len(p.parts), reverse=True):
        directory.rmdir()
    base.rmdir()
    manifest.unlink()
