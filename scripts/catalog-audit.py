# SPDX-License-Identifier: Apache-2.0
"""Generate the per-option capability matrix and validate catalog integrity."""
from pathlib import Path
import csv
import json

ROOT = Path(__file__).resolve().parents[1]
catalog=json.loads((ROOT/'engine/catalog/options.json').read_text(encoding='utf-8'))
rows=[]
names=set()
all_names={o['name'] for g in catalog['groups'] for o in g['options']}
ffmpeg_options={'--extract-audio','--audio-format','--audio-quality','--remux-video','--recode-video','--embed-subs','--embed-thumbnail','--embed-metadata','--embed-chapters','--embed-info-json','--split-chapters','--remove-chapters','--force-keyframes-at-cuts','--concat-playlist','--convert-subs','--convert-thumbnails','--download-sections','--postprocessor-args'}
for group in catalog['groups']:
    for option in group['options']:
        name=option['name']
        assert name not in names, f'Duplicate option: {name}'
        assert name.startswith('-') and option.get('label'), f'Invalid option: {name}'
        names.add(name)
        opposite='--'+name[5:] if name.startswith('--no-') else '--no-'+name[2:]
        dependency='FFmpeg/FFprobe' if name in ffmpeg_options else '站点/媒体支持；以原生帮助为准'
        if name in {'--downloader','--downloader-args'}:dependency='所选外部下载器可执行文件'
        if name in {'--plugin-dirs','--use-postprocessor'}:dependency='用户提供的可信插件'
        rows.append({
            'option':name,'aliases':' '.join(option['aliases']),'label':option['label'],
            'group':group['label'],'type':option['input_type'] if option['value'] else 'switch',
            'value_required':option['value'],'repeatable':option['repeatable'],
            'choices':' | '.join(str(x) for x in option.get('choices',[])),
            'route':'tools' if option.get('command_only') else 'advanced',
            'sensitive':option.get('sensitive',False),'preview_safe':option.get('preview',False),
            'hidden_compatibility':option.get('hidden',False),
            'engine_version':catalog['version'],'description_zh':option.get('description',''),
            'scope':option.get('scope',''),'default_behavior':option.get('default_behavior',''),
            'opposite_option':opposite if opposite in all_names else '',
            'dependency':dependency,
            'verification':'目录结构及唯一性自动检查；使用匹配媒体/网站/依赖执行参数并核对实际产物，详见 validation.md',
            'catalog_verified':'yes','engine_behavior':'requires scenario verification',
        })
assert len(rows)>=323, f'Catalog unexpectedly shrank: {len(rows)}'
out=ROOT/'docs/capability-matrix.csv'
out.parent.mkdir(parents=True, exist_ok=True)
with out.open('w',encoding='utf-8-sig',newline='') as f:
    writer=csv.DictWriter(f,fieldnames=list(rows[0]));writer.writeheader();writer.writerows(rows)
report={'engine_version':catalog['version'],'options':len(rows),'groups':len(catalog['groups']),
        'managed_options':sum(r['route']=='advanced' for r in rows),'tool_options':sum(r['route']=='tools' for r in rows),
        'catalog_integrity':'passed','behavior_validation':'See test results; catalog presence is not behavior verification.'}
(ROOT/'docs/catalog-audit.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(json.dumps(report,ensure_ascii=False))
