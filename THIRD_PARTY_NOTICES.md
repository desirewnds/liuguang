# Third-party notices

流光原生桌面版使用下列第三方组件。应用本体采用根目录 [LICENSE](LICENSE) 中的 Apache-2.0，源码仓库为 [desirewnds/liuguang](https://github.com/desirewnds/liuguang)。2.0 原生发行包不包含 WebView2、Tauri 或 React。各组件的版权及许可证仍属于各自作者，不会因项目采用 Apache-2.0 而改变；组件无担保条款见各自许可全文。

## yt-dlp

Source-material storage: source paths below refer to members of the maintainer's archived `engine/sources/open-source-materials.zip`, which is excluded from Git. A binary release must provide its matching source materials separately; this source repository does not contain that archive or claim complete corresponding-source coverage. Runtime notices under `engine/licenses/` remain ordinary repository files. Archived documents preserve collection-time records; current layout is described in [source status](docs/third-party-source-status.md).

- Version: **2026.10.21**.
- Project and binary: <https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.21>.
- Project code: Unlicense, see `engine/licenses/yt-dlp-UNLICENSE.txt`.
- The official Windows executable embeds additional dependencies. Their complete supplied notices are preserved in `engine/licenses/yt-dlp-THIRD-PARTY.txt`; the executable must not be described as having only one license.
- The copied executable was checked against the official release `SHA2-256SUMS`. The exact executable hash is in `engine/downloads.lock.json`.
- Original source archive retained as `engine/sources/yt-dlp-2026.10.21.zip`.

## FFmpeg and FFprobe

- Binary version: **n8.1.3-35-gf1acb0a266-20261114**, Windows x64 GPL shared build.
- Build release: <https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/tag/autobuild-2026-11-14-12-37> (the project formerly used the BtbN/FFmpeg-Builds location).
- Selected binary archive: `ffmpeg-n8.1.3-35-gf1acb0a266-win64-gpl-shared-8.1.zip`.
- Archive SHA-256: `94dd295461cada31a9e7f79ec0f9ec317d9afcbfe9e60b337679bce2c5a60016`.
- The two command-line executables share seven private FFmpeg DLLs in `engine/`. They do not install system libraries, change PATH or require a separately installed FFmpeg. The upstream `ffplay` program and development headers/import libraries were not part of the original application runtime and are not added by this change.
- Source recipe commit: `c9fe5114060e507409f7ee07edfd82efaf25d767`.
- This build enables GPL and version 3 features. Its complete supplied license is `engine/licenses/ffmpeg-LICENSE.txt`; `engine/licenses/GPL-3.0.txt` also preserves the GNU text. Do not label this build “LGPL-only”.
- Build recipe licensing and instructions are preserved in `engine/licenses/FFmpeg-Builds-LICENSE` and `engine/licenses/FFmpeg-Builds-README.txt`.
- Exact build configuration is captured from the selected executable in `engine/licenses/ffmpeg-build.txt`.
- Supplied dependency license and copyright texts are preserved in `engine/licenses/ffmpeg-dependencies/`, indexed by `engine/licenses/ffmpeg-dependencies.json`. The GCC Runtime Library Exception is included as `engine/licenses/GCC-RUNTIME-LIBRARY-EXCEPTION.txt`.

FFmpeg and FFprobe are separate processes, not C libraries linked into the application. Their executables, private DLLs and third-party licensing obligations still apply when redistributing them. This shared edition remains a GPL build: it is not an LGPL-only substitution. FFmpeg project source, dependency source revisions, build recipes, nested source and Cargo lock verification reports are in `engine/sources/`. Read [source status](docs/third-party-source-status.md) for the recorded coverage and remaining verification limits; an archive of FFmpeg alone does not contain all code linked into these executables and DLLs.

Version 1.0.2 retains the exact FFmpeg revision, build recipe and enabled features used in 1.0.1. The shared candidate's version, compiler and library ABIs were compared, along with 16 capability listings, both full command-line help outputs and media processing fixtures. The report and captured outputs are archived as `engine/sources/ffmpeg-shared-verification.json` and its adjacent directory. Original 1.0.1 lock and configuration records remain for audit; old static binary archives were removed during cleanup.

The original web application's Gyan FFmpeg 8.1.2 binaries were not modified. Associated 8.1.2 source, notices and research records remain inside the consolidated archive; duplicate binary archives were removed. These records are provenance research, not the desktop runtime selection. Desktop FFmpeg was replaced with a build whose dependency recipes are publicly available.

## Node.js

- Version: **24.15.0**, official Windows x64 executable.
- Binary: <https://nodejs.org/dist/v24.15.0/win-x64/node.exe>.
- The executable hash was checked against the official `SHASUMS256.txt` at <https://nodejs.org/dist/v24.15.0/SHASUMS256.txt>.
- Complete Node.js and bundled dependency copyright/license notices: `engine/licenses/node-LICENSE.txt`, obtained from <https://github.com/nodejs/node/blob/v24.15.0/LICENSE>.
- Node.js core uses the MIT license; bundled dependencies retain the additional terms listed in that file. Do not reduce the aggregate executable to an unsupported single-license claim.

Node is included only as a JavaScript runtime for the download engine; ordinary users do not need to install it separately.

## Iced

- The native UI uses Iced 0.14.0, with exact sub-crate revisions in Cargo.lock.
- Iced uses the MIT license. The original copyright and permission notices are preserved under packaging/notices/application-dependencies/.
- Some published sub-crates omit the workspace-root license. Their exact release commit is taken from .cargo_vcs_info.json; upstream texts, Git blob identities and source URLs are archived in packaging/upstream-notices/.
- No browser runtime is required by the native application. Old WebView2 files and documentation are historical baseline material, excluded from the native runtime.

## Application build dependencies

The native application uses Rust libraries including Iced, wgpu, tiny-skia, Tokio, rusqlite, reqwest and Windows integration crates. Exact dependency versions are pinned in the workspace Cargo.lock. scripts/collect-native-notices.py collects the actual Windows GNU dependency metadata and supplied notices into packaging/notices/. The packager overlays this index into the installed engine/licenses/application-dependencies.json; it does not ship the old Tauri/npm index.

The index includes development/build dependencies for transparency; it does not imply that every listed package is linked into the runtime. Missing texts are reported rather than assigned a guessed license. The index deliberately distinguishes archived notices from a completed legal review.

The macOS port has additional target-specific AppKit, Foundation, notification and Unix dependencies. The existing Windows index is not evidence of macOS license coverage. Each macOS architecture must collect its resolved dependency notices and its actual media-engine provenance before packaging; source adaptation alone does not establish that these steps have run. See docs/macos.md for the current build status.

The current 287-package index contains one independently copyleft Rust dependency, option-ext 0.2.0 (MPL-2.0). Its unchanged .crate source is already present in the consolidated material archive at engine/sources/application/option-ext-0.2.0.crate. The entry SHA-256 matches Cargo.lock: 04744f49eae99ab78e0d5c0b603ab218f515ea8cfe5a456d7629ad883a3b6e7d. self_cell 1.3.0 offers Apache-2.0 OR GPL-2.0-only; this distribution uses its Apache-2.0 option and preserves that text. This focused source coverage check does not replace the existing media-engine source review.

No custom font binaries are bundled; the UI uses the operating system's font stack. The project's supplementary Bilibili plugin under `engine/plugins/` is part of the application source covered by Apache-2.0. yt-dlp and other third-party code retain their own licenses.

## Distribution record

Native runtime integrity hashes are compiled from `packaging/native-engine-manifest.json`; the historical full engine baseline remains in `engine/manifest.json`; downloadable binary hashes are in `engine/downloads.lock.json`; source artifact hashes are in `engine/sources-manifest.json`. Build recipes and source archives are separate release materials and are intentionally excluded from the installed runtime.

The application source is maintained at [desirewnds/liuguang](https://github.com/desirewnds/liuguang) under Apache-2.0. Public binary releases must preserve the notices and provide the applicable matching source materials alongside the installer. Upstream daily binary releases have a retention limit, so a permanent release must not depend solely on those transient download links. Full media-engine source coverage remains under review; existing Windows builds are unsigned, and macOS builds and real-machine validation have not been completed. Publishing this source repository does not establish binary-release, signing or runtime-verification results.
