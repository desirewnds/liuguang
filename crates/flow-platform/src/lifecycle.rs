// SPDX-License-Identifier: Apache-2.0
//! Ownership boundaries for desktop state. Media directories are never cleanup roots.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

pub const APP_ID: &str = "org.flowdown.desktop";

/// Resource lookup is independent of the current working directory and PATH.
pub fn engine_directory(executable: &Path) -> Result<PathBuf, String> {
    let parent = executable.parent().ok_or("无法取得引擎位置")?;
    #[cfg(target_os = "macos")]
    if parent.file_name().is_some_and(|name| name == "MacOS") {
        if let Some(contents) = parent
            .parent()
            .filter(|path| path.file_name().is_some_and(|name| name == "Contents"))
        {
            return Ok(contents.join("Resources/engine"));
        }
    }
    Ok(parent.join("engine"))
}

pub fn executable_path(directory: &Path, name: &str) -> PathBuf {
    directory.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    })
}

/// Isolated validation must not alter the real app's OS-owned notification history.
pub fn isolation_active() -> bool {
    [
        "FLOW_LOCAL_DATA_DIR",
        "FLOW_ROAMING_DATA_DIR",
        "FLOW_DOWNLOAD_DIR",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some())
}
const SHELL_MARKER: &str = ".flow-shell-owner.json";
const CORE_MARKER: &str = ".flow-owned-v2.json";
const INSTALL_MARKER: &str = ".flow-install-owner.json";

#[derive(Clone)]
pub struct Paths {
    pub roaming: PathBuf,
    pub local: PathBuf,
    pub data_base: PathBuf,
    pub shell: PathBuf,
    pub downloads: PathBuf,
}

impl Paths {
    pub fn system() -> Result<Self, String> {
        validate_isolation_overrides([
            std::env::var_os("FLOW_LOCAL_DATA_DIR").is_some(),
            std::env::var_os("FLOW_ROAMING_DATA_DIR").is_some(),
            std::env::var_os("FLOW_DOWNLOAD_DIR").is_some(),
        ])?;
        let roaming = explicit_path(
            "FLOW_ROAMING_DATA_DIR",
            dirs::data_dir().ok_or("无法取得应用数据位置")?.join(APP_ID),
        )?;
        #[cfg(target_os = "macos")]
        let local_default = dirs::cache_dir().ok_or("无法取得缓存位置")?.join(APP_ID);
        #[cfg(not(target_os = "macos"))]
        let local_default = dirs::data_local_dir()
            .ok_or("无法取得本地数据位置")?
            .join(APP_ID);
        let local = explicit_path("FLOW_LOCAL_DATA_DIR", local_default)?;
        let data_base = std::env::var_os("FLOW_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| roaming.clone());
        let videos = dirs::video_dir()
            .or_else(dirs::download_dir)
            .or_else(|| {
                dirs::home_dir().map(|p| {
                    p.join(if cfg!(target_os = "macos") {
                        "Movies"
                    } else {
                        "Videos"
                    })
                })
            })
            .ok_or("无法取得视频保存位置")?;
        let downloads = explicit_path("FLOW_DOWNLOAD_DIR", videos.join("流光"))?;
        Self::for_locations(roaming, local, data_base, downloads)
    }

    /// Construct explicit application locations, also used by isolated tests.
    pub fn for_locations(
        roaming: PathBuf,
        local: PathBuf,
        data_base: PathBuf,
        downloads: PathBuf,
    ) -> Result<Self, String> {
        for path in [&roaming, &local, &data_base, &downloads] {
            safe_path(path)?;
        }
        if overlaps(&downloads, &local)
            || overlaps(&downloads, &roaming)
            || overlaps(&downloads, &data_base)
        {
            return Err("视频目录与应用数据目录必须分开".into());
        }
        #[cfg(target_os = "macos")]
        let shell = roaming.join("shell-v2");
        #[cfg(not(target_os = "macos"))]
        let shell = local.join("shell-v2");
        Ok(Self {
            roaming,
            shell,
            local,
            data_base,
            downloads,
        })
    }

    pub fn initialize(&self) -> Result<(), String> {
        ensure_owned(&self.shell)?;
        #[cfg(target_os = "macos")]
        ensure_owned(&self.cache_base())?;
        for name in ["cli-cache", "runtime"] {
            let path = self.cache_base().join(name);
            safe_path(&path)?;
            private_directory(&path)?;
        }
        Ok(())
    }

    /// Windows preserves its existing layout. macOS keeps disposable data in
    /// Library/Caches while window state and the CLI engine remain persistent.
    pub fn cache_base(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.local.join("cache-v2")
        } else {
            self.shell.clone()
        }
    }

    pub fn core_cache_root(&self) -> Option<PathBuf> {
        if cfg!(target_os = "macos") {
            // A custom data directory has its own owner marker. Keep its
            // disposable cache separate when users switch FLOW_DATA_DIR.
            let identity = Sha256::digest(normalized(&self.data_base).as_bytes());
            Some(self.cache_base().join("core").join(format!("{identity:x}")))
        } else {
            None
        }
    }

    pub fn register_data(&self) -> Result<(), String> {
        self.initialize()?;
        safe_path(&self.data_base)?;
        let index = self.shell.join("storage-locations.json");
        let mut entries = registered_roots(self)?;
        entries.insert(self.data_base.clone());
        write_json(&index, &json!({"schema":1,"app":APP_ID,"roots":entries}))?;
        capture_legacy_shell(self)?;
        Ok(())
    }
}

fn validate_isolation_overrides(provided: [bool; 3]) -> Result<(), String> {
    if provided.iter().any(|v| *v) && !provided.iter().all(|v| *v) {
        return Err("隔离运行必须同时设置 FLOW_LOCAL_DATA_DIR、FLOW_ROAMING_DATA_DIR 和 FLOW_DOWNLOAD_DIR，避免写入正常使用的数据目录".into());
    }
    Ok(())
}

/// Remove only the previous GUI's owned private browser cache, preserving CLI state.
pub fn cleanup_obsolete_webview(paths: &Paths) -> Result<bool, String> {
    let browser = paths.shell.join("webview");
    if !browser.exists() {
        return Ok(false);
    }
    if !shell_owned(&paths.shell)? {
        return Err("旧界面缓存没有有效所有权标记，已保留".into());
    }
    let mut protected = vec![paths.downloads.clone()];
    for base in registered_roots(paths)? {
        protected.extend(protected_roots(&base)?);
    }
    if protected.iter().any(|path| overlaps(&browser, path)) {
        return Err("旧界面缓存与视频位置重叠，已保留".into());
    }
    remove_private_tree(&browser)?;
    Ok(true)
}

fn explicit_path(name: &str, fallback: PathBuf) -> Result<PathBuf, String> {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or(fallback);
    safe_path(&path)?;
    Ok(path)
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn is_reparse(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn directory_entry(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x10 != 0
    }
    #[cfg(not(windows))]
    {
        meta.is_dir()
    }
}

#[cfg(target_os = "macos")]
fn system_alias(path: &Path, metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    let expected = match path.to_str() {
        Some("/tmp") => "/private/tmp",
        Some("/var") => "/private/var",
        Some("/etc") => "/private/etc",
        _ => return false,
    };
    metadata.uid() == 0
        && fs::read_link(path).is_ok_and(|target| {
            let target = if target.is_absolute() {
                target
            } else {
                path.parent().unwrap_or(Path::new("/")).join(target)
            };
            target == Path::new(expected)
        })
}

/// Check every existing ancestor, not just the final path. Never traverse junctions.
pub fn safe_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(format!(
            "必须使用不含相对跳转的绝对路径：{}",
            path.display()
        ));
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if is_reparse(&meta) => {
                #[cfg(target_os = "macos")]
                if system_alias(ancestor, &meta) {
                    continue;
                }
                return Err(format!(
                    "为保护文件，不处理目录链接：{}",
                    ancestor.display()
                ));
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(error(e)),
        }
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, String> {
    safe_path(path)?;
    let metadata = fs::metadata(path).map_err(error)?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err("数据位置记录无效".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)
}

fn write_json(path: &Path, data: &Value) -> Result<(), String> {
    safe_path(path)?;
    let staged = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staged)
        .map_err(error)?;
    file.write_all(&serde_json::to_vec_pretty(data).map_err(error)?)
        .map_err(error)?;
    file.sync_all().map_err(error)?;
    drop(file);
    // Same-directory rename replaces the old record atomically on Windows. If a
    // handle or ACL prevents replacement, the previous ownership proof remains.
    fs::rename(staged, path).map_err(error)
}

