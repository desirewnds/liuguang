# SPDX-License-Identifier: Apache-2.0
"""Optional, read-only real-site checks. No cookies, proxy discovery or video download."""
from pathlib import Path
import json, subprocess, time

root = Path(__file__).resolve().parents[1]
engine = root/'engine'
base = [str(engine/'yt-dlp.exe'), '--ignore-config', '--proxy', '', '--socket-timeout', '12',
        '--retries', '0', '--extractor-retries', '0', '--skip-download', '--dump-single-json',
        '--encoding', 'utf-8', '--js-runtimes', f'node:{engine / "node.exe"}',
        '--ffmpeg-location', str(engine), '--plugin-dirs', str(engine/'plugins')]
cases = [
    ('bilibili-playlist', ['--flat-playlist','--yes-playlist'], 'https://www.bilibili.com/video/BV1JV411t7ow?t=5.0'),
    ('bilibili-current-p7', ['--no-playlist'], 'https://www.bilibili.com/video/BV1JV411t7ow?t=5.0&p=7'),
    ('youtube-current', ['--no-playlist'], 'https://youtu.be/D_0Veya2XAY?list=PLNcg_FV9n7qZY_2eAtUzEUulNjTJREhQe'),
]
report=[]
for name, flags, url in cases:
    start=time.monotonic()
    try:
        result=subprocess.run(base+flags+['--',url],capture_output=True,encoding='utf-8',errors='replace',timeout=90)
        row={'case':name,'elapsed_seconds':round(time.monotonic()-start,2),'exit_code':result.returncode,'connection':'direct, no cookies'}
        if result.returncode==0:
            info=json.loads(result.stdout)
            entries=info.get('entries',[])
            row.update({'id':info.get('id'),'title':info.get('title'),'entry_count':len(entries),
                'p7': [{'index':e.get('playlist_index'),'title':e.get('title'),'url':e.get('url')} for e in entries if e.get('playlist_index')==7],
                'heights':sorted({f['height'] for f in info.get('formats',[]) if f.get('height')})})
        else: row['error']=result.stderr[-2500:]
    except subprocess.TimeoutExpired:
        row={'case':name,'error':'Timed out after 90 seconds','connection':'direct, no cookies'}
    report.append(row)
    print(json.dumps(row,ensure_ascii=False),flush=True)
out=root/'test-artifacts/site-probe.json';out.parent.mkdir(parents=True,exist_ok=True)
out.write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
