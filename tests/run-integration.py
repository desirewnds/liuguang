# SPDX-License-Identifier: Apache-2.0
"""Short, sequential real-engine verification with project-owned temporary data."""
from argparse import ArgumentParser
from functools import partial
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='Use an already built flow-integration executable; avoids another build')
    parser.add_argument('--engine-root', type=Path, default=ROOT/'engine')
    parser.add_argument('--keep-artifacts', action='store_true')
    args = parser.parse_args()
    engine = args.engine_root.resolve(strict=True)
    artifacts = ROOT/'test-artifacts/integration'
    artifacts.mkdir(parents=True, exist_ok=True)
    if not artifacts.resolve().is_relative_to(ROOT.resolve()):
        raise SystemExit('Test artifact directory points outside the project')
    work = Path(tempfile.mkdtemp(prefix='run-', dir=artifacts)).resolve()
    fixture = work/'fixture'
    fixture.mkdir()
    temporary = work/'temporary'
    temporary.mkdir()
    flags = (subprocess.BELOW_NORMAL_PRIORITY_CLASS | subprocess.CREATE_NO_WINDOW) if os.name == 'nt' else 0
    env = dict(os.environ, TEMP=str(temporary), TMP=str(temporary), TMPDIR=str(temporary),
               CARGO_HOME=str(ROOT/'.build/cargo'), CARGO_TARGET_DIR=str(ROOT/'target'),
               CARGO_BUILD_JOBS='1', RAYON_NUM_THREADS='1', PYTHONDONTWRITEBYTECODE='1')
    server = None
    try:
        subprocess.run([str(engine/'ffmpeg.exe'), '-hide_banner', '-loglevel', 'error', '-y',
            '-filter_threads', '1', '-filter_complex_threads', '1',
            '-f', 'lavfi', '-i', 'testsrc2=size=320x180:rate=24', '-f', 'lavfi', '-i', 'sine=frequency=440',
            '-t', '2', '-c:v', 'libx264', '-threads:v', '1', '-c:a', 'aac', '-threads:a', '1',
            '-movflags', '+faststart', str(fixture/'sample.mp4')],
            check=True, timeout=30, env=env, creationflags=flags)
        subprocess.run([str(engine/'ffmpeg.exe'), '-hide_banner', '-loglevel', 'error', '-y',
            '-i', str(fixture/'sample.mp4'), '-map', '0:v', '-map', '0:a', '-c', 'copy',
            '-f', 'dash', '-seg_duration', '1', 'stream.mpd'],
            cwd=fixture, check=True, timeout=30, env=env, creationflags=flags)
        (fixture/'playlist.html').write_text(
            '<title>Fixture collection</title><video controls src="sample.mp4"></video><video controls src="stream.mpd"></video>',
            encoding='utf-8')
        for language, text in [('en', 'FLOW_SUBTITLE_ENGLISH'), ('zh-CN', '流光字幕校验'), ('fr', 'FLOW_FRENCH_NOT_SELECTED')]:
            (fixture/f'sample.{language}.vtt').write_text(
                f'WEBVTT\n\n00:00:00.000 --> 00:00:01.500\n{text}\n', encoding='utf-8')
        (fixture/'subtitles.html').write_text(
            '<!doctype html><meta charset="utf-8"><title>Fixture subtitles</title>'
            '<video controls><source src="sample.mp4" type="video/mp4">'
            '<track kind="subtitles" srclang="en" src="sample.en.vtt">'
            '<track kind="subtitles" srclang="zh-CN" src="sample.zh-CN.vtt">'
            '<track kind="subtitles" srclang="fr" src="sample.fr.vtt"></video>', encoding='utf-8')

        class Handler(SimpleHTTPRequestHandler):
            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(('127.0.0.1', 0), partial(Handler, directory=str(fixture)))
        threading.Thread(target=server.serve_forever, daemon=True).start()
        inputs = [str(ROOT), f'http://127.0.0.1:{server.server_port}', str(engine), str(work)]
        if args.binary:
            binary = args.binary.resolve(strict=True)
            command = [str(binary), *inputs]
        else:
            command = ['cargo', 'run', '--release', '--locked', '--offline', '-j', '1',
                       '-p', 'flow-integration', '--', *inputs]
        result = subprocess.run(command, cwd=ROOT, env=env, creationflags=flags, timeout=600,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                text=True, encoding='utf-8', errors='replace')
        print(result.stdout, end='')
        result.check_returncode()
    finally:
        if server:
            server.shutdown()
            server.server_close()
        if args.keep_artifacts:
            print(f'Artifacts kept: {work}')
        else:
            resolved = work.resolve()
            if work.is_symlink() or not resolved.is_relative_to(artifacts.resolve()) or resolved == artifacts.resolve():
                raise RuntimeError('Refusing cleanup outside the generated test run')
            shutil.rmtree(work)


if __name__ == '__main__':
    main()