fn ensure_owned(root: &Path) -> Result<(), String> {
    safe_path(root)?;
    let marker = root.join(SHELL_MARKER);
    if marker.exists() {
        if !shell_owned(root)? {
            return Err("应用数据所有权标记不匹配，已保留原文件".into());
        }
        return Ok(());
    }
    if root.exists() && fs::read_dir(root).map_err(error)?.next().is_some() {
        return Err(format!(
            "应用私有目录已有未识别文件，请保留并检查：{}",
            root.display()
        ));
    }
    private_directory(root)?;
    write_json(
        &marker,
        &json!({"schema":2,"kind":"flow-shell-state","app":APP_ID,"id":uuid::Uuid::new_v4().to_string()}),
    )
}

fn shell_owned(root: &Path) -> Result<bool, String> {
    let marker = root.join(SHELL_MARKER);
    if !marker.exists() {
        return Ok(false);
    }
    let data = read_json(&marker)?;
    Ok(data["schema"] == 2 && data["kind"] == "flow-shell-state" && data["app"] == APP_ID)
}

fn core_owned(root: &Path) -> Result<bool, String> {
    let marker = root.join(CORE_MARKER);
    if !marker.exists() {
        return Ok(false);
    }
    let data = read_json(&marker)?;
    Ok(data["schema"] == 2
        && data["kind"] == "flow-managed-state"
        && data["id"]
            .as_str()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()))
}

pub(crate) fn registered_roots(paths: &Paths) -> Result<BTreeSet<PathBuf>, String> {
    let mut result = BTreeSet::from([paths.roaming.clone()]);
    let index = paths.shell.join("storage-locations.json");
    if !index.exists() {
        return Ok(result);
    }
    if !shell_owned(&paths.shell)? {
        return Err("应用数据位置记录没有有效所有权标记".into());
    }
    let record = read_json(&index)?;
    if record["schema"] != 1 || record["app"] != APP_ID {
        return Err("应用数据位置记录无效".into());
    }
    let roots = record["roots"].as_array().ok_or("应用数据位置列表无效")?;
    if roots.len() > 128 {
        return Err("应用数据位置记录数量异常".into());
    }
    for value in roots {
        let root = PathBuf::from(value.as_str().ok_or("应用数据位置无效")?);
        safe_path(&root)?;
        result.insert(root);
    }
    Ok(result)
}

pub(crate) fn normalized(path: &Path) -> String {
    #[cfg(windows)]
    {
        let value = path
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase();
        if let Some(rest) = value.strip_prefix("//?/unc/") {
            format!("//{rest}")
        } else {
            value.strip_prefix("//?/").unwrap_or(&value).to_string()
        }
    }
    #[cfg(not(windows))]
    {
        // Canonicalize only the existing prefix so paths for a new file remain
        // comparable. Never fold case: APFS supports both case modes.
        let mut ancestor = path;
        let mut missing = Vec::new();
        while !ancestor.exists() {
            let Some(name) = ancestor.file_name() else {
                break;
            };
            missing.push(name.to_owned());
            let Some(parent) = ancestor.parent() else {
                break;
            };
            ancestor = parent;
        }
        let mut actual = fs::canonicalize(ancestor).unwrap_or_else(|_| ancestor.to_path_buf());
        for name in missing.into_iter().rev() {
            actual.push(name);
        }
        actual.to_string_lossy().trim_end_matches('/').to_owned()
    }
}
fn overlaps(a: &Path, b: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;
        let inside = |path: &Path, root: &Path| {
            fs::metadata(root).is_ok_and(|root_meta| {
                path.ancestors().any(|part| {
                    fs::metadata(part).is_ok_and(|meta| {
                        meta.dev() == root_meta.dev() && meta.ino() == root_meta.ino()
                    })
                })
            })
        };
        if inside(a, b) || inside(b, a) {
            return true;
        }
    }
    let a = normalized(a);
    let b = normalized(b);
    a == b || a.starts_with(&(b.clone() + "/")) || b.starts_with(&(a + "/"))
}

fn private_directory(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(error)
}

fn digest(path: &Path) -> Result<String, String> {
    safe_path(path)?;
    let mut file = fs::File::open(path).map_err(error)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(error)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn protected_roots(base: &Path) -> Result<Vec<PathBuf>, String> {
    let mut protected = vec![base.join("downloads")];
    let record = base.join("managed-v2/legacy-owned-files.json");
    if record.exists() {
        let data = read_json(&record)?;
        if !data["protected_media_roots"].is_null() {
            let roots = data["protected_media_roots"]
                .as_array()
                .ok_or("历史视频位置记录无效，已保留应用数据")?;
            if roots.len() > 4096 {
                return Err("历史视频位置数量异常，已保留应用数据".into());
            }
            for value in roots {
                let path = PathBuf::from(value.as_str().ok_or("历史视频路径无效，已保留应用数据")?);
                // Ambiguous or linked historic locations must not weaken media protection.
                safe_path(&path)?;
                protected.push(path);
            }
        }
    }
    Ok(protected)
}

/// Only delete the explicitly owned private tree. A linked child is unlinked, never followed.
fn remove_private_tree(root: &Path) -> Result<(), String> {
    safe_path(root)?;
    if !root.exists() {
        return Ok(());
    }
    let marker_backups: Vec<_> = [
        SHELL_MARKER,
        CORE_MARKER,
        "legacy-owned-files.json",
        "storage-locations.json",
        "legacy-shell-files.json",
    ]
    .into_iter()
    .filter_map(|name| fs::read(root.join(name)).ok().map(|bytes| (name, bytes)))
    .collect();
    let mut entries = fs::read_dir(root)
        .map_err(error)?
        // Keep only paths: an entry may retain its directory enumeration handle.
        // Release every entry and the iterator before removing any directories.
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    // Keep ownership and migration records until all other files have been removed,
    // so interrupted/locked-file cleanup can safely retry.
    entries.sort_by_key(
        |path| match path.file_name().and_then(|name| name.to_str()) {
            Some(SHELL_MARKER | CORE_MARKER) => 2,
            Some(
                "legacy-owned-files.json" | "storage-locations.json" | "legacy-shell-files.json",
            ) => 1,
            _ => 0,
        },
    );
    for path in entries {
        let meta = fs::symlink_metadata(&path).map_err(error)?;
        if is_reparse(&meta) {
            // RemoveDirectoryW unlinks a directory junction itself; no target enumeration.
            if directory_entry(&meta) {
                fs::remove_dir(&path).map_err(error)?;
            } else {
                fs::remove_file(&path).map_err(error)?;
            }
        } else if meta.is_dir() {
            remove_private_tree(&path)?;
        } else {
            safe_path(&path)?;
            fs::remove_file(&path).map_err(error)?;
        }
    }
    match fs::remove_dir(root) {
        Ok(()) => Ok(()),
        Err(failure) => {
            // A locked root may survive after its files were removed. Restore ownership
            // proof so a later retry never mistakes it for an unrelated directory.
            safe_path(root)?;
            for (name, bytes) in marker_backups {
                if let Ok(mut file) = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(root.join(name))
                {
                    file.write_all(&bytes).map_err(error)?;
                }
            }
            // Do not enumerate and recursively delete again: any newly appeared
            // entry was not part of the owned cleanup snapshot.
            Err(format!(
                "无法移除应用私有目录 {}：{}；已保留剩余文件和所有权记录",
                root.display(),
                failure
            ))
        }
    }
}

fn relative_file(base: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("旧版文件清单包含不安全路径".into());
    }
    let result = base.join(path);
    safe_path(&result)?;
    Ok(result)
}

