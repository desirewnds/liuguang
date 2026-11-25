# SPDX-License-Identifier: Apache-2.0
"""Restore hash-pinned private Mac runtimes, audit them and freeze a compile input.

--list is read-only and works on Windows. Preparation runs on the native Mac
architecture. The existing Windows engine tree and manifests are never changed.
"""
from pathlib import Path
import argparse
import importlib.util
import json
import os
import re
import shutil
from macos_common import (ROOT, TARGETS, OWNER, acquire, area, audit_macho_tree, child,
                          clean_owned, file_rows, load_lock, packages, read_output,
                          require_mac, run, sha, sign, verify_engine, write_json)


def capability_report(engine: Path) -> dict:
    binary = engine / 'ffmpeg'
    environment = {k: v for k, v in os.environ.items()
                   if k not in {'NODE_OPTIONS', 'FFREPORT', 'PYTHONPATH', 'PYTHONHOME'}
                   and not k.startswith('DYLD_')}
    output = {name: run([binary, '-hide_banner', '-' + name], env=environment)
              for name in ('version', 'encoders', 'decoders', 'muxers', 'demuxers', 'filters', 'protocols')}
    # Preserve the application's complete audio/video/subtitle workflows. This
    # is deliberately not a claim that Windows-only hardware APIs exist on Mac.
    required = {
        'encoders': ['libx264', 'libx265', 'aac', 'libmp3lame', 'libopus', 'flac', 'alac', 'pcm_s16le',
                     'mov_text', 'srt', 'ass', 'webvtt', 'mjpeg', 'png'],
        'decoders': ['h264', 'hevc', 'vp9', 'aac', 'opus', 'subrip'],
        'muxers': ['mp4', 'matroska', 'webm', 'mp3', 'ipod', 'flac', 'wav', 'ogg', 'adts', 'srt', 'webvtt', 'image2'],
        'demuxers': ['mov', 'matroska', 'mpegts', 'hls', 'concat'],
        'filters': ['scale', 'aresample', 'trim', 'atrim', 'subtitles', 'metadata', 'ametadata', 'thumbnail'],
        'protocols': ['file', 'pipe', 'http', 'https', 'tcp', 'tls'],
    }
    for kind, names in required.items():
        for name in names:
            if not re.search(r'(?<![A-Za-z0-9_])' + re.escape(name) + r'(?![A-Za-z0-9_])', output[kind]):
                raise ValueError(f'FFmpeg missing {kind} capability required by Flow: {name}')
    if '--enable-nonfree' in output['version']:
        raise ValueError('Nonredistributable FFmpeg build rejected')
    if '9.0.2' not in output['version'] or '--enable-gpl' not in output['version']:
        raise ValueError('Unexpected pinned FFmpeg identity/configuration')
    full_help = run([binary, '-hide_banner', '-h', 'full'], env=environment)
    for option in ('-map_metadata', '-map_chapters', '-attach', '-disposition', '-ss', '-t'):
        if not re.search(r'(?m)^\s*' + re.escape(option) + r'(?:\s|\[|:)', full_help):
            raise ValueError('FFmpeg missing metadata/attachment/section option: ' + option)
    probe = run([engine / 'ffprobe', '-hide_banner', '-version'], env=environment)
    if '9.0.2' not in probe:
        raise ValueError('FFprobe does not match FFmpeg version')
    versions = {'ffmpeg': output['version'].splitlines()[0], 'ffprobe': probe.splitlines()[0],
                'yt-dlp': run([engine / 'yt-dlp', '--ignore-config', '--no-cache-dir', '--version'], env=environment).strip(),
                'node': run([engine / 'node', '--version'], env=environment).strip()}
    if versions['yt-dlp'] != '2026.10.21' or versions['node'] != 'v24.15.0':
        raise ValueError('Runtime version output does not match download lock')
    if run([engine / 'node', '-e', 'process.stdout.write(String(new Function("return 40 + 2")()))'], env=environment) != '42':
        raise ValueError('The signed Node runtime cannot execute JavaScript')
    return {'required': required, 'reported': output, 'versions': versions,
            'real_media_smoke_test': False, 'all_windows_ffmpeg_features_equivalent': False}


