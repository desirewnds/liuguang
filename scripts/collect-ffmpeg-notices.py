# SPDX-License-Identifier: Apache-2.0
"""Preserve supplied license/notice texts from the archived FFmpeg dependencies."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import hashlib
import json
import re
import tarfile

ROOT=Path(__file__).resolve().parents[1]
SOURCE=ROOT/'engine/sources/dependencies'
DEST=ROOT/'engine/licenses/ffmpeg-dependencies'

def collect(archive):
    package=archive.name.removesuffix('.tar.gz').removesuffix('.crate')
    records=[]
    with tarfile.open(archive) as tar:
        for member in tar.getmembers():
            if not member.isfile():continue
            parts=Path(member.name).parts
            relative=parts[1:] if len(parts)>1 else parts
            if not relative:continue
            base=relative[-1]
            direct=len(relative)==1 and re.match(r'^(licen[cs]e|copying|notice|copyright|authors)([.\-_].*)?$',base,re.I)
            directory=len(relative)<=3 and relative[0].lower() in ['licenses','licences','license','legal']
            nested=re.match(r'^(licen[cs]e|copying|notice|copyright)([.\-_].*)?$',base,re.I) is not None
            if not direct and not directory and not nested:continue
            # No archive member is extracted to a path selected by the archive.
            data=tar.extractfile(member).read()
            name=re.sub(r'[^A-Za-z0-9_.-]','_',package+'-'+ '/'.join(relative))+'.txt'
            (DEST/name).write_bytes(data)
            records.append({'archive':archive.relative_to(SOURCE).as_posix(),'member':member.name,
                'notice':name,'sha256':hashlib.sha256(data).hexdigest()})
        if not records:
            # Some header-only libraries put their notice only in the first
            # comment block. Preserve the complete block verbatim, deduplicated.
            seen=set()
            for member in tar.getmembers():
                if not member.isfile() or member.size>2*1024*1024 or not member.name.endswith(('.h','.c','.hpp','.cc','.rs')):continue
                prefix=tar.extractfile(member).read(32768)
                match=re.match(rb'\s*/\*.*?\*/',prefix,re.S)
                if not match:continue
                data=match.group(0)
                if not any(word in data.lower() for word in [b'copyright',b'permission is hereby',b'redistribution and use',b'public license']):continue
                sha=hashlib.sha256(data).hexdigest()
                if sha in seen:continue
                seen.add(sha)
                name=re.sub(r'[^A-Za-z0-9_.-]','_',package)+'-header-notice-'+sha[:12]+'.txt'
                (DEST/name).write_bytes(data)
                records.append({'archive':archive.relative_to(SOURCE).as_posix(),'member':member.name,
                    'notice':name,'sha256':sha,'scope':'Complete initial source comment block'})
    return {'archive':archive.relative_to(SOURCE).as_posix(),'notices':records}

def main():
    # A consolidated materials ZIP is deliberately not expanded during normal
    # builds. Missing inputs must never replace the existing notice index with
    # an empty success report or even create an output directory.
    archives=sorted(p for p in [*SOURCE.glob('*.tar.gz'),*SOURCE.glob('cargo/*.crate')] if p.is_file()) if SOURCE.is_dir() else []
    if not archives:
        raise SystemExit('No expanded FFmpeg source archives found; restore the reviewed materials explicitly. Existing notices were left unchanged.')
    DEST.mkdir(parents=True,exist_ok=True)
    with ThreadPoolExecutor(max_workers=6) as pool:results=list(pool.map(collect,archives))
    report={'note':'Includes all archived recipe dependencies, including optional and build/test components. Original notices are preserved without assigning a guessed combined license.',
        'archives':len(results),'notice_files':sum(len(x['notices']) for x in results),'entries':results}
    (DEST.parent/'ffmpeg-dependencies.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({'archives':report['archives'],'notice_files':report['notice_files'],'without_root_notice':sum(not x['notices'] for x in results)}))

if __name__=='__main__':
    main()