fn cleanup_legacy(
    base: &Path,
    protected: &[PathBuf],
    kept: &mut Vec<String>,
) -> Result<(), String> {
    let manifest = base.join("managed-v2/legacy-owned-files.json");
    if !manifest.exists() {
        return Ok(());
    }
    let record = read_json(&manifest)?;
    if record["schema"] != 1
        || record["legacy_root"]
            .as_str()
            .is_none_or(|p| normalized(Path::new(p)) != normalized(base))
    {
        return Err("旧版文件清单与当前目录不符".into());
    }
    let files = record["files"].as_array().ok_or("旧版文件清单无效")?;
    for file in files {
        let relative = file["path"].as_str().ok_or("旧版文件名无效")?;
        // Never trust arbitrary filenames from a manifest. Only known DBs and UUID logs qualify.
        let allowed_database = matches!(
            relative,
            "flow.sqlite3" | "flow.sqlite3-wal" | "flow.sqlite3-shm" | "flow.sqlite3-journal"
        );
        let allowed_engine = matches!(
            relative,
            "engine/yt-dlp.exe"
                | "engine/yt-dlp.previous"
                | "engine/yt-dlp.exe.new"
                | "engine/yt-dlp.exe.old"
                | "engine/staging/yt-dlp.exe"
                | "engine/staging/yt-dlp.exe.new"
                | "engine/staging/yt-dlp.exe.old"
                | "engine/native-update/yt-dlp.exe"
                | "engine/native-update/yt-dlp.exe.new"
                | "engine/native-update/yt-dlp.exe.old"
        );
        let p = Path::new(relative);
        let allowed_log = p.parent() == Some(Path::new("logs"))
            && p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
            && matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("log" | "stdout")
            );
        if !allowed_database && !allowed_log && !allowed_engine {
            return Err("旧版清理清单包含非应用文件".into());
        }
        let path = relative_file(base, relative)?;
        if !path.exists() {
            continue;
        }
        if protected.iter().any(|root| overlaps(&path, root))
            || Some(fs::metadata(&path).map_err(error)?.len()) != file["bytes"].as_u64()
            || Some(digest(&path)?.as_str()) != file["sha256"].as_str()
        {
            kept.push(path.to_string_lossy().into());
            continue;
        }
        fs::remove_file(path).map_err(error)?;
    }
    let _ = fs::remove_dir(base.join("logs")); // Empty only; unknown logs remain.
    let _ = fs::remove_dir(base.join("engine/staging"));
    let _ = fs::remove_dir(base.join("engine/native-update"));
    let _ = fs::remove_dir(base.join("engine"));
    Ok(())
}

const LEGACY_BROWSER_SCOPES: &[&str] = &[
    "EBWebView/Default/Cache",
    "EBWebView/Default/Code Cache",
    "EBWebView/Default/GPUCache",
    "EBWebView/Default/DawnGraphiteCache",
    "EBWebView/Default/DawnWebGPUCache",
    "EBWebView/Default/Service Worker",
    "EBWebView/Default/Session Storage",
    "EBWebView/Default/Local Storage",
    "EBWebView/Default/IndexedDB",
    "EBWebView/Default/Network",
    "EBWebView/Default/blob_storage",
    "EBWebView/Default/WebStorage",
    "EBWebView/Default/Storage",
    "EBWebView/Default/shared_proto_db",
    "EBWebView/BrowserMetrics",
    "EBWebView/Crashpad",
    "EBWebView/ShaderCache",
    "EBWebView/GrShaderCache",
    "EBWebView/DawnGraphiteCache",
    "EBWebView/DawnWebGPUCache",
];
const LEGACY_BROWSER_JSON: &[&str] = &[
    "EBWebView/Local State",
    "EBWebView/Variations",
    "EBWebView/Default/Preferences",
    "EBWebView/Default/Secure Preferences",
];

fn browser_owned_relative(relative: &str) -> bool {
    let relative = relative.replace('\\', "/");
    LEGACY_BROWSER_JSON.contains(&relative.as_str())
        || LEGACY_BROWSER_SCOPES
            .iter()
            .any(|p| relative.starts_with(&format!("{p}/")))
}

fn capture_tree(
    base: &Path,
    directory: &Path,
    protected: &[PathBuf],
    rows: &mut Vec<Value>,
) -> Result<(), String> {
    if protected.iter().any(|p| overlaps(directory, p)) {
        return Ok(());
    }
    safe_path(directory)?;
    if !directory.is_dir() {
        return Ok(());
    }
    for item in fs::read_dir(directory).map_err(error)? {
        let path = item.map_err(error)?.path();
        let meta = fs::symlink_metadata(&path).map_err(error)?;
        if is_reparse(&meta) {
            continue;
        }
        if meta.is_dir() {
            capture_tree(base, &path, protected, rows)?;
        } else if meta.is_file() && !protected.iter().any(|p| overlaps(&path, p)) {
            rows.push(json!({"base":base,"path":path.strip_prefix(base).map_err(error)?.to_string_lossy().replace('\\',"/"),"sha256":digest(&path)?,"bytes":meta.len()}));
        }
    }
    Ok(())
}

fn capture_legacy_shell(paths: &Paths) -> Result<(), String> {
    let manifest = paths.shell.join("legacy-shell-files.json");
    if manifest.exists() {
        return Ok(());
    }
    let mut files = Vec::new();
    // Legacy shell files have fixed names and recognizable content. No directory-wide adoption.
    for (base, name) in [
        (&paths.roaming, "tray-explained"),
        (&paths.data_base, "tray-explained"),
        (&paths.roaming, ".window-state.json"),
    ] {
        let path = base.join(name);
        if !path.is_file() {
            continue;
        }
        safe_path(&path)?;
        let contents = fs::read(&path).map_err(error)?;
        let recognized = if name == "tray-explained" {
            contents == b"1"
        } else {
            serde_json::from_slice::<Value>(&contents).is_ok_and(|v| v["main"].is_object())
        };
        if recognized {
            files.push(
                json!({"base":base,"path":name,"sha256":digest(&path)?,"bytes":contents.len()}),
            );
        }
    }
    let mut protected = Vec::new();
    for base in registered_roots(paths)? {
        protected.extend(protected_roots(&base)?);
    }
    // Recognize the old application-specific profile; never scan a personal browser profile.
    let recognized_profile = ["EBWebView/Local State", "EBWebView/Default/Preferences"]
        .iter()
        .all(|name| read_json(&paths.local.join(name)).is_ok_and(|v| v.is_object()));
    if recognized_profile {
        for scope in LEGACY_BROWSER_SCOPES {
            capture_tree(
                &paths.local,
                &paths.local.join(scope),
                &protected,
                &mut files,
            )?;
        }
        for relative in LEGACY_BROWSER_JSON {
            let path = paths.local.join(relative);
            if !protected.iter().any(|p| overlaps(&path, p))
                && read_json(&path).is_ok_and(|v| v.is_object())
            {
                files.push(json!({"base":paths.local,"path":relative,"sha256":digest(&path)?,"bytes":fs::metadata(&path).map_err(error)?.len()}));
            }
        }
    }
    write_json(&manifest, &json!({"schema":1,"files":files}))
}

