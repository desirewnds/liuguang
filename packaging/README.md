# Native Windows packaging

The Iced package is independent of Tauri, npm, WebView2 and external NSIS plugins.
It uses the existing Windows x64 Rust toolchain and an explicitly supplied NSIS
compiler. It does not install a compiler, alter global PATH, register media DLLs,
run an installer, or upload a release.

## Build

Run from the desktop project root after the native Cargo lockfile and dependency
cache are prepared. This is a build instruction, not a command run by these docs:

```powershell
./scripts/build-native.ps1 -Makensis 'C:/explicit/NSIS/makensis.exe'
```

`-Offline` requires the project-scoped Cargo cache to already contain dependencies.
The default is one Cargo job and BelowNormal process priority. `-Jobs` is an
explicit opt-in to higher parallelism. Process priority and environment are
restored even if a build step fails; no benchmark or installer is launched.
`-ToolDirectories` accepts explicit compiler/CMake directories and prepends them
to PATH only for this script and its children. Existing environment variables are
restored on return. Cargo cache is `.build/cargo`, scratch is `.build/tmp`, and
build output is `target/`. No developer cache is included in the application.
First-time native engine restoration uses `python -B scripts/prepare-native-engines.py`;
pass `--offline-dir` for previously obtained exact archives, or `--verify-only`
for a read-only check. `--list` shows the selected packages without downloading
or hashing them. This entry point selects yt-dlp, shared FFmpeg and Node only;
it never selects WebView2, installs global tools or modifies the historical lock.
It imports checksum/extraction helpers from `prepare-engines.py`, whose legacy
standalone entry point should not be used for a native build.

Use `-NoPackage` to stop at the verified runnable payload; use `-StageOnly` to
reuse already built release binaries. The payload is under
`target/native-package/payload`, with `flow-desktop.exe`, `flow-cli.exe`, `flow.cmd`
and one `engine/` directory. The source-of-truth engine directory remains intact.

`package-native.py --prepare-manifest` verifies the canonical SHA-pinned resources,
excludes the browser and old Tauri/npm notices, then adds the native notices
whose index must match `Cargo.lock`. The generated
`packaging/native-engine-manifest.json` is a reviewable, checked-in compilation
input for the native installer helper. `--stage` verifies both that file and its
verbatim bytes embedded in the compiled CLI without executing the binary, then
uses hardlinks where supported.
The GUI, CLI and resource manifest must be rebuilt after pinned resources change.
The original `downloads.lock.json` bytes remain unchanged; its browser package is
historical acquisition metadata, not an installed runtime or download action.
The native manifest contains no WebView2 files. The canonical material archive
does not enter the payload.

The build script collects native notices before preparing the manifest. A
development-only `--prepare-manifest --allow-incomplete-notices` permits code
checks while outstanding license texts are being collected. Staging and final
packaging always reject missing texts or stale lock hashes; that development
flag cannot bypass release checks. Archived notices do not by themselves establish
that every source-distribution/license obligation has completed legal review.

## Check without installing

```powershell
python -B scripts/package-native.py --verify-inputs
python -B scripts/package-native.py --audit
python -B scripts/package-native.py --audit-imports
python -B packaging/test_native_package.py --makensis 'C:/explicit/NSIS/makensis.exe'
python -B packaging/test_native_source.py
```

The test compiler receives tiny non-runnable payloads in disposable project-local
fixtures; the resulting installer is never launched and is deleted with the
fixture. These checks cover payload selection, hash verification, path handling,
explicit deletion lists and NSIS syntax. They do not prove installed Windows
behavior or GUI layout. The real `--compile` command always checks PE ordinary and
delay imports against same-directory libraries and a Windows-system allowlist;
optional dynamically loaded GPU drivers are outside that static check.

## Installation lifecycle

- Per-user installation keeps the existing `org.flowdown.desktop` identity and
  product uninstall key. No PATH or shared-runtime changes are made.
- The installer's own helper is extracted into NSIS's private plugin directory
  and checks the target before payload copying. Existing executables are not
  trusted for preflight.
- An existing registered installation is checked, then its exact `uninstall.exe`
  is invoked with `/UPDATE`. No arbitrary registry command string is executed.
- GUI/CLI process checks do not kill tasks. Active application processes block
  replacement until the user exits them.
- Initialization records SHA-backed installed-file ownership and imports existing
  data through the platform lifecycle code. A pending ownership record is written
  before files are promoted. Each file is copied to a transaction-specific sibling
  temporary file, flushed and verified, then promoted without overwriting a late
  foreign file. Re-running the same installer at the same location can resume
  partial copying; modified files or a different payload are preserved and rejected.
  Registration is retained when initialization fails. This is safe retry support,
  not a promise of automatic whole-install rollback: an old installation may
  already have been removed before a later upgrade step fails.
- Normal uninstall cleans owned application state by default. `/KEEPDATA`
  retains settings/history and wins over `/DELETEAPPDATA`; `/UPDATE` skips data
  cleanup entirely. User video paths are never installation deletion roots.
- NSIS deletes only the generated file list; directory removal is empty-only.
  Linked paths are rejected. Unknown or modified same-named files stop preflight.
  Shortcut creation/deletion validates its stored target through the helper.

## Cleanup and output

`python -B scripts/package-native.py --clean` removes only an intact owned stage.
Untracked or modified stage files are preserved with an error. Rebuilding never
overwrites an existing release EXE automatically; review/remove a failed build
explicitly before retrying. The original 1.0.2 installer remains until native
delivery has completed verification.

The native versioned manifest and SHA-256 file accompany the installer under
`release/`. These are unsigned local artifacts, not published releases. Isolated
installation, upgrade, rollback and uninstall checks remain separate acceptance
steps; no claim of completion should be inferred from package compilation alone.

## Source distribution

`python -B scripts/package-native-source.py --list` reports the explicit native
source allowlist and bytes without writing an archive. `--archive` is the separate
packaging action, refuses an existing output, and leaves the previous source or
third-party-materials archives unchanged. It includes the Rust workspace, native
packaging sources, manifests, notices, required catalog/plugin source and docs.
It excludes the old frontend/Tauri tree, runtime binaries, developer caches,
media, environment credentials and application state. This small source archive
does not replace the separate third-party corresponding-source materials.
