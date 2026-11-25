# macOS bundle pipeline

This directory contains source inputs for Apple Silicon and Intel packages. No
Mac application has been built or run during this Windows development session.
The maintainer has waived Mac real-machine validation for this round. That waiver
does not turn an unexecuted build into a successful build.

## Build on an existing native Mac toolchain

Requires Python 3.11+, Rust and Xcode Command Line Tools already on the builder.
No Homebrew installation is performed by these scripts. The finished application
does not need any of those development tools installed on the user's Mac.

```sh
python3 -B scripts/build-macos.py --target aarch64-apple-darwin
# Use a native Intel runner for this target:
python3 -B scripts/build-macos.py --target x86_64-apple-darwin
```

Each target has a separate `.build/macos/<target>/` and `target/<target>/` tree.
Archives and Cargo packages are cached under `.build/`. The build uses one Cargo
job, a lower scheduling priority and project-private temporary directories. It
never changes Windows runtime files, its prepared manifest, the Windows installer
or anything in the parent web project. `--offline --offline-dir <directory>`
requires exact archives named in `downloads.lock.json`; there is no latest-version
fallback. `--no-package` compiles the app without making a distribution archive.

The GitHub workflow is manual and uses one matrix job at a time. No workflow has
been run here. Its optional retained artifacts are temporary build outputs, not a
public Release or a permanent source-material host. It never executes the GUI or
installs an app. It does not need a paid Apple account.

## Bundle contract

```text
流光.app/Contents/
  Info.plist
  MacOS/flow-desktop
  MacOS/flow-cli
  Resources/flow.icns
  Resources/engine-manifest.json
  Resources/engine/
    manifest.json
    yt-dlp
    ffmpeg
    ffprobe
    node
    catalog/ ...
    plugins/ ...
    licenses/ ...
```

Four engines are private files, located through the bundle. None are installed
into `/usr/local/bin`, `/opt/homebrew`, system framework directories or PATH. The
yt-dlp executable includes its Python runtime. Its universal upstream executable
is preserved, rather than stripping an embedded self-extracting archive without
validating that transformation. Node and media binaries are architecture-specific.

`otool` recursively checks every declared non-system dynamic library dependency,
including per-architecture load commands. External Homebrew or developer-machine
paths cause a hard failure. `lipo` checks actual architecture and the target slice's
minimum macOS version. This static audit cannot inspect dynamic loads inside the
PyInstaller archive or prove all OS/driver interactions.

The current build floor is **macOS 13.5**, matching the pinned Node 24.15.0 release's
[documented deployment target](https://github.com/nodejs/node/blob/v24.15.0/BUILDING.md).
Every bundle binary must pass its Mach-O deployment check at or below that floor.
This is not a claim of actual compatibility testing on macOS 13.5.

## Integrity and signing order

1. Verify each fixed archive SHA-256 before reading any member.
2. Extract only the intended regular files, and audit architecture and dependencies.
3. Sign the private runtime copies, then record their final payload hashes.
4. Collect the selected target's actual Cargo dependency notices and generate its
   `engine-manifest.json`; embed it via `FLOW_MACOS_RESOURCE_MANIFEST` during Rust compilation.
5. Copy the exact resources into the bundle, sign only the GUI, CLI and outer app,
   and verify the resource hashes again. Never use `codesign --deep` for signing.
6. Build and verify a DMG; emit its SHA-256 and per-file/architecture report.

Default signatures are ad hoc, useful for native Mach-O integrity but not a trusted
Developer ID identity. Downloads can still trigger Gatekeeper restrictions. No
script clears quarantine or instructs users to disable Gatekeeper. Optional
`--sign-identity 'Developer ID Application: …'` signs private runtime copies before
compilation. Only Node receives JIT/executable-memory entitlements, derived from
[the pinned Node signing input](https://github.com/nodejs/node/blob/v24.15.0/tools/osx-entitlements.plist)
without its debugger, DYLD-environment or library-validation exceptions. A tiny
JavaScript evaluation checks the signed Node copy before compilation. yt-dlp receives the library
validation exception needed by its bundled Python modules. The app itself receives
neither entitlement. Public notarization is an explicit optional action through
`--notarize-profile <existing-keychain-profile>`; no credentials are embedded.

## Runtime selection and actual feature boundary

yt-dlp 2026.10.21 comes from its official GitHub release and Node 24.15.0 from the
Node release server. Their exact archive hashes are pinned in this directory.
FFmpeg/FFprobe 9.0.2 are **Martin Riedl's third-party GPL builds**, with fixed build
paths and publisher SHA-256 files. They are not official FFmpeg-project binaries.
They are not the Windows 8.1.3 build and must not be described as having every
Windows codec, protocol, filter or hardware backend.

The selected published configurations include x264/x265, AV1, VPx, MP3/Opus/Vorbis,
subtitle libraries and common media processing. The preparation step verifies the
actual executable's encoders, decoders, containers, protocols, filters and options
needed by Flow's merge, recode, audio extraction, section, subtitle, metadata,
chapter and thumbnail workflows. It records full command output. This capability
check is small and is not a substitute for a real-media or GUI regression test.

Pinned publisher build descriptions:

- [Apple Silicon 9.0.2](https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/versions.txt)
- [Intel 9.0.2](https://ffmpeg.martin-riedl.de/download/macos/amd64/1789931006_9.0.2/versions.txt)
- [Public build recipes](https://git.martin-riedl.de/ffmpeg/build-script)

## Source and license status

Node's complete notice is taken from its pinned official archive. yt-dlp's
unchanged release Unlicense and aggregate third-party notice are shared portable
inputs, not a claim that the Windows dependency graph describes Mac. GPLv3 is
included for the chosen FFmpeg configuration. Cargo notices are collected from
the selected Mac graph, using exact-version archived notices and reviewed
upstream source records when packages omit their license files; unknown missing
texts fail instead of silently reusing a differently versioned package. Only
package IDs present in Cargo's selected `resolve.nodes` graph are included.

For a separate source-only license review, run
`python3 -B scripts/collect-macos-notices.py --target <target>`. Its default output
is `.build/macos/<target>/notice-review/`, separate from the owned runtime stage;
`--output` may choose another directory within `.build/macos/`. It does not create
an incomplete engine directory that would prevent a later preparation.

The complete FFmpeg dependency license/source coverage is **not yet verified**.
Its published configuration differs from Windows, and the old Windows source ZIP
is not proof of Mac corresponding-source completeness. Generated reports preserve
`complete_corresponding_source_verified: false` and `complete_licensing_review:
false`. Before public binary distribution, the maintainer must complete and host
the matching upstream source and notice materials. No public release or upload is
performed by the build scripts. These pending source obligations are independent
of the user's decision not to conduct real-machine UI/installation testing.

## Cleanup

`python3 -B scripts/package-macos.py --target <target> --clean-stage` removes only
the stage files recorded under our ownership marker, after comparing every hash.
Unknown or modified files stop cleanup and remain on disk. Runtime preparation
uses the same ownership rule for its own previous stage. Downloads and Cargo
caches are intentionally reusable; the scripts never recursively delete an
arbitrary caller-supplied directory.