fn cleanup_legacy_shell(
    paths: &Paths,
    protected: &[PathBuf],
    kept: &mut Vec<String>,
) -> Result<(), String> {
    let manifest = paths.shell.join("legacy-shell-files.json");
    if !manifest.exists() {
        return Ok(());
    }
    let record = read_json(&manifest)?;
    for entry in record["files"].as_array().ok_or("旧版外壳文件清单无效")? {
        let name = entry["path"].as_str().ok_or("旧版外壳文件名无效")?;
        if !matches!(name, "tray-explained" | ".window-state.json") && !browser_owned_relative(name)
        {
            return Err("旧版外壳文件清单包含未知文件".into());
        }
        let base = PathBuf::from(entry["base"].as_str().ok_or("旧版外壳位置无效")?);
        if base != paths.local && !registered_roots(paths)?.contains(&base) {
            return Err("旧版外壳清单位置未注册".into());
        }
        let path = relative_file(&base, name)?;
        if path.exists() {
            if protected.iter().any(|root| overlaps(&path, root))
                || Some(digest(&path)?.as_str()) != entry["sha256"].as_str()
            {
                kept.push(path.to_string_lossy().into());
            } else {
                fs::remove_file(&path).map_err(error)?;
                if browser_owned_relative(name) {
                    let mut parent = path.parent();
                    while let Some(directory) = parent {
                        if directory == base {
                            break;
                        }
                        safe_path(directory)?;
                        if fs::remove_dir(directory).is_err() {
                            break;
                        }
                        parent = directory.parent();
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn cleanup(paths: &Paths, keep_data: bool) -> Result<Value, String> {
    safe_path(&paths.downloads)?;
    let roots = registered_roots(paths)?;
    let mut protected = vec![paths.downloads.clone()];
    for base in &roots {
        protected.extend(protected_roots(base)?);
    }
    let mut kept = Vec::new();
    // Validate every cleanup root before deleting anything.
    for base in &roots {
        let managed = base.join("managed-v2");
        safe_path(&managed)?;
        if managed.exists() && !core_owned(&managed)? {
            return Err(format!(
                "缺少应用所有权标记，保留目录：{}",
                managed.display()
            ));
        }
        if managed.exists() && protected.iter().any(|p| overlaps(p, &managed)) {
            return Err("应用数据与旧视频位置重叠，已保留全部文件".into());
        }
    }
    safe_path(&paths.shell)?;
    if paths.shell.exists() && !shell_owned(&paths.shell)? {
        return Err("外壳数据没有有效所有权标记，已保留文件".into());
    }
    if protected.iter().any(|p| overlaps(p, &paths.shell)) {
        return Err("外壳数据与旧视频位置重叠，已保留文件".into());
    }
    #[cfg(target_os = "macos")]
    {
        let cache = paths.cache_base();
        safe_path(&cache)?;
        if cache.exists()
            && (!shell_owned(&cache)? || protected.iter().any(|path| overlaps(path, &cache)))
        {
            return Err("缓存目录没有有效归属或与视频重叠，已保留全部文件".into());
        }
    }
    if !keep_data && paths.shell.exists() {
        cleanup_legacy_shell(paths, &protected, &mut kept)?;
    }
    for base in &roots {
        let managed = base.join("managed-v2");
        if !managed.exists() {
            continue;
        }
        if keep_data {
            for child in ["cache", "runtime"] {
                let p = managed.join(child);
                if p.exists() {
                    remove_private_tree(&p)?;
                }
            }
        } else {
            cleanup_legacy(base, &protected, &mut kept)?;
            remove_private_tree(&managed)?;
            let _ = fs::remove_dir(base); // Preserve legacy videos and unknown files.
        }
    }
    if paths.shell.exists() {
        if keep_data {
            for child in ["webview", "cli-cache", "runtime"] {
                let p = paths.shell.join(child);
                if p.exists() {
                    remove_private_tree(&p)?;
                }
            }
        } else {
            remove_private_tree(&paths.shell)?;
        }
    }
    #[cfg(target_os = "macos")]
    if paths.cache_base().exists() {
        remove_private_tree(&paths.cache_base())?;
    }
    let _ = fs::remove_dir(&paths.local);
    let _ = fs::remove_dir(&paths.roaming);
    Ok(
        json!({"ok":true,"settings_preserved":keep_data,"preserved_changed_legacy_files":kept,"media_preserved":true}),
    )
}

/// CLI preserves native byte streams. Environment changes apply to this child only.
pub fn run_cli(
    paths: &Paths,
    engine: &Path,
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<i32, String> {
    paths.initialize()?;
    let runtime = paths
        .cache_base()
        .join("runtime")
        .join(uuid::Uuid::new_v4().to_string());
    safe_path(&runtime)?;
    private_directory(&runtime)?;
    let _runtime_guard = SessionTemp {
        path: runtime.clone(),
    };
    // Native -U/--update-to must update this application's private CLI copy,
    // never the installer-owned executable or a global yt-dlp installation.
    let cli_directory = paths.shell.join("cli-engine");
    safe_path(&cli_directory)?;
    private_directory(&cli_directory)?;
    let executable = executable_path(&cli_directory, "yt-dlp");
    safe_path(&executable)?;
    if !executable.exists() {
        let staged = executable_path(&runtime, "yt-dlp");
        fs::copy(executable_path(engine, "yt-dlp"), &staged).map_err(error)?;
        if let Err(failure) = fs::rename(&staged, &executable) {
            if !executable.is_file() {
                let _ = remove_private_tree(&runtime);
                return Err(error(failure));
            }
            fs::remove_file(staged).map_err(error)?;
        }
    }
    let mut child = Command::new(executable);
    child
        .arg("--ignore-config")
        .arg("--cache-dir")
        .arg(paths.cache_base().join("cli-cache"))
        .arg("--ffmpeg-location")
        .arg(engine)
        .arg("--js-runtimes")
        .arg(format!(
            "node:{}",
            executable_path(engine, "node").display()
        ))
        .arg("--no-plugin-dirs")
        .arg("--plugin-dirs")
        .arg(engine.join("plugins"))
        .args(args)
        .env("TEMP", &runtime)
        .env("TMP", &runtime)
        .env("TMPDIR", &runtime)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("XDG_CACHE_HOME", paths.cache_base().join("cli-cache"))
        .env_remove("NODE_COMPILE_CACHE")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        child.process_group(0);
    }
    let result = (|| -> Result<std::process::ExitStatus, String> {
        #[cfg(unix)]
        let mut unix_job = UnixCliJob::prepare()?;
        let mut running = child.spawn().map_err(error)?;
        #[cfg(unix)]
        if let Err(error) = unix_job.attach(running.id()) {
            let _ = running.kill();
            let _ = running.wait();
            return Err(error);
        }
        #[cfg(windows)]
        let job = match CliJob::attach(&running) {
            Ok(job) => job,
            Err(error) => {
                let _ = running.kill();
                let _ = running.wait();
                return Err(error);
            }
        };
        #[cfg(unix)]
        let result = unix_job.wait(&mut running);
        #[cfg(not(unix))]
        let result = running.wait().map_err(error);
        #[cfg(windows)]
        drop(job);
        #[cfg(unix)]
        let result = {
            let restored = unix_job.finish();
            // Errors before waitpid reaps the leader must not leave a zombie.
            if result.is_err() {
                let _ = running.kill();
                let _ = running.wait();
            }
            result.and_then(|status| restored.map(|()| status))
        };
        result
    })();
    let cleanup_result = remove_private_tree(&runtime);
    match result {
        Ok(status) => {
            cleanup_result?;
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                Ok(status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)))
            }
            #[cfg(not(unix))]
            Ok(status.code().unwrap_or(1))
        }
        Err(e) => {
            let _ = cleanup_result;
            Err(e)
        }
    }
}

#[cfg(unix)]
static CLI_GROUP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
#[cfg(unix)]
static CLI_SIGNAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
#[cfg(unix)]
static CLI_JOB_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn forward_cli_signal(signal: libc::c_int) {
    use std::sync::atomic::Ordering;
    // Only async-signal-safe operations; the second interruption forces a stop.
    let signal = if CLI_SIGNAL.swap(signal, Ordering::SeqCst) == 0 {
        signal
    } else {
        libc::SIGKILL
    };
    CLI_SIGNAL.store(signal, Ordering::SeqCst);
    let group = CLI_GROUP.load(Ordering::SeqCst);
    if group > 1 {
        unsafe { libc::kill(-group, signal) };
    }
}

#[cfg(unix)]
struct UnixCliJob {
    group: libc::pid_t,
    previous: Vec<(libc::c_int, libc::sigaction)>,
    terminal: Option<UnixCliTerminal>,
}
#[cfg(unix)]
impl UnixCliJob {
    fn prepare() -> Result<Self, String> {
        use std::sync::atomic::Ordering;
        if CLI_JOB_ACTIVE
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("命令行入口已有运行中的引擎".into());
        }
        CLI_SIGNAL.store(0, Ordering::SeqCst);
        let mut job = Self {
            group: 0,
            previous: Vec::new(),
            terminal: None,
        };
        job.terminal = UnixCliTerminal::current()?;
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                let mut previous: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = forward_cli_signal as *const () as usize;
                libc::sigemptyset(&mut action.sa_mask);
                if libc::sigaction(signal, &action, &mut previous) != 0 {
                    return Err(format!(
                        "无法设置命令行退出处理：{}",
                        std::io::Error::last_os_error()
                    ));
                }
                job.previous.push((signal, previous));
            }
        }
        Ok(job)
    }
    fn attach(&mut self, pid: u32) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        let pid = libc::pid_t::try_from(pid).map_err(|_| "命令行进程编号无效")?;
        if pid <= 1 || unsafe { libc::getpgid(pid) } != pid {
            return Err("命令行引擎没有建立独立进程组".into());
        }
        self.group = pid;
        CLI_GROUP.store(pid, Ordering::SeqCst);
        if let Some(terminal) = &mut self.terminal {
            terminal.give_to(pid)?;
            // The child may already have attempted a terminal read before the
            // parent transfers the foreground group, stopping it with SIGTTIN.
            unsafe { libc::kill(-pid, libc::SIGCONT) };
        }
        let pending = CLI_SIGNAL.load(Ordering::SeqCst);
        if pending != 0 {
            unsafe { libc::kill(-pid, pending) };
        }
        Ok(())
    }

    fn wait(
        &mut self,
        child: &mut std::process::Child,
    ) -> Result<std::process::ExitStatus, String> {
        use std::os::unix::process::ExitStatusExt;
        loop {
            let mut status = 0;
            let waited = unsafe { libc::waitpid(self.group, &mut status, libc::WUNTRACED) };
            if waited < 0 {
                let failure = std::io::Error::last_os_error();
                if failure.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error(failure));
            }
            if libc::WIFSTOPPED(status) {
                if matches!(libc::WSTOPSIG(status), libc::SIGTTIN | libc::SIGTTOU)
                    && self
                        .terminal
                        .as_ref()
                        .is_some_and(|terminal| terminal.is_foreground(self.group))
                {
                    // A pre-handoff stop may still be pending in waitpid after
                    // SIGCONT. The engine now owns the tty and can simply retry.
                    unsafe { libc::kill(-self.group, libc::SIGCONT) };
                    continue;
                }
                if let Some(terminal) = &mut self.terminal {
                    terminal.capture_child_state();
                    terminal.reclaim()?;
                }
                if CLI_SIGNAL.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    // Let the invoking shell observe a stopped wrapper, so
                    // Ctrl-Z / fg work even though the engine has its own group.
                    unsafe { libc::kill(libc::getpid(), libc::SIGSTOP) };
                }
                if let Some(terminal) = &mut self.terminal {
                    terminal.give_to(self.group)?;
                }
                unsafe { libc::kill(-self.group, libc::SIGCONT) };
                continue;
            }
            if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                // waitpid has reaped this exact child; Child has no kill-on-drop.
                debug_assert_eq!(child.id(), self.group as u32);
                return Ok(std::process::ExitStatus::from_raw(status));
            }
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        CLI_GROUP.store(0, std::sync::atomic::Ordering::SeqCst);
        if self.group > 1 {
            unsafe { libc::kill(-self.group, libc::SIGKILL) };
            self.group = 0;
        }
        if let Some(terminal) = &mut self.terminal {
            terminal.reclaim()?;
        }
        Ok(())
    }
}
#[cfg(unix)]
impl Drop for UnixCliJob {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        let _ = self.finish();
        for (signal, action) in self.previous.iter().rev() {
            unsafe { libc::sigaction(*signal, action, std::ptr::null_mut()) };
        }
        CLI_JOB_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// The CLI owns the controlling terminal only while its caller gave it the
/// foreground. Never take a terminal away from another shell job.
#[cfg(unix)]
struct UnixCliTerminal {
    device: fs::File,
    parent_group: libc::pid_t,
    original: libc::termios,
    child_state: Option<libc::termios>,
    child_group: Option<libc::pid_t>,
}

#[cfg(unix)]
impl UnixCliTerminal {
    fn is_foreground(&self, group: libc::pid_t) -> bool {
        use std::os::fd::AsRawFd;
        unsafe { libc::tcgetpgrp(self.device.as_raw_fd()) == group }
    }

