# SPDX-License-Identifier: Apache-2.0
"""Build and verify a standalone Iced payload/NSIS package; never install or upload.

Runtime staging uses verified hardlinks where supported. Only explicitly owned
stage files are replaced or cleaned, and the canonical engine is never modified.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess

ROOT = Path(__file__).resolve().parents[1]
VERSION = '2.0.0'
OWNER = 'org.flowdown.desktop.native-package.v1'
STAGE = ROOT/'target/native-package'
PAYLOAD = STAGE/'payload'
MANIFEST = STAGE/'package-manifest.json'


def digest(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')


def reject_links(path: Path) -> None:
    current = path.absolute()
    while True:
        if current.exists() or current.is_symlink():
            attrs = current.lstat()
            if current.is_symlink() or getattr(attrs, 'st_file_attributes', 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
                raise ValueError(f'Refusing linked package path: {current}')
        if current.parent == current:
            break
        current = current.parent


def safe_child(base: Path, relative: str) -> Path:
    normalized = relative.replace('\\', '/')
    parts = PurePosixPath(normalized).parts
    if not parts or any(part in ('..', '.', '') or ':' in part for part in parts) or normalized.startswith('/'):
        raise ValueError(f'Invalid relative package path: {relative}')
    path = base.joinpath(*parts)
    reject_links(path)
    if not path.resolve().is_relative_to(base.resolve()):
        raise ValueError(f'Package path escapes root: {relative}')
    return path


def is_browser(relative: str) -> bool:
    value = relative.replace('\\', '/').lower()
    return value.startswith('webview2/') or value.startswith('licenses/webview2')


def is_old_application_notice(relative: str) -> bool:
    value = relative.replace('\\', '/').lower()
    return value == 'licenses/application-dependencies.json' or value.startswith('licenses/application-dependencies/')


def native_notice_resources(*, allow_incomplete: bool = False) -> list[dict]:
    base = ROOT/'packaging/notices'
    index_path = base/'application-dependencies.json'
    reject_links(index_path)
    if not index_path.is_file():
        raise ValueError('Generate native dependency notices before preparing or staging a release')
    index = json.loads(index_path.read_text(encoding='utf-8'))
    if index.get('schema') != 1 or index.get('platform') != 'windows-x86_64-gnu' or index.get('cargo_lock_sha256') != digest(ROOT/'Cargo.lock'):
        raise ValueError('Native dependency notices do not match Cargo.lock and Windows GNU')
    names = {}
    for package in index['packages']:
        if not allow_incomplete and (package.get('status') != 'notice-archived' or not package.get('notices')):
            raise ValueError(f'Native dependency notice requires review: {package.get("name")} {package.get("version")}')
        for notice in package['notices']:
            name = notice['file']
            if '/' in name or '\\' in name:
                raise ValueError('Invalid native notice filename')
            path = safe_child(base/'application-dependencies', name)
            if not path.is_file() or digest(path) != notice['sha256']:
                raise ValueError(f'Native dependency notice mismatch: {name}')
            if name.lower() in names and names[name.lower()] != name:
                raise ValueError('Case-colliding native notice filenames')
            names[name.lower()] = name
    actual = {p.name for p in (base/'application-dependencies').iterdir() if p.is_file()}
    if actual != set(names.values()):
        raise ValueError('Unlisted or missing native dependency notices')
    paths = [index_path] + [base/'application-dependencies'/name for name in sorted(names.values())]
    return [{'path': 'licenses/'+path.relative_to(base).as_posix(), 'bytes': path.stat().st_size, 'sha256': digest(path)} for path in paths]


def resource_source(name: str) -> Path:
    if is_old_application_notice(name):
        return safe_child(ROOT/'packaging/notices', name.removeprefix('licenses/'))
    return safe_child(ROOT/'engine', name)


def native_resources(*, require_notices: bool = True, allow_incomplete_notices: bool = False) -> dict:
    source = json.loads((ROOT/'engine/manifest.json').read_text(encoding='utf-8'))
    result = {**source, 'versions': {k: v for k, v in source['versions'].items() if k.lower() != 'webview2'},
              'files': [item for item in source['files'] if not is_browser(item['path']) and not is_old_application_notice(item['path'])]}
    if require_notices:
        result['files'].extend(native_notice_resources(allow_incomplete=allow_incomplete_notices))
        result['cargo_lock_sha256'] = digest(ROOT/'Cargo.lock')
    result['files'].sort(key=lambda row: row['path'])
    seen = set()
    for row in result['files']:
        name = row['path']
        if name.lower() in seen:
            raise ValueError(f'Duplicate runtime destination: {name}')
        seen.add(name.lower())
        path = resource_source(name)
        if not path.is_file() or path.stat().st_size != row['bytes'] or digest(path) != row['sha256']:
            raise ValueError(f'Pinned resource mismatch: {name}')
    return result


def prepared_manifest() -> Path:
    return ROOT/'packaging/native-engine-manifest.json'


def prepare_manifest(*, allow_incomplete_notices: bool = False) -> dict:
    resources = native_resources(allow_incomplete_notices=allow_incomplete_notices)
    path = prepared_manifest()
    reject_links(path)
    write_json(path, resources)
    return {'manifest': str(path), 'files': len(resources['files']), 'cargo_lock_sha256': resources['cargo_lock_sha256']}


def check_prepared_manifest(resources: dict) -> None:
    path = prepared_manifest()
    reject_links(path)
    if not path.is_file() or json.loads(path.read_text(encoding='utf-8')) != resources:
        raise ValueError('Prepare the native resource manifest and rebuild before staging')


def check_compiled_manifest(binary: Path, resources: dict) -> None:
    # include_str! is retained verbatim by the CLI's explicit read-only manifest
    # command. Check its actual bytes without executing a possibly stale binary
    # from a previous build, which might not recognize the helper switch.
    data = prepared_manifest().read_bytes()
    if json.loads(data) != resources or data not in binary.read_bytes():
        raise ValueError('Native CLI was compiled with different resources; rebuild before staging')


def clean_stage() -> None:
    reject_links(STAGE)
    if not STAGE.exists():
        return
    if not MANIFEST.is_file():
        if any(STAGE.iterdir()):
            raise ValueError('Unowned package stage; preserve it and investigate before cleanup')
        STAGE.rmdir()
        return
    previous = json.loads(MANIFEST.read_text(encoding='utf-8'))
    if previous.get('owner') != OWNER:
        raise ValueError('Unrecognized native package stage owner')
    paths = []
    for row in previous['files']:
        path = safe_child(PAYLOAD, row['path'])
        if path.exists():
            if digest(path) != row['sha256']:
                raise ValueError(f'Modified stage file retained: {path}')
            paths.append(path)
    generated = previous.get('generated', [])
    for row in generated:
        relative = row['path']
        path = safe_child(STAGE, relative)
        if path.exists():
            if digest(path) != row['sha256']:
                raise ValueError(f'Modified generated stage file retained: {path}')
            paths.append(path)
    # engine-manifest.json is generated before the first payload entry. Its
    # exact bytes must still match this prepared compile-time input on retry.
    engine_manifest = STAGE/'engine-manifest.json'
    if engine_manifest.exists() and engine_manifest not in paths:
        if digest(engine_manifest) != digest(prepared_manifest()):
            raise ValueError('Modified generated engine manifest retained')
        paths.append(engine_manifest)
    allowed = {p.resolve() for p in paths} | {MANIFEST.resolve()}
    actual = list(STAGE.rglob('*'))
    for path in actual:
        reject_links(path)
        if path.is_file() and path.resolve() not in allowed:
            raise ValueError(f'Untracked package stage file retained: {path}')
    for path in paths:
        path.unlink()
    MANIFEST.unlink()
    for path in sorted((p for p in actual if p.is_dir()), key=lambda p: len(p.parts), reverse=True):
        path.rmdir()
    STAGE.rmdir()


def link_or_copy(source: Path, destination: Path) -> None:
    reject_links(source)
    reject_links(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.link(source, destination)
    except OSError:
        shutil.copyfile(source, destination)


def nsis(value: str) -> str:
    if any(character in value for character in ('\n', '\r', '\0')):
        raise ValueError('Control character in NSIS input')
    return value.replace('$', '$$').replace('"', '$\\"')


def generate_installer(manifest: dict) -> None:
    names = [row['path'] for row in manifest['files']]
    directories = set()
    for name in names:
        path = PurePosixPath(name).parent
        while path != PurePosixPath('.'):
            directories.add(path.as_posix())
            path = path.parent
    lines = ['; Generated only from the verified package manifest.', '!macro CheckPayloadPaths PREFIX']
    for name in names + ['uninstall.exe', '.flow-install-owner.json', '.flow-install-pending.json']:
        lines.append(f'  !insertmacro RequireSafePath "${{PREFIX}}" "$INSTDIR\\{nsis(name.replace("/", chr(92)))}"')
    lines += ['!macroend', '!macro ExtractPayload', '  CreateDirectory "$PLUGINSDIR\\payload"', '  SetOutPath "$PLUGINSDIR\\payload"']
    for directory in sorted(directories, key=lambda v: (v.count('/'), v)):
        lines.append(f'  CreateDirectory "$PLUGINSDIR\\payload\\{nsis(directory.replace("/", chr(92)))}"')
    for name in names:
        lines.append(f'  File "/oname={nsis(name.replace("/", chr(92)))}" "{nsis(str(PAYLOAD/Path(name)))}"')
    lines += ['!macroend', '!macro DeletePayloadResources']
    for name in names:
        if name not in ('flow-cli.exe', 'flow-desktop.exe'):
            lines.append(f'  !insertmacro DeleteOwnedFile "$INSTDIR\\{nsis(name.replace("/", chr(92)))}"')
    lines += ['!macroend', '!macro RemoveEmptyPayloadDirectories']
    for directory in sorted(directories, key=lambda v: (v.count('/'), v), reverse=True):
        lines.append(f'  RMDir "$INSTDIR\\{nsis(directory.replace("/", chr(92)))}"')
    lines += ['!macroend', '']
    (STAGE/'payload.nsh').write_text('\n'.join(lines), encoding='utf-8')
    settings = {
        'FLOW_VERSION': VERSION,
        'FLOW_OUTFILE': str(ROOT/'release'/f'liuguang-{VERSION}-windows-x64-setup.exe'),
        'FLOW_HELPER': str(PAYLOAD/'flow-cli.exe'),
        'FLOW_PACKAGE_MANIFEST': str(MANIFEST),
        'FLOW_ICON': str(ROOT/'packaging/icon.ico'),
    }
    text = '\n'.join(f'!define {key} "{nsis(value)}"' for key, value in settings.items())
    text += f'\n!define FLOW_SIZE_KIB {(sum(row["bytes"] for row in manifest["files"])+1023)//1024}\n'
    (STAGE/'settings.nsh').write_text(text, encoding='utf-8')
    shutil.copyfile(ROOT/'packaging/installer.nsi', STAGE/'installer.nsi')
    manifest['generated'] = [{'path': name, 'sha256': digest(STAGE/name)} for name in
                             ['engine-manifest.json', 'settings.nsh', 'payload.nsh', 'installer.nsi']]
    write_json(MANIFEST, manifest)


def stage_bundle(*, verify_binary: bool = True) -> dict:
    resources = native_resources()
    check_prepared_manifest(resources)
    inputs = [('engine/'+row['path'], resource_source(row['path'])) for row in resources['files']]
    for name in ['flow-desktop.exe', 'flow-cli.exe']:
        binary = ROOT/'target/release'/name
        if not binary.is_file() or binary.read_bytes()[:2] != b'MZ':
            raise ValueError(f'Build the native Windows release first: {binary}')
        inputs.append((name, binary))
    if verify_binary:
        check_compiled_manifest(ROOT/'target/release/flow-cli.exe', resources)
    inputs += [('flow.cmd', ROOT/'packaging/flow.cmd'),
               ('engine/licenses/THIRD_PARTY_NOTICES.md', ROOT/'THIRD_PARTY_NOTICES.md'),
               ('engine/licenses/source-status.md', ROOT/'docs/third-party-source-status.md')]
    for _, path in inputs:
        reject_links(path)
    clean_stage()
    STAGE.mkdir(parents=True)
    # Save each completed entry so interruption leaves a safely recoverable stage.
    manifest = {'schema': 1, 'owner': OWNER, 'version': VERSION, 'platform': 'windows-x86_64', 'files': []}
    write_json(MANIFEST, manifest)
    write_json(STAGE/'engine-manifest.json', resources)
    inputs.append(('engine/manifest.json', STAGE/'engine-manifest.json'))
    names = set()
    for relative, source in inputs:
        if relative.lower() in names:
            raise ValueError(f'Duplicate package path: {relative}')
        names.add(relative.lower())
        destination = safe_child(PAYLOAD, relative)
        link_or_copy(source, destination)
        manifest['files'].append({'path': relative, 'bytes': destination.stat().st_size, 'sha256': digest(destination)})
        write_json(MANIFEST, manifest)
    manifest['bytes'] = sum(row['bytes'] for row in manifest['files'])
    write_json(MANIFEST, manifest)
    generate_installer(manifest)
    return manifest


def audit_bundle() -> dict:
    reject_links(STAGE)
    manifest = json.loads(MANIFEST.read_text(encoding='utf-8'))
    if manifest.get('owner') != OWNER or manifest.get('version') != VERSION:
        raise ValueError('Native package identity mismatch')
    generated = manifest.get('generated', [])
    if {row['path'] for row in generated} != {'engine-manifest.json', 'settings.nsh', 'payload.nsh', 'installer.nsi'}:
        raise ValueError('Native installer generation is incomplete')
    for row in generated:
        if digest(safe_child(STAGE, row['path'])) != row['sha256']:
            raise ValueError('Native installer script integrity mismatch')
    names = set()
    for row in manifest['files']:
        name = row['path']
        if name.lower() in names:
            raise ValueError('Duplicate package destination')
        names.add(name.lower())
        if name.lower().startswith('engine/webview2/') or 'webview2loader' in name.lower() or '/sources/' in name.lower():
            raise ValueError(f'Forbidden native payload: {name}')
        path = safe_child(PAYLOAD, name)
        if path.stat().st_size != row['bytes'] or digest(path) != row['sha256']:
            raise ValueError(f'Native package integrity mismatch: {name}')
    actual = {p.relative_to(PAYLOAD).as_posix().lower() for p in PAYLOAD.rglob('*') if p.is_file()}
    if actual != names:
        raise ValueError('Unlisted or missing payload files')
    resources = json.loads((PAYLOAD/'engine/manifest.json').read_text(encoding='utf-8'))
    expected = native_resources()
    check_prepared_manifest(expected)
    if resources != expected:
        raise ValueError('Native resources differ from the pinned canonical inputs')
    required = {'flow-desktop.exe', 'flow-cli.exe', 'flow.cmd', 'engine/node.exe', 'engine/yt-dlp.exe',
                'engine/ffmpeg.exe', 'engine/ffprobe.exe', 'engine/downloads.lock.json'}
    if not required <= names:
        raise ValueError(f'Missing application dependencies: {required-names}')
    return {'version': VERSION, 'files': len(names), 'bytes': sum(row['bytes'] for row in manifest['files']),
            'webview2': False, 'runtime_verified': True}


def audit_imports() -> dict:
    """Prove ordinary/delay PE imports close over same-directory DLLs + Windows.

    This does not claim to enumerate optional GPU driver DLLs loaded dynamically.
    """
    spec = importlib.util.spec_from_file_location('flow_pe_audit', ROOT/'scripts/verify-ffmpeg-shared.py')
    parser = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parser)
    windows = parser.WINDOWS_IMPORTS | {
        'uxtheme.dll', 'hid.dll', 'oleacc.dll', 'd3dcompiler_47.dll',
        'msctf.dll', 'profapi.dll', 'shcore.dll', 'win32u.dll', 'dinput8.dll',
        # Windows inbox APIs; provenance is recorded in docs/verification-2.0.0.md.
        'combase.dll', 'opengl32.dll', 'dbghelp.dll',
    }
    report = {}
    for path in sorted(PAYLOAD.rglob('*')):
        if path.suffix.lower() not in ('.exe', '.dll'):
            continue
        info = parser.pe_imports(path)
        siblings = {p.name.lower() for p in path.parent.iterdir() if p.is_file()}
        dependencies = set(info['imports']+info['delay_imports'])
        unresolved = sorted(name for name in dependencies if name not in windows and name not in siblings
                            and not name.startswith(('api-ms-win-', 'ext-ms-win-')))
        if unresolved:
            raise ValueError(f'Unresolved native imports for {path.name}: {unresolved}')
        report[path.relative_to(PAYLOAD).as_posix()] = info
    return report


def compile_installer(makensis: Path, *, verify_imports: bool = True) -> dict:
    result = audit_bundle()
    if verify_imports:
        result['pe_imports'] = audit_imports()
    compiler = makensis.resolve(strict=True)
    reject_links(compiler)
    if compiler.name.lower() != 'makensis.exe':
        raise ValueError('Explicit makensis.exe is required')
    output = ROOT/'release'/f'liuguang-{VERSION}-windows-x64-setup.exe'
    if output.exists():
        raise ValueError(f'Refusing to overwrite an existing release: {output}')
    output.parent.mkdir(exist_ok=True)
    environment = os.environ.copy()
    temporary = ROOT/'.build/tmp'
    temporary.mkdir(parents=True, exist_ok=True)
    for variable in ('TEMP', 'TMP', 'TMPDIR'):
        environment[variable] = str(temporary)
    subprocess.run([str(compiler), '/V2', '/INPUTCHARSET', 'UTF8', str(STAGE/'installer.nsi')],
                   cwd=STAGE, env=environment, check=True)
    result['installer'] = {'file': output.name, 'bytes': output.stat().st_size, 'sha256': digest(output)}
    result['signed'] = False
    write_json(ROOT/'release'/f'liuguang-{VERSION}-manifest.json', result)
    (ROOT/'release'/f'liuguang-{VERSION}-SHA256SUMS.txt').write_text(
        result['installer']['sha256']+'  '+output.name+'\n', encoding='ascii')
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument('--stage', action='store_true')
    action.add_argument('--audit', action='store_true')
    action.add_argument('--compile', action='store_true')
    action.add_argument('--clean', action='store_true')
    action.add_argument('--verify-inputs', action='store_true')
    action.add_argument('--audit-imports', action='store_true')
    action.add_argument('--prepare-manifest', action='store_true')
    parser.add_argument('--makensis', type=Path, help='Existing NSIS compiler; never downloaded or installed')
    parser.add_argument('--allow-incomplete-notices', action='store_true', help='Development manifest only; never permits staging or release')
    args = parser.parse_args()
    if args.allow_incomplete_notices and not args.prepare_manifest:
        parser.error('--allow-incomplete-notices is only valid with --prepare-manifest')
    if args.prepare_manifest:
        print(json.dumps(prepare_manifest(allow_incomplete_notices=args.allow_incomplete_notices)))
    elif args.stage:
        result = stage_bundle()
        print(json.dumps({'files': len(result['files']), 'bytes': result['bytes'], 'stage': str(PAYLOAD)}))
    elif args.audit:
        print(json.dumps(audit_bundle()))
    elif args.compile:
        if not args.makensis:
            parser.error('--compile requires --makensis')
        print(json.dumps(compile_installer(args.makensis), ensure_ascii=False, indent=2))
    elif args.clean:
        clean_stage()
        print('Removed verified native package stage only.')
    elif args.audit_imports:
        audit_bundle()
        print(json.dumps(audit_imports(), indent=2))
    else:
        result = native_resources(require_notices=False)
        print(json.dumps({'files': len(result['files']), 'bytes': sum(row['bytes'] for row in result['files'])}))


if __name__ == '__main__':
    main()
