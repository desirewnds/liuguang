# SPDX-License-Identifier: Apache-2.0
"""Archive the exact public SVN revisions used by the pinned FFmpeg recipe."""
from concurrent.futures import ThreadPoolExecutor
from html.parser import HTMLParser
from io import BytesIO
from pathlib import Path
import base64
import hashlib
import json
import tarfile
import urllib.parse
import urllib.request
import time

ROOT=Path(__file__).resolve().parents[1]
TARGET=ROOT/'engine/sources/dependencies'
SOURCES=[
    ('xvidcore','2204','https://svn.xvid.org/!svn/bc/2204/trunk/xvidcore/',True),
    ('lame','7113','https://svn.code.sf.net/p/lame/svn/!svn/bc/7113/trunk/lame/',False),
]
class Links(HTMLParser):
    def __init__(self): super().__init__();self.paths=[]
    def handle_starttag(self,tag,attrs):
        if tag=='a':
            link=dict(attrs).get('href','')
            if link and not link.startswith(('../','/','#')) and ':' not in link:self.paths.append(link)

def archive(source):
    name,revision,base,anonymous=source
    headers={'User-Agent':'Liuguang-source-archive/1.0'}
    if anonymous: headers['Authorization']='Basic '+base64.b64encode(b'anonymous:').decode('ascii')
    def get(path):
        url=urllib.parse.urljoin(base,path)
        if not url.startswith(base):raise ValueError('SVN path escaped selected revision')
        for attempt in range(5):
            try:
                with urllib.request.urlopen(urllib.request.Request(url,headers=headers),timeout=40) as r:return path,r.read()
            except Exception:
                if attempt==4:raise
                time.sleep(min(2**attempt,8))
    pending=[''];files={}
    with ThreadPoolExecutor(max_workers=8) as pool:
        while pending:
            next_paths=[]
            for path,data in pool.map(get,pending):
                if path=='' or path.endswith('/'):
                    parser=Links();parser.feed(data.decode('utf-8'))
                    next_paths.extend(path+urllib.parse.unquote(link) for link in parser.paths)
                else:files[path]=data
            pending=next_paths
    target=TARGET/f'{name}-r{revision}.tar.gz'
    with tarfile.open(target,'w:gz') as tar:
        for path,data in sorted(files.items()):
            entry=tarfile.TarInfo(name+'-'+revision+'/'+path);entry.size=len(data);entry.mode=0o644
            tar.addfile(entry,BytesIO(data))
    return {'name':name,'revision':revision,'url':base,'file':target.name,'files':len(files),
        'sha256':hashlib.sha256(target.read_bytes()).hexdigest(),'bytes':target.stat().st_size}

if __name__=='__main__':
    TARGET.mkdir(parents=True,exist_ok=True)
    results=[]
    for source in SOURCES:
        try:result=archive(source)
        except Exception as error:result={'name':source[0],'error':str(error)}
        results.append(result);print(json.dumps(result),flush=True)
    (TARGET.parent/'svn-sources.json').write_text(json.dumps(results,indent=2)+'\n',encoding='utf-8')