    fn current() -> Result<Option<Self>, String> {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        let device = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOCTTY)
            .open("/dev/tty")
        {
            Ok(device) => device,
            Err(failure)
                if matches!(
                    failure.raw_os_error(),
                    Some(libc::ENXIO | libc::ENODEV | libc::ENOTTY | libc::ENOENT)
                ) =>
            {
                return Ok(None);
            }
            Err(failure) => return Err(format!("无法打开命令行控制终端：{failure}")),
        };
        let parent_group = unsafe { libc::getpgrp() };
        if unsafe { libc::tcgetpgrp(device.as_raw_fd()) } < 0 {
            return Err(format!(
                "无法读取命令行终端前台进程组：{}",
                std::io::Error::last_os_error()
            ));
        }
        let mut original = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(device.as_raw_fd(), &mut original) } != 0 {
            return Err(format!(
                "无法保存命令行终端状态：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Some(Self {
            device,
            parent_group,
            original,
            child_state: None,
            child_group: None,
        }))
    }

    fn change(&self, group: libc::pid_t, attributes: Option<&libc::termios>) -> Result<(), String> {
        use std::os::fd::AsRawFd;
        unsafe {
            // The wrapper is a background group while its child owns the tty.
            // Block job-control signals just for this thread while reclaiming it.
            let mut blocked: libc::sigset_t = std::mem::zeroed();
            let mut previous: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut blocked);
            libc::sigaddset(&mut blocked, libc::SIGTTOU);
            libc::sigaddset(&mut blocked, libc::SIGTTIN);
            let masked = libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut previous);
            if masked != 0 {
                return Err(error(std::io::Error::from_raw_os_error(masked)));
            }
            let result = if libc::tcsetpgrp(self.device.as_raw_fd(), group) != 0 {
                Err(std::io::Error::last_os_error())
            } else if let Some(attributes) = attributes {
                if libc::tcsetattr(self.device.as_raw_fd(), libc::TCSANOW, attributes) != 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            };
            let restored =
                libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
            result.map_err(error)?;
            if restored != 0 {
                return Err(error(std::io::Error::from_raw_os_error(restored)));
            }
            Ok(())
        }
    }

    fn give_to(&mut self, group: libc::pid_t) -> Result<(), String> {
        use std::os::fd::AsRawFd;
        let foreground = unsafe { libc::tcgetpgrp(self.device.as_raw_fd()) };
        if foreground == self.parent_group || foreground == group {
            if self.child_group.is_none() && self.child_state.is_none() {
                // A job started with '&' may only receive its first foreground
                // ownership later via 'fg'. Save the terminal at that handoff.
                if unsafe { libc::tcgetattr(self.device.as_raw_fd(), &mut self.original) } != 0 {
                    return Err(error(std::io::Error::last_os_error()));
                }
            }
            // Mark first so Drop restores even if changing termios fails after
            // tcsetpgrp succeeded.
            self.child_group = Some(group);
            self.change(group, self.child_state.as_ref())?;
        }
        Ok(())
    }

    fn capture_child_state(&mut self) {
        use std::os::fd::AsRawFd;
        if self.child_group != Some(unsafe { libc::tcgetpgrp(self.device.as_raw_fd()) }) {
            return;
        }
        let mut attributes = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(self.device.as_raw_fd(), &mut attributes) } == 0 {
            self.child_state = Some(attributes);
        }
    }

    fn reclaim(&mut self) -> Result<(), String> {
        use std::os::fd::AsRawFd;
        if let Some(group) = self.child_group {
            let foreground = unsafe { libc::tcgetpgrp(self.device.as_raw_fd()) };
            if foreground == group || foreground == self.parent_group {
                self.change(self.parent_group, Some(&self.original))?;
            }
            self.child_group = None;
        }
        Ok(())
    }
}

