# SPDX-License-Identifier: Apache-2.0
"""Verify recorded source artifacts; does not infer legal/rebuild compliance."""
from pathlib import Path
import hashlib
import json

ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'engine/sources'
rows=[]
for filename in ['ffmpeg-source-inventory.json','ffmpeg-nested-source-report.json','ffmpeg-cargo-source-report.json','application-source-index.json']:
    report=json.loads((BASE/filename).read_text(encoding='utf-8'))
    entries=report if isinstance(report,list) else report['entries']
    if isinstance(report,dict) and report.get('inspection_errors'):raise SystemExit('Unresolved nested source inspection errors')
    for item in entries:
        if item.get('status') not in [None,'archived','verified']:raise SystemExit('Unresolved source: '+json.dumps(item))
        file=(BASE/item['file']).resolve()
        if BASE.resolve() not in file.parents:raise SystemExit('Unsafe source manifest path')
        with file.open('rb') as source:digest=hashlib.file_digest(source,'sha256').hexdigest()
        if digest!=item['sha256']:raise SystemExit('Source checksum mismatch: '+item['file'])
        rows.append(item['file'])
print(json.dumps({'verified_records':len(rows),'unique_archives':len(set(rows)),'scope':'source availability and integrity only'}))
