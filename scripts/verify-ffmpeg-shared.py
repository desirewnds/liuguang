# SPDX-License-Identifier: Apache-2.0
"""Verify a pinned FFmpeg shared candidate against the original pinned static build.

This tool never downloads, installs or promotes binaries. All writes and child
temporary files stay under the explicit project candidate directory.
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shlex
import shutil
import struct
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ARCHIVE_SHA = '94dd295461cada31a9e7f79ec0f9ec317d9afcbfe9e60b337679bce2c5a60016'
ARCHIVE_ROOT = 'ffmpeg-n8.1.3-35-gf1acb0a266-win64-gpl-shared-8.1'
BASELINE_SHA = {
    'ffmpeg.exe': '5bde1784598d66ff1ff3ad8cdc1ae31a342c924ac355ff6cbef188c58e95a567',
    'ffprobe.exe': 'ef8956b591d079e402e36c04274946c8e960736efee55280405aeef6e9a96def',
}
RUNTIME_NAMES = {
    'ffmpeg.exe', 'ffprobe.exe', 'avcodec-62.dll', 'avdevice-62.dll',
    'avfilter-11.dll', 'avformat-62.dll', 'avutil-60.dll',
    'swresample-6.dll', 'swscale-9.dll',
}
# Windows platform imports, not files discovered from the current machine.
WINDOWS_IMPORTS = {
    'advapi32.dll', 'avicap32.dll', 'avrt.dll', 'bcrypt.dll', 'bcryptprimitives.dll', 'cfgmgr32.dll',
    'comctl32.dll', 'comdlg32.dll', 'crypt32.dll', 'd3d11.dll', 'd3d12.dll',
    'd3d9.dll', 'd2d1.dll', 'dcomp.dll', 'dnsapi.dll', 'dwmapi.dll', 'dwrite.dll', 'dxgi.dll',
    'gdi32.dll', 'imm32.dll', 'iphlpapi.dll', 'kernel32.dll', 'ksuser.dll',
    'mf.dll', 'mfplat.dll', 'mfreadwrite.dll', 'mfuuid.dll', 'mpr.dll',
    'msimg32.dll', 'msvcrt.dll', 'ncrypt.dll', 'netapi32.dll', 'normaliz.dll',
    'ntdll.dll', 'ole32.dll', 'oleaut32.dll', 'powrprof.dll', 'propsys.dll',
    'psapi.dll', 'rpcrt4.dll', 'secur32.dll', 'setupapi.dll', 'shell32.dll',
    'shlwapi.dll', 'strmiids.dll', 'user32.dll', 'userenv.dll', 'usp10.dll',
    'version.dll', 'vfw32.dll', 'winhttp.dll', 'wininet.dll', 'winmm.dll',
    'winspool.drv', 'wintrust.dll', 'ws2_32.dll', 'wtsapi32.dll', 'ucrtbase.dll',
}
LISTS = [
    'encoders', 'decoders', 'codecs', 'formats', 'demuxers', 'muxers',
    'filters', 'protocols', 'bsfs', 'hwaccels', 'devices', 'pix_fmts',
    'sample_fmts', 'layouts', 'dispositions', 'colors',
]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def select_baseline(explicit, preserved, engine):
    """Select only the original, SHA-pinned static executables for comparison."""
    if explicit is not None:
        source = explicit.resolve()
    elif any((preserved / name).exists() for name in BASELINE_SHA):
        source = preserved.resolve()
    else:
        source = engine.resolve()
    failures = [name for name, expected in BASELINE_SHA.items()
                if not (source / name).is_file() or digest(source / name) != expected]
    if failures:
        raise SystemExit(
            'Original static baseline missing or SHA-256 mismatch in ' + str(source)
            + ': ' + ', '.join(failures) + '. Supply --baseline-dir DIR containing '
            'the original ffmpeg.exe and ffprobe.exe from the bin/ directory of '
            'engine/sources/btbn-ffmpeg-n8.1.3-35-win64-gpl.zip. '
            'The installed shared build cannot serve as the static baseline.'
        )
    return source


def pe_imports(path):
    data = path.read_bytes()
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if data[:2] != b'MZ' or data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('Not a PE file: ' + str(path))
    machine, sections = struct.unpack_from('<HH', data, pe + 4)
    if machine != 0x8664:
        raise ValueError('Expected x86-64 PE: ' + str(path))
    optional_size = struct.unpack_from('<H', data, pe + 20)[0]
    optional = pe + 24
    if struct.unpack_from('<H', data, optional)[0] != 0x20b:
        raise ValueError('Expected PE32+')
    image_base = struct.unpack_from('<Q', data, optional + 24)[0]
    section_rows = []
    for index in range(sections):
        offset = optional + optional_size + 40 * index
        virtual_size, rva, size, raw = struct.unpack_from('<4I', data, offset + 8)
        section_rows.append((rva, max(virtual_size, size), raw))

    def file_offset(rva):
        for start, length, raw in section_rows:
            if start <= rva < start + length:
                return raw + rva - start
        raise ValueError('Unmapped PE RVA: ' + hex(rva))

    def name_at(rva):
        offset = file_offset(rva)
        return data[offset:data.index(b'\0', offset)].decode('ascii').lower()

    regular, delayed = [], []
    import_rva = struct.unpack_from('<I', data, optional + 112 + 8)[0]
    if import_rva:
        cursor = file_offset(import_rva)
        while any(data[cursor:cursor + 20]):
            regular.append(name_at(struct.unpack_from('<I', data, cursor + 12)[0]))
            cursor += 20
    delay_rva = struct.unpack_from('<I', data, optional + 112 + 13 * 8)[0]
    if delay_rva:
        cursor = file_offset(delay_rva)
        while any(data[cursor:cursor + 32]):
            attributes, name = struct.unpack_from('<II', data, cursor)
            delayed.append(name_at(name if attributes & 1 else name - image_base))
            cursor += 32
    return {'imports': sorted(set(regular)), 'delay_imports': sorted(set(delayed))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', required=True, type=Path)
    parser.add_argument('--baseline-dir', type=Path,
                        help='Directory containing both original SHA-pinned static EXEs; defaults to an existing verified work/static-baseline, then the engine only if still static')
    parser.add_argument('--work', type=Path, default=ROOT / 'test-artifacts/dependency-candidates/ffmpeg-shared')
    args = parser.parse_args()
    work = args.work.resolve()
    if not work.is_relative_to((ROOT / 'test-artifacts').resolve()):
        raise SystemExit('Work directory must stay in project test-artifacts')
    if digest(args.archive) != ARCHIVE_SHA:
        raise SystemExit('Candidate archive SHA-256 mismatch')
    candidate = work / 'shared'
    baseline = work / 'static-baseline'
    temporary = work / 'temp'
    baseline_source = select_baseline(args.baseline_dir, baseline, ROOT / 'engine')
    for directory in [candidate, baseline, temporary]:
        directory.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.archive) as archive:
        bad_member = archive.testzip()
        if bad_member:
            raise SystemExit('Invalid ZIP CRC: ' + bad_member)
        actual_bin = {Path(name).name for name in archive.namelist()
                      if name.startswith(ARCHIVE_ROOT + '/bin/') and not name.endswith('/')}
        if actual_bin != RUNTIME_NAMES | {'ffplay.exe'}:
            raise SystemExit('Unexpected upstream bin inventory: ' + repr(actual_bin))
        for name in sorted(RUNTIME_NAMES):
            (candidate / name).write_bytes(archive.read(ARCHIVE_ROOT + '/bin/' + name))
    for name in BASELINE_SHA:
        if (baseline_source / name).resolve() != (baseline / name).resolve():
            shutil.copy2(baseline_source / name, baseline / name)
    env = os.environ.copy()
    for variable in ['TEMP', 'TMP', 'TMPDIR', 'XDG_CACHE_HOME']:
        env[variable] = str(temporary)
    # Prevent accidental satisfaction of missing non-system DLLs through PATH.
    env['PATH'] = str(Path(os.environ['SystemRoot']) / 'System32')
    env.pop('FFREPORT', None)

    def run(folder, executable, arguments, output_name):
        result = subprocess.run([str(folder / executable), *arguments], cwd=work,
                                env=env, capture_output=True, timeout=90)
        (work / (output_name + '.stdout')).write_bytes(result.stdout)
        (work / (output_name + '.stderr')).write_bytes(result.stderr)
        if result.returncode:
            raise RuntimeError(f'{output_name}: exit {result.returncode}; see stderr artifact')
        return result.stdout

    report = {
        'archive': str(args.archive.resolve()), 'archive_sha256': ARCHIVE_SHA,
        'status': 'running', 'capabilities': {}, 'pe': {},
        'limits': ['No installer or GUI executed.',
                   'Hardware acceleration enumeration is compared; GPU hardware execution is not tested.',
                   'PE imports cover regular/delay imports, not optional DLLs loaded by name at runtime.'],
    }
    try:
        system_imports = set()
        for name in sorted(RUNTIME_NAMES):
            info = pe_imports(candidate / name)
            all_imports = set(info['imports'] + info['delay_imports'])
            info['private'] = sorted(all_imports & RUNTIME_NAMES)
            info['system'] = sorted(dep for dep in all_imports if dep in WINDOWS_IMPORTS
                                    or dep.startswith(('api-ms-win-', 'ext-ms-win-')))
            info['unresolved'] = sorted(all_imports - set(info['private']) - set(info['system']))
            report['pe'][name] = info
            system_imports.update(info['system'])
            if info['unresolved']:
                raise RuntimeError(f'Unresolved imports for {name}: {info["unresolved"]}')
        report['system_imports'] = sorted(system_imports)
        for executable in ['ffmpeg.exe', 'ffprobe.exe']:
            version_outputs = []
            for folder, label in [(baseline, 'static'), (candidate, 'shared')]:
                version_outputs.append(run(folder, executable, ['-version'],
                                           label + '-' + executable + '-version').decode('utf-8'))
            version_lines = [value.splitlines() for value in version_outputs]
            configs = [set(shlex.split(next(line for line in lines if line.startswith('configuration:'))
                                      .removeprefix('configuration:')))
                       for lines in version_lines]
            added, removed = configs[1] - configs[0], configs[0] - configs[1]
            report[executable + '-configure-difference'] = {'added': sorted(added), 'removed': sorted(removed)}
            if added != {'--enable-shared', '--disable-static'} or removed:
                raise RuntimeError('Unexpected configure difference: ' + executable)
            if [line for line in version_lines[0] if not line.startswith('configuration:')] != [
                    line for line in version_lines[1] if not line.startswith('configuration:')]:
                raise RuntimeError('Version, compiler or library ABI differs: ' + executable)
        for option in LISTS:
            values = [run(folder, 'ffmpeg.exe', ['-hide_banner', '-' + option], label + '-' + option)
                      for folder, label in [(baseline, 'static'), (candidate, 'shared')]]
            same = values[0].replace(b'\r\n', b'\n') == values[1].replace(b'\r\n', b'\n')
            report['capabilities'][option] = {'identical': same, 'bytes': len(values[1]),
                                            'sha256': hashlib.sha256(values[1]).hexdigest()}
            if not same:
                raise RuntimeError('Capability list differs: ' + option)
        for executable in ['ffmpeg.exe', 'ffprobe.exe']:
            values = [run(folder, executable, ['-hide_banner', '-h', 'full'], label + '-' + executable + '-help')
                      for folder, label in [(baseline, 'static'), (candidate, 'shared')]]
            same = values[0].replace(b'\r\n', b'\n') == values[1].replace(b'\r\n', b'\n')
            report['capabilities'][executable + '-full-help'] = {'identical': same, 'bytes': len(values[1])}
            if not same:
                raise RuntimeError('CLI options differ: ' + executable)
        for folder, label in [(baseline, 'static'), (candidate, 'shared')]:
            output = work / (label + '.mp4')
            run(folder, 'ffmpeg.exe', ['-hide_banner', '-nostdin', '-y', '-f', 'lavfi', '-i',
                'testsrc2=size=160x90:rate=10:duration=1', '-f', 'lavfi', '-i',
                'sine=frequency=1000:sample_rate=48000:duration=1', '-c:v', 'libx264',
                '-pix_fmt', 'yuv420p', '-threads', '1', '-c:a', 'aac', '-shortest', str(output)], label + '-encode')
            run(folder, 'ffprobe.exe', ['-v', 'error', '-show_streams', '-show_format', '-of', 'json', str(output)], label + '-probe')
            run(folder, 'ffmpeg.exe', ['-v', 'error', '-nostdin', '-y', '-i', str(output), '-map', '0',
                '-c', 'copy', str(work / (label + '.mkv'))], label + '-remux')
            run(folder, 'ffmpeg.exe', ['-v', 'error', '-nostdin', '-i', str(output), '-map', '0',
                '-f', 'framemd5', '-'], label + '-decode')
        report['decoded_frames_identical'] = (work / 'static-decode.stdout').read_bytes() == (work / 'shared-decode.stdout').read_bytes()
        if not report['decoded_frames_identical']:
            raise RuntimeError('Decoded media differs')
        report['files'] = [{'path': name, 'bytes': (candidate / name).stat().st_size,
                            'sha256': digest(candidate / name)} for name in sorted(RUNTIME_NAMES)]
        report['before_bytes'] = sum((baseline / name).stat().st_size for name in ['ffmpeg.exe', 'ffprobe.exe'])
        report['after_bytes'] = sum(item['bytes'] for item in report['files'])
        report['saved_bytes'] = report['before_bytes'] - report['after_bytes']
        report['status'] = 'passed'
    except Exception as error:
        report['status'] = 'failed'
        report['error'] = str(error)
        raise
    finally:
        (work / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'status': report['status'], 'files': len(report['files']),
                      'saved_bytes': report['saved_bytes'], 'report': str(work / 'report.json')}))


if __name__ == '__main__':
    main()