#[cfg(windows)]
struct CliJob(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl CliJob {
    fn attach(child: &std::process::Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, System::JobObjects::*};
        unsafe {
            let job = Self(CreateJobObjectW(None, None).map_err(error)?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
            .map_err(error)?;
            AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle())).map_err(error)?;
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for CliJob {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Read-only installer preflight. NSIS may not traverse replaced resource junctions.
fn check_resource_paths(root: &Path) -> Result<(), String> {
    safe_path(root)?;
    if root.parent().is_none() || root.file_name().is_none() {
        return Err("安装位置不能是磁盘根目录".into());
    }
    for sensitive in [
        dirs::home_dir(),
        dirs::desktop_dir(),
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::video_dir(),
    ]
    .into_iter()
    .flatten()
    {
        if normalized(&sensitive) == normalized(root) {
            return Err("请选择专门的流光安装子目录，不要直接使用个人文件夹".into());
        }
    }
    for name in [
        "flow-desktop.exe",
        "flow-cli.exe",
        "flow.cmd",
        "uninstall.exe",
        "WebView2Loader.dll",
    ] {
        safe_path(&root.join(name))?;
    }
    fn walk(path: &Path) -> Result<(), String> {
        safe_path(path)?;
        if !path.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(path).map_err(error)? {
            let path = entry.map_err(error)?.path();
            safe_path(&path)?;
            if path.is_dir() {
                walk(&path)?;
            }
        }
        Ok(())
    }
    walk(&root.join("engine"))
}

#[cfg(not(target_os = "macos"))]
pub const NATIVE_RESOURCE_MANIFEST: &str =
    include_str!("../../../packaging/native-engine-manifest.json");
#[cfg(target_os = "macos")]
pub const NATIVE_RESOURCE_MANIFEST: &str = include_str!(env!("FLOW_MACOS_RESOURCE_MANIFEST"));

pub(crate) fn package_files() -> Result<Vec<String>, String> {
    let manifest: Value = serde_json::from_str(NATIVE_RESOURCE_MANIFEST).map_err(error)?;
    let mut files = manifest["files"]
        .as_array()
        .ok_or("应用资源清单无效")?
        .iter()
        .map(|row| {
            row["path"]
                .as_str()
                .map(|p| format!("engine/{p}"))
                .ok_or("应用资源路径无效".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    files.extend(
        [
            "flow-desktop.exe",
            "flow-cli.exe",
            "flow.cmd",
            "uninstall.exe",
            "engine/manifest.json",
            "engine/licenses/THIRD_PARTY_NOTICES.md",
            "engine/licenses/source-status.md",
        ]
        .map(str::to_string),
    );
    files.sort();
    files.dedup();
    Ok(files)
}

fn installation_ownership(root: &Path) -> Result<Value, String> {
    let marker = root.join(INSTALL_MARKER);
    if marker.exists() {
        let record = read_json(&marker)?;
        if record["app"] != APP_ID
            || record["schema"] != 1
            || !record["files"].is_object()
            || !record["files"]["flow-desktop.exe"].is_string()
        {
            return Err("安装目录的归属记录无效，已保留原文件".into());
        }
        return Ok(record["files"].clone());
    }
    let legacy: Value =
        serde_json::from_str(include_str!("../assets/legacy-1.0.0-files.json")).map_err(error)?;
    let known_legacy = ["flow-desktop.exe", "engine/manifest.json"]
        .iter()
        .all(|name| {
            root.join(name).is_file()
                && digest(&root.join(name))
                    .is_ok_and(|hash| legacy["files"][*name].as_str() == Some(hash.as_str()))
        });
    if known_legacy {
        Ok(legacy["files"].clone())
    } else {
        Ok(json!({}))
    }
}

pub fn check_install_root(root: &Path) -> Result<(), String> {
    check_resource_paths(root)?;
    let owned = match crate::installer_payload::pending_ownership(root)? {
        Some(pending) => pending,
        None => installation_ownership(root)?,
    };
    for relative in package_files()? {
        let path = relative_file(root, &relative)?;
        if !path.exists() {
            continue;
        }
        if !path.is_file() {
            return Err(format!(
                "安装目标已有非文件项目，已保留：{}",
                path.display()
            ));
        }
        let verified = match owned.get(&relative) {
            Some(Value::String(expected)) => digest(&path)? == *expected,
            // Original NSIS uninstaller bytes were generated during installation.
            Some(Value::Null) if relative == "uninstall.exe" => {
                let mut signature = [0u8; 2];
                fs::File::open(&path)
                    .map_err(error)?
                    .read_exact(&mut signature)
                    .map_err(error)?;
                signature == *b"MZ"
            }
            _ => false,
        };
        if !verified {
            return Err(format!(
                "安装目录已有未知或被修改的同名文件，请保留它并选择独立安装目录：{}",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Called only after the installer has copied its payload. Never adopts unknown filenames.
pub fn record_installation(root: &Path) -> Result<(), String> {
    check_install_root(root)?;
    let embedded: Value = serde_json::from_str(NATIVE_RESOURCE_MANIFEST).map_err(error)?;
    for row in embedded["files"].as_array().ok_or("应用资源清单无效")? {
        let relative = format!("engine/{}", row["path"].as_str().ok_or("应用资源路径无效")?);
        let path = relative_file(root, &relative)?;
        if row["sha256"].as_str() != Some(digest(&path)?.as_str()) {
            return Err(format!("安装资源校验失败：{relative}"));
        }
    }
    let mut files = serde_json::Map::new();
    for relative in package_files()? {
        let path = relative_file(root, &relative)?;
        if !path.is_file() {
            return Err(format!("安装文件缺失：{relative}"));
        }
        files.insert(relative, json!(digest(&path)?));
    }
    write_json(
        &root.join(INSTALL_MARKER),
        &json!({"schema":1,"app":APP_ID,"version":env!("CARGO_PKG_VERSION"),"files":files}),
    )?;
    crate::installer_payload::finish(root)
}

pub struct SessionTemp {
    path: PathBuf,
}

/// OS-owned toast history is cleared by application ID only; never enumerate other apps.
pub fn clear_notification_history() -> Result<(), String> {
    if isolation_active() {
        return Err("隔离模式不清理系统通知历史".into());
    }
    #[cfg(windows)]
    {
        use windows::{
            core::HSTRING,
            Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
            UI::Notifications::ToastNotificationManager,
        };
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(error)?;
        let result = ToastNotificationManager::History()
            .and_then(|history| history.ClearWithId(&HSTRING::from(APP_ID)))
            .map_err(error);
        unsafe {
            RoUninitialize();
        }
        result
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}
impl SessionTemp {
    pub fn create(paths: &Paths) -> Result<Self, String> {
        paths.initialize()?;
        let path = paths
            .cache_base()
            .join("runtime")
            .join(format!("gui-{}", uuid::Uuid::new_v4()));
        safe_path(&path)?;
        private_directory(&path)?;
        Ok(Self { path })
    }
    /// Called before application threads start; only this process and its children inherit it.
    pub fn apply_to_current_process(&self) {
        for name in ["TEMP", "TMP", "TMPDIR"] {
            std::env::set_var(name, &self.path);
        }
    }
}
impl Drop for SessionTemp {
    fn drop(&mut self) {
        let _ = remove_private_tree(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engine_lookup_does_not_depend_on_working_directory() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(
            engine_directory(&base.join("flow-cli")).unwrap(),
            base.join("engine")
        );
        assert_eq!(
            executable_path(&base, "node"),
            base.join(if cfg!(windows) { "node.exe" } else { "node" })
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_bundle_and_data_paths_keep_runtime_cache_and_media_separate() {
        let root = Path::new("/Applications/流光.app/Contents");
        assert_eq!(
            engine_directory(&root.join("MacOS/flow-cli")).unwrap(),
            root.join("Resources/engine")
        );
        let paths = Paths::for_locations(
            "/fixture/support".into(),
            "/fixture/caches".into(),
            "/fixture/support".into(),
            "/fixture/movies".into(),
        )
        .unwrap();
        assert_eq!(paths.shell, Path::new("/fixture/support/shell-v2"));
        let cache = paths.core_cache_root().unwrap();
        assert_eq!(
            cache.parent(),
            Some(Path::new("/fixture/caches/cache-v2/core"))
        );
        let alternate = Paths::for_locations(
            paths.roaming.clone(),
            paths.local.clone(),
            "/fixture/custom-data".into(),
            paths.downloads.clone(),
        )
        .unwrap();
        assert_ne!(paths.core_cache_root(), alternate.core_cache_root());
        assert_ne!(
            normalized(Path::new("/fixture/Media")),
            normalized(Path::new("/fixture/media"))
        );
    }
    #[test]
    fn isolation_overrides_must_be_complete() {
        assert!(validate_isolation_overrides([false, false, false]).is_ok());
        assert!(validate_isolation_overrides([true, true, true]).is_ok());
        for partial in [
            [true, false, false],
            [false, true, false],
            [false, false, true],
            [true, true, false],
            [true, false, true],
            [false, true, true],
        ] {
            assert!(validate_isolation_overrides(partial).is_err());
        }
    }
    struct TestDirectory(PathBuf);
    impl std::ops::Deref for TestDirectory {
        type Target = Path;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
    impl AsRef<Path> for TestDirectory {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let boundary =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-artifacts/lifecycle");
            if let Ok(boundary) = fs::canonicalize(boundary) {
                if self.0.starts_with(boundary) {
                    let _ = remove_private_tree(&self.0);
                }
            }
        }
    }
    fn fixture() -> (TestDirectory, Paths) {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-artifacts/lifecycle")
            .join(uuid::Uuid::new_v4().to_string());
        // Normalize the compile-time ../ before applying strict runtime checks.
        fs::create_dir_all(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let roaming = base.join("roaming").join(APP_ID);
        let local = base.join("local").join(APP_ID);
        let paths = Paths::for_locations(roaming.clone(), local, roaming, base.join("Videos/流光"))
            .unwrap();
        paths.initialize().unwrap();
        (TestDirectory(base), paths)
    }
    fn managed(base: &Path) -> PathBuf {
        let managed = base.join("managed-v2");
        fs::create_dir_all(&managed).unwrap();
        write_json(
            &managed.join(CORE_MARKER),
            &json!({"schema":2,"kind":"flow-managed-state","id":uuid::Uuid::new_v4().to_string()}),
        )
        .unwrap();
        managed
    }
    #[test]
    fn explicit_locations_require_absolute_separate_media_paths() {
        let (base, paths) = fixture();
        assert!(Paths::for_locations(
            paths.roaming.clone(),
            paths.local.clone(),
            paths.data_base.clone(),
            base.join("media")
        )
        .is_ok());
        assert!(Paths::for_locations(
            paths.roaming.clone(),
            paths.local.clone(),
            paths.data_base.clone(),
            paths.shell.join("videos")
        )
        .is_err());
        assert!(Paths::for_locations(
            PathBuf::from("relative"),
            paths.local.clone(),
            paths.data_base.clone(),
            base.join("media")
        )
        .is_err());
    }
    #[test]
    fn native_initialization_does_not_create_browser_cache() {
        let (_base, paths) = fixture();
        assert!(!paths.shell.join("webview").exists());
        assert!(paths.cache_base().join("cli-cache").is_dir());
        assert!(paths.cache_base().join("runtime").is_dir());
    }
    #[test]
    fn native_upgrade_removes_owned_browser_cache_but_preserves_cli() {
        let (_base, paths) = fixture();
        fs::create_dir_all(paths.shell.join("webview/Default/Cache")).unwrap();
        fs::write(
            paths.shell.join("webview/Default/Cache/item"),
            b"browser-cache",
        )
        .unwrap();
        fs::create_dir_all(paths.shell.join("cli-engine")).unwrap();
        fs::write(
            paths.shell.join("cli-engine/yt-dlp.exe"),
            b"private cli engine",
        )
        .unwrap();
        fs::write(paths.cache_base().join("cli-cache/item"), b"cli cache").unwrap();
        assert!(cleanup_obsolete_webview(&paths).unwrap());
        assert!(!paths.shell.join("webview").exists());
        assert_eq!(
            fs::read(paths.shell.join("cli-engine/yt-dlp.exe")).unwrap(),
            b"private cli engine"
        );
        assert_eq!(
            fs::read(paths.cache_base().join("cli-cache/item")).unwrap(),
            b"cli cache"
        );
        assert!(!cleanup_obsolete_webview(&paths).unwrap());
    }
    #[test]
    fn removes_owned_state_but_keeps_legacy_media_and_unknown_files() {
        let (_base, paths) = fixture();
        let owned = managed(&paths.data_base);
        fs::create_dir_all(paths.data_base.join("downloads")).unwrap();
        let video = paths.data_base.join("downloads/episode.custom");
        fs::write(&video, b"video bytes no extension guessing").unwrap();
        fs::write(paths.data_base.join("personal.txt"), b"user").unwrap();
        fs::write(owned.join("flow.sqlite3"), b"app state").unwrap();
        paths.register_data().unwrap();
        cleanup(&paths, false).unwrap();
        assert!(!owned.exists());
        assert!(!paths.shell.exists());
        assert_eq!(
            fs::read(video).unwrap(),
            b"video bytes no extension guessing"
        );
        assert!(paths.data_base.join("personal.txt").exists());
    }
    #[test]
    fn keep_data_still_removes_private_caches_and_temporary_files() {
        let (_base, paths) = fixture();
        let owned = managed(&paths.data_base);
        fs::create_dir_all(owned.join("cache")).unwrap();
        fs::write(owned.join("cache/item"), b"cache").unwrap();
        fs::write(owned.join("flow.sqlite3"), b"history").unwrap();
        paths.register_data().unwrap();
        cleanup(&paths, true).unwrap();
        assert!(owned.join("flow.sqlite3").exists());
        assert!(!owned.join("cache").exists());
        assert!(!paths.shell.join("webview").exists());
    }
    #[test]
    fn cleanup_rejects_historical_media_ancestors_and_descendants_before_deleting() {
        for scope in [
            "core-root",
            "core-parent",
            "core-child",
            "shell-root",
            "shell-parent",
            "shell-child",
        ] {
            let (_base, paths) = fixture();
            let owned = managed(&paths.data_base);
            fs::write(owned.join("flow.sqlite3"), b"keep state").unwrap();
            fs::write(paths.cache_base().join("cli-cache/item"), b"keep cache").unwrap();
            let media = match scope {
                "core-root" => owned.clone(),
                "core-parent" => paths.data_base.clone(),
                "core-child" => owned.join("videos"),
                "shell-root" => paths.shell.clone(),
                "shell-parent" => paths.shell.parent().unwrap().to_path_buf(),
                _ => paths.shell.join("videos"),
            };
            write_json(&owned.join("legacy-owned-files.json"), &json!({"schema":1,"legacy_root":paths.data_base,"files":[],"protected_media_roots":[media]})).unwrap();
            for keep in [false, true] {
                assert!(cleanup(&paths, keep).is_err(), "{scope}, keep_data={keep}");
                assert_eq!(fs::read(owned.join("flow.sqlite3")).unwrap(), b"keep state");
                assert_eq!(
                    fs::read(paths.cache_base().join("cli-cache/item")).unwrap(),
                    b"keep cache"
                );
                assert!(owned.join(CORE_MARKER).exists());
            }
        }
    }
    #[test]
    fn cleanup_protects_default_downloads_without_a_migration_record() {
        let (_base, mut paths) = fixture();
        let owned = managed(&paths.data_base);
        paths.downloads = paths.shell.join("videos");
        fs::create_dir_all(&paths.downloads).unwrap();
        let media = paths.downloads.join("episode.media");
        fs::write(&media, b"video bytes").unwrap();
        assert!(cleanup(&paths, false).is_err());
        assert_eq!(fs::read(media).unwrap(), b"video bytes");
        assert!(owned.join(CORE_MARKER).exists());
    }
    #[test]
    fn malformed_historic_media_paths_block_cleanup_without_touching_state() {
        let (_base, paths) = fixture();
        let owned = managed(&paths.data_base);
        for media in [
            json!(["relative/videos"]),
            json!([false]),
            json!("not an array"),
        ] {
            write_json(&owned.join("legacy-owned-files.json"), &json!({"schema":1,"legacy_root":paths.data_base,"files":[],"protected_media_roots":media})).unwrap();
            assert!(cleanup(&paths, false).is_err());
            assert!(owned.join(CORE_MARKER).exists());
            assert!(paths.shell.join(SHELL_MARKER).exists());
        }
    }
    #[test]
    fn refuses_unmarked_state_and_parent_traversal() {
        let (_base, paths) = fixture();
        fs::create_dir_all(paths.data_base.join("managed-v2")).unwrap();
        assert!(cleanup(&paths, false).is_err());
        let relative_jump = paths.data_base.join("..").join("videos");
        assert!(safe_path(&relative_jump).is_err());
        assert!(paths.shell.exists());
    }
    #[test]
    fn tracks_custom_state_without_deleting_the_custom_parent() {
        let (base, mut paths) = fixture();
        paths.data_base = base.join("custom-parent");
        let owned = managed(&paths.data_base);
        fs::write(paths.data_base.join("keep.media"), b"video").unwrap();
        paths.register_data().unwrap();
        cleanup(&paths, false).unwrap();
        assert!(!owned.exists());
        assert!(paths.data_base.join("keep.media").exists());
    }
    #[test]
    fn refuses_unknown_install_collisions_and_accepts_matching_owned_files() {
        let (base, _) = fixture();
        let root = base.join("install");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("personal.video"), b"video").unwrap();
        check_install_root(&root).unwrap();
        fs::write(root.join("flow.cmd"), b"personal file").unwrap();
        assert!(check_install_root(&root).is_err());
        assert_eq!(fs::read(root.join("flow.cmd")).unwrap(), b"personal file");
        fs::write(root.join("flow-desktop.exe"), b"owned application").unwrap();
        write_json(&root.join(INSTALL_MARKER), &json!({"schema":1,"app":APP_ID,"files":{
            "flow.cmd":digest(&root.join("flow.cmd")).unwrap(),"flow-desktop.exe":digest(&root.join("flow-desktop.exe")).unwrap()}})).unwrap();
        check_install_root(&root).unwrap();
        fs::write(root.join("flow.cmd"), b"changed personal file").unwrap();
        assert!(check_install_root(&root).is_err());
    }
    #[test]
    fn removes_only_unchanged_legacy_files_including_staged_engine() {
        let (_base, paths) = fixture();
        let owned = managed(&paths.data_base);
        fs::create_dir_all(paths.data_base.join("engine/staging")).unwrap();
        let staged = paths.data_base.join("engine/staging/yt-dlp.exe.old");
        fs::write(&staged, b"MZ old updater").unwrap();
        let db = paths.data_base.join("flow.sqlite3");
        fs::write(&db, b"old application database").unwrap();
        let record = json!({"schema":1,"legacy_root":paths.data_base,"files":[
            {"path":"engine/staging/yt-dlp.exe.old","role":"engine-binary","sha256":digest(&staged).unwrap(),"bytes":fs::metadata(&staged).unwrap().len()},
            {"path":"flow.sqlite3","role":"database","sha256":digest(&db).unwrap(),"bytes":fs::metadata(&db).unwrap().len()}]});
        write_json(&owned.join("legacy-owned-files.json"), &record).unwrap();
        paths.register_data().unwrap();
        fs::write(&db, b"changed user content").unwrap();
        let report = cleanup(&paths, false).unwrap();
        assert!(!staged.exists());
        assert_eq!(fs::read(db).unwrap(), b"changed user content");
        assert_eq!(
            report["preserved_changed_legacy_files"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn legacy_browser_cleanup_keeps_unrecognized_personal_files() {
        let (_base, paths) = fixture();
        managed(&paths.data_base);
        fs::create_dir_all(paths.local.join("EBWebView/Default/Cache")).unwrap();
        fs::write(paths.local.join("EBWebView/Local State"), b"{}").unwrap();
        fs::write(paths.local.join("EBWebView/Default/Preferences"), b"{}").unwrap();
        let cache = paths.local.join("EBWebView/Default/Cache/item");
        fs::write(&cache, b"cache").unwrap();
        let personal = paths.local.join("EBWebView/keep.original");
        fs::write(&personal, b"video").unwrap();
        paths.register_data().unwrap();
        cleanup(&paths, false).unwrap();
        assert!(!cache.exists());
        assert_eq!(fs::read(personal).unwrap(), b"video");
    }
    #[cfg(windows)]
    #[test]
    fn locked_root_keeps_ownership_proof_for_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_base, paths) = fixture();
        let owned = managed(&paths.data_base);
        paths.register_data().unwrap();
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x02000000)
            .open(&owned)
            .unwrap();
        assert!(cleanup(&paths, false).is_err());
        assert!(owned.join(CORE_MARKER).exists());
        drop(handle);
        cleanup(&paths, false).unwrap();
        assert!(!owned.exists());
    }
    #[cfg(windows)]
    #[test]
    fn closing_cli_job_stops_its_engine_process() {
        use std::{
            os::windows::process::CommandExt,
            time::{Duration, Instant},
        };
        let (base, _paths) = fixture();
        let node = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../engine/node.exe");
        let mut child = Command::new(node)
            .args(["--eval", "setTimeout(()=>{},30000)"])
            .current_dir(&base)
            .env("TEMP", &*base)
            .env("TMP", &*base)
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_COMPILE_CACHE")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let job = CliJob::attach(&child).unwrap();
        drop(job);
        let start = Instant::now();
        while child.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let exited = child.try_wait().unwrap().is_some();
        if !exited {
            let _ = child.kill();
            let _ = child.wait();
        }
        assert!(exited, "closing the wrapper job must terminate its engine");
    }
    #[cfg(windows)]
    #[test]
    fn junction_target_survives_cleanup_and_linked_roots_are_rejected() {
        let (base, paths) = fixture();
        let owned = managed(&paths.data_base);
        let outside = base.join("media-target");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.bin"), b"video").unwrap();
        let junction = owned.join("linked");
        let status = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(safe_path(&junction.join("keep.bin")).is_err());
        paths.register_data().unwrap();
        cleanup(&paths, false).unwrap();
        assert_eq!(fs::read(outside.join("keep.bin")).unwrap(), b"video");
    }
}
