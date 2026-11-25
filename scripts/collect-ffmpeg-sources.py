# SPDX-License-Identifier: Apache-2.0
"""Archive upstream source pinned by the selected FFmpeg build recipe.

Downloads source only; never executes a downloaded build script. The report
distinguishes downloaded source from unresolved archives and nested dependencies.
It deliberately does not declare the result a complete corresponding-source offer.
"""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import argparse
import hashlib
import json
import gzip
import importlib.util
import re
import subprocess
import tarfile
import urllib.request
import urllib.parse
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SOURCES = ROOT / 'engine/sources'
RECIPE = SOURCES / 'btbn-build-recipe.zip'
RECIPE_SHA = '28671ca872b4ed1d03ec107cba4064f3609e1bd3d5a1b9f2397026c0d3564b8c'

def archive_url(repo, revision):
    repo = repo.removesuffix('.git').rstrip('/')
    parsed = urllib.parse.urlparse(repo)
    if parsed.hostname == 'github.com':
        return repo + '/archive/' + urllib.parse.quote(revision, safe='') + '.tar.gz'
    if parsed.hostname in ['gitlab.freedesktop.org', 'gitlab.com', 'code.videolan.org']:
        return repo + '/-/archive/' + revision + '/' + repo.rsplit('/',1)[-1] + '-' + revision + '.tar.gz'
    if parsed.hostname in ['aomedia.googlesource.com', 'chromium.googlesource.com']:
        return repo + '/+archive/' + revision + '.tar.gz'
    return None

def inventory():
    assert hashlib.sha256(RECIPE.read_bytes()).hexdigest() == RECIPE_SHA, 'Recipe archive checksum mismatch'
    entries = []
    with zipfile.ZipFile(RECIPE) as z:
        for name in sorted(z.namelist()):
            if '/scripts.d/' not in name or not name.endswith('.sh'): continue
            source = z.read(name).decode('utf-8')
            variables = dict(re.findall(r'^((?:SCRIPT_REPO|SCRIPT_COMMIT|SCRIPT_REV)\d*)="([^"\n]+)"', source, re.M))
            script = name.split('/scripts.d/')[1]
            for key, repo in variables.items():
                if not key.startswith('SCRIPT_REPO'): continue
                suffix = key.removeprefix('SCRIPT_REPO')
                revision = variables.get('SCRIPT_COMMIT'+suffix) or variables.get('SCRIPT_REV'+suffix)
                if not revision: continue
                entries.append({'recipe':script,'repository':repo,'revision':revision,
                    'archive_url':archive_url(repo,revision), 'status':'pending',
                    'nested_dependencies': any(token in source for token in ['cargo vendor','cargo update','git-sync-deps','submodule']),
                    'note':'Recipe inventory includes optional/non-Windows packages; not all are linked in this build.'})
    entries.append({'recipe':'FFmpeg','repository':'https://github.com/FFmpeg/FFmpeg.git',
                    'revision':'f1acb0a266', 'archive_url':'https://github.com/FFmpeg/FFmpeg/archive/f1acb0a266.tar.gz',
                    'status':'pending','nested_dependencies':False})
    return entries

