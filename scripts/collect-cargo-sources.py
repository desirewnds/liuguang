# SPDX-License-Identifier: Apache-2.0
"""Archive FFmpeg Rust media-library dependencies, verified against their lockfiles."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import hashlib
import json
import tarfile
import tomllib
import urllib.request

ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'engine/sources'

def main():
    packages={}
    for pattern in ['rav1e-*.tar.gz','librsvg-*.tar.gz']:
        archive=next((BASE/'dependencies').glob(pattern))
        with tarfile.open(archive) as tar:
            name=next(n for n in tar.getnames() if n.endswith('/Cargo.lock') and n.count('/')==1)
            lock=tomllib.loads(tar.extractfile(name).read().decode('utf-8'))
        for package in lock['package']:
            if package.get('source'):
                if not package['source'].startswith('registry+'):raise ValueError('Non-registry Cargo source needs explicit review')
                packages[package['name']+'@'+package['version']]=package
    folder=BASE/'dependencies/cargo';folder.mkdir(parents=True,exist_ok=True)
    def fetch(package):
        name=package['name']+'-'+package['version']+'.crate';target=folder/name
        try:
            if not target.exists():
                with urllib.request.urlopen('https://static.crates.io/crates/'+package['name']+'/'+name,timeout=40) as response:target.write_bytes(response.read())
            digest=hashlib.sha256(target.read_bytes()).hexdigest()
            if digest!=package['checksum']:raise ValueError('Cargo.lock checksum mismatch')
            return {'name':package['name'],'version':package['version'],'status':'verified','sha256':digest,'file':target.relative_to(BASE).as_posix()}
        except Exception as error:return {'name':package['name'],'version':package['version'],'status':'unavailable','error':str(error)}
    with ThreadPoolExecutor(max_workers=8) as pool:results=list(pool.map(fetch,packages.values()))
    (BASE/'ffmpeg-cargo-locks.json').write_text(json.dumps(list(packages.values()),indent=2)+'\n',encoding='utf-8')
    (BASE/'ffmpeg-cargo-source-report.json').write_text(json.dumps(results,indent=2)+'\n',encoding='utf-8')
    failures=[x for x in results if x['status']!='verified']
    print(json.dumps({'packages':len(results),'failed':len(failures)}))
    if failures:raise SystemExit(1)

if __name__=='__main__':main()