def shared_sources(engine: Path):
    # These are app-owned portable source inputs, not Windows binaries/notices.
    windows = json.loads((ROOT / 'engine/manifest.json').read_text(encoding='utf-8'))
    shared_notices = {'licenses/yt-dlp-UNLICENSE.txt', 'licenses/yt-dlp-THIRD-PARTY.txt', 'licenses/GPL-3.0.txt'}
    for row in windows['files']:
        name = row['path']
        if not name.startswith(('catalog/', 'plugins/')) and name not in shared_notices:
            continue
        source = child(ROOT / 'engine', name)
        if sha(source) != row['sha256'] or source.stat().st_size != row['bytes']:
            raise ValueError('Portable input differs from its pinned source: ' + name)
        destination = child(engine, name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    for name in shared_notices:
        if not child(engine, name).is_file():
            raise ValueError('Missing common notice: ' + name)


def prepare(target: str, *, offline_dir: Path | None = None, identity='-') -> dict:
    require_mac(target)
    base = area(target)
    base.mkdir(parents=True, exist_ok=True)
    engine, ownership = base / 'engine', base / 'engine-owner.json'
    if engine.exists():
        if ownership.exists():
            clean_owned(engine, ownership)
        else:
            raise ValueError('Previous incomplete engine stage retained; inspect it before cleanup: ' + str(engine))
    # Download and validate archives before creating a new resource stage.
    archive_rows = [(row, acquire(row, offline_dir)) for row in packages(target)]
    engine.mkdir()
    try:
        upstream = []
        for row, archive in archive_rows:
            outputs = []
            for output in row['outputs']:
                destination = child(engine, output['path'])
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(read_output(archive, row, output))
                executable = output['path'] in {'yt-dlp', 'node', 'ffmpeg', 'ffprobe'} or output['path'].endswith('.dylib')
                destination.chmod(0o755 if executable else 0o644)
                outputs.append({'path': output['path'], 'original_sha256': sha(destination)})
            upstream.append({'name': row['name'], 'version': row['version'], 'url': row['url'],
                             'archive_sha256': row['sha256'], 'outputs_before_signing': outputs,
                             'build_information': row.get('build_information')})
        shared_sources(engine)
        spec = importlib.util.spec_from_file_location('flow_mac_notices', ROOT / 'scripts/collect-macos-notices.py')
        notices = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(notices)
        notice_count = notices.collect(target, engine)
        binaries = [engine / name for name in ('yt-dlp', 'node', 'ffmpeg', 'ffprobe')]
        minimum = load_lock()['minimum_macos']
        imports = audit_macho_tree(binaries, target, engine, minimum)
        # Sign only our private copies, before generating the embedded hashes.
        signed_paths = {str(path.resolve()): path for path in binaries}
        for report in imports.values():
            for dependency in report['dependencies']:
                if dependency['kind'] == 'bundled':
                    path = Path(dependency['path'])
                    signed_paths[str(path.resolve())] = path
        for path in sorted(signed_paths.values(), key=lambda p: (p.suffix != '.dylib', str(p))):
            entitlement = {'node': 'node-entitlements.plist', 'yt-dlp': 'python-entitlements.plist'}.get(path.name)
            sign(path, identity, ROOT / 'packaging/macos' / entitlement if entitlement else None)
        capabilities = capability_report(engine)
        write_json(engine / 'licenses/macos-runtime-provenance.json', {
            'schema': 1, 'target': target, 'downloads_lock_sha256': sha(ROOT / 'packaging/macos/downloads.lock.json'),
            'archives': upstream, 'signing': 'ad-hoc' if identity == '-' else 'Developer ID',
            'signed_files': sorted(p.relative_to(engine).as_posix() for p in signed_paths.values()),
            'ffmpeg_publisher': 'Martin Riedl third-party builds',
            'complete_corresponding_source_verified': False,
            'complete_licensing_review': False,
            'source_review': 'packaging/macos/README.md; Mac binaries and library revisions differ from Windows; Windows source material is not sufficient evidence for this build.'})
        (engine / 'licenses/ffmpeg-build.txt').write_text(capabilities['reported']['version'], encoding='utf-8')
        rows = file_rows(engine)
        manifest = {'schema': 1, 'platform': 'macos-' + TARGETS[target], 'target': target,
                    'cargo_lock_sha256': sha(ROOT / 'Cargo.lock'), 'versions': capabilities['versions'], 'files': rows}
        write_json(base / 'engine-manifest.json', manifest)
        (engine / 'manifest.json').write_bytes((base / 'engine-manifest.json').read_bytes())
        write_json(base / 'engine-verification.json', {'target': target, 'minimum_macos': minimum,
                    'macho': imports, 'capabilities': capabilities, 'rust_notice_packages': notice_count})
        return manifest
    finally:
        # Even on a failed capability/signature check, retain an exact ownership
        # inventory so an explicit subsequent preparation can clean only ours.
        write_json(ownership, {'owner': OWNER, 'files': file_rows(engine)})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=TARGETS, required=True)
    parser.add_argument('--list', action='store_true')
    parser.add_argument('--verify-only', action='store_true')
    parser.add_argument('--offline-dir', type=Path)
    parser.add_argument('--sign-identity', default='-')
    args = parser.parse_args()
    if args.list:
        print(json.dumps({'target': args.target, 'packages': packages(args.target)}, ensure_ascii=False, indent=2))
    elif args.verify_only:
        manifest = json.loads((area(args.target) / 'engine-manifest.json').read_text(encoding='utf-8'))
        if manifest.get('target') != args.target or manifest['cargo_lock_sha256'] != sha(ROOT / 'Cargo.lock'):
            raise ValueError('Target or Cargo lock mismatch')
        verify_engine(area(args.target) / 'engine', area(args.target) / 'engine-manifest.json')
        provenance = json.loads((area(args.target) / 'engine/licenses/macos-runtime-provenance.json').read_text(encoding='utf-8'))
        if provenance['downloads_lock_sha256'] != sha(ROOT / 'packaging/macos/downloads.lock.json'):
            raise ValueError('The prepared runtime download lock is stale')
        print('Verified private ' + args.target + ' runtime files; no app execution performed')
    else:
        manifest = prepare(args.target, offline_dir=args.offline_dir, identity=args.sign_identity)
        print(json.dumps({'target': args.target, 'files': len(manifest['files']),
                          'manifest': str(area(args.target) / 'engine-manifest.json')}, ensure_ascii=False))


if __name__ == '__main__':
    main()