def fetch(item):
    if not item['archive_url']:
        repo=item['repository'];revision=item['revision']
        names={'https://git.code.sf.net/p/mingw-w64/mingw-w64.git':'mingw-w64',
            'https://git.savannah.gnu.org/git/libiconv.git':'libiconv','https://git.savannah.gnu.org/git/gnulib.git':'gnulib',
            'https://git.code.sf.net/p/opencore-amr/code':'opencore-amr','https://git.code.sf.net/p/soxr/code':'soxr'}
        try:
            if repo in names:
                name=names[repo];target=SOURCES/'dependencies'/(name+'-'+revision+'.tar.gz')
                target.parent.mkdir(parents=True,exist_ok=True)
                if not target.exists():
                    local=SOURCES/'repos'/name;local.mkdir(parents=True,exist_ok=True)
                    subprocess.run(['git','init','--bare',str(local)],capture_output=True,check=True,timeout=20)
                    actual_repo='https://github.com/coreutils/gnulib.git' if name=='gnulib' else repo
                    subprocess.run(['git','-C',str(local),'-c','credential.helper=','fetch','--depth=1',actual_repo,revision],capture_output=True,check=True,timeout=120)
                    result=subprocess.run(['git','-C',str(local),'archive','--format=tar',revision],capture_output=True,check=True,timeout=60)
                    with gzip.open(target,'wb') as f:f.write(result.stdout)
            elif repo.startswith(('https://svn.xvid.org/','https://svn.code.sf.net/p/lame/')):
                spec=importlib.util.spec_from_file_location('svn_sources',ROOT/'scripts/collect-svn-sources.py')
                module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
                source=module.SOURCES[0 if 'xvid.org' in repo else 1]
                target=SOURCES/'dependencies'/f'{source[0]}-r{revision}.tar.gz'
                if not target.exists():module.archive(source)
            else:return {**item,'status':'requires-vcs-checkout'}
            if not tarfile.is_tarfile(target):raise ValueError('Invalid source archive')
            with target.open('rb') as f:sha=hashlib.file_digest(f,'sha256').hexdigest()
            return {**item,'status':'archived','file':target.relative_to(SOURCES).as_posix(),'bytes':target.stat().st_size,'sha256':sha}
        except Exception as error:return {**item,'status':'unavailable','error':str(error)}
    identity = item['repository']+'@'+item['revision']
    name = re.sub(r'[^a-zA-Z0-9_.-]','_',item['repository'].rstrip('/').rsplit('/',1)[-1].removesuffix('.git'))+'-'+item['revision']+'.tar.gz'
    target = SOURCES/'dependencies'/name
    target.parent.mkdir(parents=True,exist_ok=True)
    try:
        if not target.exists():
            req=urllib.request.Request(item['archive_url'],headers={'User-Agent':'Liuguang-source-archive/1.0'})
            with urllib.request.urlopen(req,timeout=35) as response, target.with_suffix('.partial').open('wb') as output:
                for block in iter(lambda:response.read(1024*1024),b''): output.write(block)
            target.with_suffix('.partial').replace(target)
        if not tarfile.is_tarfile(target):
            raise ValueError('Response is not a valid source archive')
        result={**item,'status':'archived','file':target.relative_to(SOURCES).as_posix(),
                'bytes':target.stat().st_size,'sha256':hashlib.file_digest(target.open('rb'),'sha256').hexdigest()}
        print('Archived '+identity,flush=True)
        return result
    except Exception as error:
        print('Unavailable '+identity+': '+str(error),flush=True)
        return {**item,'status':'unavailable','error':str(error)}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fetch',action='store_true',help='Explicitly download source archives from the pinned upstream repositories')
    parser.add_argument('--jobs',type=int,default=6)
    args=parser.parse_args()
    entries=inventory()
    # Identical source may be used by multiple recipe stages; download once.
    unique={x['repository']+'@'+x['revision']:x for x in entries}
    if args.fetch:
        with ThreadPoolExecutor(max_workers=max(1,min(args.jobs,8))) as pool:
            downloaded=list(pool.map(fetch,unique.values()))
        result={x['repository']+'@'+x['revision']:x for x in downloaded}
        entries=[{**result[x['repository']+'@'+x['revision']],'recipe':x['recipe']} for x in entries]
    report={'build_recipe_commit':'c9fe5114060e507409f7ee07edfd82efaf25d767',
        'release':'autobuild-2026-11-14-12-37','binary_sha256':'b713e521551d642cc473470772b1c28c971434097e85d7e1ace0ab5ade362fcc',
        'complete_corresponding_source_verified':False,
        'limitations':['Nested dependencies (git submodules, Cargo vendor and shaderc DEPS) require separate verification.',
            'The upstream rav1e recipe updates the cc crate during build; its final resolved Cargo.lock is not included in the binary release.',
            'Source-archive availability is separate from byte-for-byte reproducible builds.'],
        'entries':entries}
    (SOURCES/'ffmpeg-source-inventory.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({s:sum(x['status']==s for x in entries) for s in sorted({x['status'] for x in entries})}))

if __name__=='__main__':main()
