# SPDX-License-Identifier: Apache-2.0
"""Build one native Mac architecture using project-local caches and one job.

No global tool installation, GUI launch, application installation or publication.
The host must already have Rust, Python 3.11+ and Xcode Command Line Tools.
"""
from pathlib import Path
import argparse
import os
import platform
import sys
from macos_common import ROOT, TARGETS, area, load_lock, require_mac, run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True, choices=TARGETS)
    parser.add_argument('--offline', action='store_true')
    parser.add_argument('--offline-dir', type=Path, help='Exact pinned upstream runtime archives')
    parser.add_argument('--no-package', action='store_true')
    parser.add_argument('--sign-identity', default='-')
    parser.add_argument('--notarize-profile')
    args = parser.parse_args()
    require_mac(args.target)
    try:
        os.nice(5)
    except OSError:
        pass
    base = area(args.target)
    temporary = base / 'tmp'
    temporary.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, 'CARGO_HOME': str(ROOT / '.build/cargo'),
           'CARGO_TARGET_DIR': str(ROOT / 'target'), 'CARGO_BUILD_JOBS': '1',
           'CMAKE_BUILD_PARALLEL_LEVEL': '1', 'RAYON_NUM_THREADS': '1', 'RUST_TEST_THREADS': '1',
           'PYTHONDONTWRITEBYTECODE': '1', 'TMPDIR': str(temporary), 'TMP': str(temporary), 'TEMP': str(temporary),
           'MACOSX_DEPLOYMENT_TARGET': load_lock()['minimum_macos'],
           'FLOW_MACOS_RESOURCE_MANIFEST': str(base / 'engine-manifest.json')}
    # Probe existing tools only; a missing tool is an error, never auto-installed.
    for command in (['cargo', '--version'], ['rustc', '-vV'], ['/usr/bin/xcrun', '--find', 'clang'],
                    ['/usr/bin/xcrun', 'clang', '--version']):
        print(run(command, env=env).strip(), flush=True)
    fetch = ['cargo', 'fetch', '--locked', '--target', args.target]
    if args.offline:
        fetch.append('--offline')
    run(fetch, env=env, capture=False)
    prepare = [sys.executable, '-B', ROOT / 'scripts/prepare-macos-engines.py', '--target', args.target,
               '--sign-identity', args.sign_identity]
    if args.offline_dir:
        prepare += ['--offline-dir', args.offline_dir.resolve()]
    elif args.offline:
        # Offline mode must never acquire binaries implicitly over the network.
        prepare += ['--offline-dir', ROOT / '.build/macos/downloads']
    run(prepare, env=env, capture=False)
    run(['cargo', 'build', '--release', '--locked', '--offline', '--jobs', '1', '--target', args.target,
         '--bin', 'flow-desktop', '--bin', 'flow-cli'], env=env, capture=False)
    if not args.no_package:
        command = [sys.executable, '-B', ROOT / 'scripts/package-macos.py', '--target', args.target,
                   '--sign-identity', args.sign_identity]
        if args.notarize_profile:
            command += ['--notarize-profile', args.notarize_profile]
        run(command, env=env, capture=False)


if __name__ == '__main__':
    main()
