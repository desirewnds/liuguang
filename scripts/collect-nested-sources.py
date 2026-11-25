# SPDX-License-Identifier: Apache-2.0
"""Archive fixed git submodules and shaderc DEPS without executing upstream code."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import configparser
import hashlib
import importlib.util
import json
import re
import subprocess
import tarfile
import urllib.parse
import urllib.request

ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'engine/sources'
spec=importlib.util.spec_from_file_location('source_collector',ROOT/'scripts/collect-ffmpeg-sources.py')
collector=importlib.util.module_from_spec(spec);spec.loader.exec_module(collector)

def submodules(item):
    archive=BASE/item['file']
    with tarfile.open(archive) as tar:
        names=tar.getnames()
        module=next((n for n in names if n.endswith('/.gitmodules') and n.count('/')==1),None)
        if not module:return []
        text=tar.extractfile(module).read().decode('utf-8')
    config=configparser.ConfigParser();config.read_string(text)
    if not config.sections():return []
    repo=item['repository'].removesuffix('.git');revision=item['revision']
    parsed=urllib.parse.urlparse(repo)
    if parsed.hostname=='github.com':
        cache=BASE/'submodule-metadata'/(hashlib.sha256((repo+'@'+revision).encode()).hexdigest()+'.json')
        if cache.is_file():tree=json.loads(cache.read_text(encoding='utf-8'))
        else:
            request=urllib.request.Request('https://api.github.com/repos'+parsed.path+'/git/trees/'+revision+'?recursive=1',headers={'User-Agent':'Liuguang-source-archive/1.0'})
            with urllib.request.urlopen(request,timeout=40) as response:tree=json.load(response)
            cache.parent.mkdir(parents=True,exist_ok=True);cache.write_text(json.dumps(tree),encoding='utf-8')
        if tree.get('truncated'):raise ValueError('Git tree truncated: '+repo)
        links={x['path']:x['sha'] for x in tree['tree'] if x.get('mode')=='160000'}
    else:
        local=BASE/'repos'/('nested-'+hashlib.sha256(repo.encode()).hexdigest()[:16])
        local.mkdir(parents=True,exist_ok=True)
        subprocess.run(['git','init','--bare',str(local)],capture_output=True,check=True,timeout=20)
        subprocess.run(['git','-C',str(local),'-c','credential.helper=','fetch','--filter=blob:none','--depth=1',repo,revision],capture_output=True,check=True,timeout=100)
        result=subprocess.run(['git','-C',str(local),'ls-tree','-r',revision],capture_output=True,encoding='utf-8',check=True,timeout=30)
        links={m[1]:m[0] for m in re.findall(r'^160000 commit ([0-9a-f]+)\t(.+)$',result.stdout,re.M)}
    result=[]
    for section in config.sections():
        path=config[section]['path'];url=config[section]['url']
        if url.startswith('../'):url=urllib.parse.urljoin(repo+'/',url)
        sha=links.get(path)
        if not sha:continue
        result.append({'recipe':repo+'::'+path,'repository':url,'revision':sha,'archive_url':collector.archive_url(url,sha),
            'status':'pending','nested_dependencies':True})
    return result

def main():
    inventory=json.loads((BASE/'ffmpeg-source-inventory.json').read_text(encoding='utf-8'))['entries']
    pending={x['repository']+'@'+x['revision']:x for x in inventory if x.get('file')}
    seen=set(pending);nested=[];errors=[]
    for depth in range(4):
        discovered=[]
        def inspect(item):
            try:return submodules(item)
            except Exception as error:
                errors.append({'repository':item['repository'],'revision':item['revision'],'error':str(error)});return []
        with ThreadPoolExecutor(max_workers=6) as pool:
            for children in pool.map(inspect,pending.values()):discovered.extend(children)
        to_fetch={}
        for item in discovered:
            key=item['repository']+'@'+item['revision']
            if key not in seen:seen.add(key);to_fetch[key]=item
        if not to_fetch:break
        with ThreadPoolExecutor(max_workers=6) as pool:results=list(pool.map(collector.fetch,to_fetch.values()))
        nested.extend(results)
        pending={x['repository']+'@'+x['revision']:x for x in results if x.get('file')}
        print('Submodule layer '+str(depth+1)+': '+str(len(results)),flush=True)
    # Shaderc uses its own DEPS file rather than git submodules. Parse literal
    # assignments and Var references only; never eval/exec the upstream file.
    archive=next((BASE/'dependencies').glob('shaderc-*.tar.gz'))
    with tarfile.open(archive) as tar:
        name=next(n for n in tar.getnames() if n.endswith('/DEPS') and n.count('/')==1)
        source=tar.extractfile(name).read().decode()
    variables=dict(re.findall(r"'([a-z0-9_]+)'\s*:\s*'([^']+)'",source.split('deps =')[0]))
    entries=[]
    for path,owner,suffix,version in re.findall(r"'([^']+)'\s*:\s*Var\('([^']+)'\)\s*\+\s*'([^']+)'\s*\+\s*Var\('([^']+)'\)",source.split('deps =')[1]):
        repo=(variables[owner]+suffix).rstrip('@');revision=variables[version]
        entries.append({'recipe':'shaderc::'+path,'repository':repo,'revision':revision,'archive_url':collector.archive_url(repo,revision),'status':'pending','nested_dependencies':True})
    with ThreadPoolExecutor(max_workers=6) as pool:nested.extend(pool.map(collector.fetch,entries))
    report={'entries':nested,'inspection_errors':errors,'complete_corresponding_source_verified':False}
    (BASE/'ffmpeg-nested-source-report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({'nested_archives':len(nested),'inspection_errors':len(errors)}))

if __name__=='__main__':main()
