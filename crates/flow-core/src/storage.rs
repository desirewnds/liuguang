// SPDX-License-Identifier: Apache-2.0
//! Storage ownership and output-path boundaries. Media is never owned by this module.
use crate::{config, err};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub(crate) const OWNER_FILE: &str = ".flow-owned-v2.json";
const RUN_OWNER: &str = ".flow-runtime-owner.json";
const CACHE_OWNER: &str = ".flow-cache-owner.json";

#[cfg(target_os = "macos")]
fn system_alias(path: &Path, metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    let expected = match path.to_str() {
        Some("/tmp") => "/private/tmp",
        Some("/var") => "/private/var",
        Some("/etc") => "/private/etc",
        _ => return false,
    };
    metadata.uid() == 0
        && std::fs::read_link(path).is_ok_and(|target| {
            let target = if target.is_absolute() {
                target
            } else {
                path.parent().unwrap_or(Path::new("/")).join(target)
            };
            target == Path::new(expected)
        })
}

#[derive(Clone)]
pub(crate) struct Storage {
    pub root: PathBuf,
    pub managed: PathBuf,
    pub cache: PathBuf,
    pub runtime: PathBuf,
    pub plugins: PathBuf,
    pub downloads: PathBuf,
    pub legacy_downloads: PathBuf,
    reserved: Vec<PathBuf>,
    owner: String,
}

pub(crate) fn ensure_no_links(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("路径必须为绝对路径".into());
    }
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let reparse = meta.file_type().is_symlink();
                if reparse || meta.file_type().is_symlink() {
                    #[cfg(target_os = "macos")]
                    if system_alias(ancestor, &meta) {
                        continue;
                    }
                    return Err(format!(
                        "目录包含符号链接或联接，请选择真实目录：{}",
                        ancestor.display()
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("无法检查目录 {}：{e}", ancestor.display())),
        }
    }
    Ok(())
}

fn lexical(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("请选择绝对路径".into());
    }
    ensure_no_links(path)?;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return Err("路径超出根目录".into());
                }
            }
            Component::Normal(s) => {
                #[cfg(windows)]
                {
                    let s = s.to_string_lossy();
                    if s.ends_with(['.', ' ']) || s.contains(':') {
                        return Err("路径不能含末尾空格、句点或备用数据流".into());
                    }
                }
                out.push(s);
            }
            _ => out.push(c.as_os_str()),
        }
    }
    ensure_no_links(&out)?;
    let mut ancestor = out.as_path();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        missing.push(ancestor.file_name().ok_or("无法解析路径")?.to_owned());
        ancestor = ancestor.parent().ok_or("无法解析路径")?;
    }
    let canonical = std::fs::canonicalize(ancestor).map_err(err)?;
    #[cfg(windows)]
    let canonical = {
        let text = canonical.to_string_lossy();
        if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
            PathBuf::from(format!("\\\\{unc}"))
        } else {
            PathBuf::from(text.strip_prefix("\\\\?\\").unwrap_or(&text))
        }
    };
    let mut result = canonical;
    for part in missing.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}
fn path_key(path: &Path) -> String {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy().replace('/', "\\");
        let value = value.strip_prefix("\\\\?\\").unwrap_or(&value);
        value.trim_end_matches('\\').to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().trim_end_matches('/').to_owned()
    }
}
fn within(path: &Path, root: &Path) -> bool {
    let p = path_key(path);
    let r = path_key(root);
    let separator = if cfg!(windows) { '\\' } else { '/' };
    if p == r || p.starts_with(&format!("{r}{separator}")) {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(root_metadata) = std::fs::metadata(root) {
            // APFS may be case sensitive or case insensitive. Compare actual
            // directory identity instead of lowercasing all user paths.
            return path.ancestors().any(|ancestor| {
                std::fs::metadata(ancestor).is_ok_and(|metadata| {
                    metadata.dev() == root_metadata.dev() && metadata.ino() == root_metadata.ino()
                })
            });
        }
    }
    false
}

impl Storage {
    #[cfg(test)]
    pub fn new(
        root: PathBuf,
        engine: &Path,
        downloads: PathBuf,
        extra: Vec<PathBuf>,
    ) -> Result<Self, String> {
        Self::new_with_cache_root(root, engine, downloads, extra, None)
    }
    pub fn new_with_cache_root(
        root: PathBuf,
        engine: &Path,
        downloads: PathBuf,
        extra: Vec<PathBuf>,
        cache_root: Option<PathBuf>,
    ) -> Result<Self, String> {
        let root = lexical(&root)?;
        let managed = root.join("managed-v2");
        ensure_no_links(&managed)?;
        let marker = managed.join(OWNER_FILE);
        ensure_no_links(&marker)?;
        let owner = if marker.exists() {
            let value: Value =
                serde_json::from_slice(&std::fs::read(&marker).map_err(err)?).map_err(err)?;
            if value["schema"] != 2
                || value["kind"] != "flow-managed-state"
                || uuid::Uuid::parse_str(value["id"].as_str().unwrap_or("")).is_err()
            {
                return Err("应用数据目录的所有权标记无效，未修改其内容".into());
            }
            value["id"].as_str().unwrap().to_owned()
        } else {
            if managed.exists() && std::fs::read_dir(&managed).map_err(err)?.next().is_some() {
                return Err("managed-v2 已存在且不属于流光，请选择其他数据目录".into());
            }
            create_private_directory(&managed)?;
            let owner = uuid::Uuid::new_v4().to_string();
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker)
                .map_err(err)?;
            f.write_all(
                json!({"schema":2,"kind":"flow-managed-state","id":owner})
                    .to_string()
                    .as_bytes(),
            )
            .map_err(err)?;
            owner
        };
        let mut reserved = vec![root.clone(), lexical(engine)?];
        for p in extra {
            reserved.push(lexical(&p)?);
        }
        let downloads = lexical(&downloads)?;
        let disposable = if let Some(cache_root) = cache_root {
            let cache_root = lexical(&cache_root)?;
            if within(&cache_root, &downloads) || within(&downloads, &cache_root) {
                return Err("视频目录与应用缓存目录必须分开".into());
            }
            if within(&root, &cache_root) || within(&cache_root, &root) {
                return Err("独立缓存目录不能与应用数据目录重叠".into());
            }
            ensure_no_links(&cache_root)?;
            let marker = cache_root.join(CACHE_OWNER);
            ensure_no_links(&marker)?;
            if marker.exists() {
                let record: Value =
                    serde_json::from_slice(&std::fs::read(&marker).map_err(err)?).map_err(err)?;
                if record["schema"] != 1
                    || record["kind"] != "flow-disposable-state"
                    || record["owner"] != owner
                    || record["data"] != json!(managed)
                {
                    return Err("缓存目录的所有权与应用数据不匹配，原文件已保留".into());
                }
            } else {
                if cache_root.exists()
                    && std::fs::read_dir(&cache_root)
                        .map_err(err)?
                        .next()
                        .is_some()
                {
                    return Err("缓存目录已有未识别文件，请使用其他目录".into());
                }
                create_private_directory(&cache_root)?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&marker)
                    .map_err(err)?;
                file.write_all(
                    json!({"schema":1,"kind":"flow-disposable-state",
                    "owner":owner,"data":managed})
                    .to_string()
                    .as_bytes(),
                )
                .map_err(err)?;
            }
            reserved.push(cache_root.clone());
            cache_root
        } else {
            managed.clone()
        };
        let result = Self {
            cache: disposable.join("cache"),
            runtime: disposable.join("runtime"),
            plugins: managed.join("plugins"),
            legacy_downloads: root.join("downloads"),
            managed,
            root,
            downloads,
            reserved,
            owner,
        };
        result.validate_directory(&result.downloads)?;
        for path in [&result.cache, &result.runtime, &result.plugins] {
            ensure_no_links(path)?;
            create_private_directory(path)?;
        }
        result.clean_stale_runs();
        Ok(result)
    }
    pub fn describe(&self, migration: &Value) -> Value {
        json!({"data_directory":self.managed,"cache_directory":self.cache,"temp_directory":self.runtime,"legacy_download_directory":self.legacy_downloads,"default_download_directory":self.downloads,"migration":migration})
    }
    pub fn validate_directory(&self, path: &Path) -> Result<PathBuf, String> {
        let path = lexical(path)?;
        if self.reserved.iter().any(|root| within(&path, root)) {
            return Err("保存位置属于应用、缓存或临时数据目录；请选择独立视频目录。旧视频保留在原处，可使用当前设置重试旧任务".into());
        }
        Ok(path)
    }
    pub fn owned_path(&self, path: &Path) -> Result<(), String> {
        let path = lexical(path)?;
        if !within(&path, &self.managed) {
            return Err("内部文件路径超出受管目录".into());
        }
        ensure_no_links(&path)
    }
    pub fn same_path(a: &Path, b: &Path) -> bool {
        let a = lexical(a).unwrap_or_else(|_| a.to_path_buf());
        let b = lexical(b).unwrap_or_else(|_| b.to_path_buf());
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::MetadataExt;
            if let (Ok(a_meta), Ok(b_meta)) = (std::fs::metadata(&a), std::fs::metadata(&b)) {
                if a_meta.dev() == b_meta.dev() && a_meta.ino() == b_meta.ino() {
                    return true;
                }
            }
        }
        path_key(&a) == path_key(&b)
    }
    pub fn private_run(&self) -> Result<RunDirectory, String> {
        ensure_no_links(&self.runtime)?;
        let token = uuid::Uuid::new_v4().to_string();
        let path = self.runtime.join(format!("run-{token}"));
        std::fs::create_dir(&path).map_err(err)?;
        std::fs::write(
            path.join(RUN_OWNER),
            json!({"schema":1,"kind":"flow-runtime","owner":self.owner,"token":token,"pid":std::process::id()}).to_string(),
        )
        .map_err(err)?;
        Ok(RunDirectory {
            path,
            runtime: self.runtime.clone(),
            owner: self.owner.clone(),
            token,
        })
    }
    fn clean_stale_runs(&self) {
        if let Ok(entries) = std::fs::read_dir(&self.runtime) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(token) = name.strip_prefix("run-") else {
                    continue;
                };
                if uuid::Uuid::parse_str(token).is_err() {
                    continue;
                }
                if ensure_no_links(&path).is_err() {
                    continue;
                }
                let marker_path = path.join(RUN_OWNER);
                if ensure_no_links(&marker_path).is_err() {
                    continue;
                }
                let marker = std::fs::read(marker_path)
                    .ok()
                    .and_then(|v| serde_json::from_slice::<Value>(&v).ok());
                let Some(pid) = marker
                    .as_ref()
                    .and_then(|v| v["pid"].as_u64())
                    .and_then(|p| u32::try_from(p).ok())
                else {
                    continue;
                };
                if creator_may_be_alive(pid) {
                    continue;
                }
                let run = RunDirectory {
                    path,
                    runtime: self.runtime.clone(),
                    owner: self.owner.clone(),
                    token: token.into(),
                };
                let _ = run.cleanup();
            }
        }
    }
    pub fn copy_bundled_plugins(&self, engine: &Path) -> Result<(), String> {
        let source = engine.join("plugins/flow/yt_dlp_plugins/extractor/bilibili_preview.py");
        if source.exists() {
            ensure_no_links(&source)?;
            let target = self
                .plugins
                .join("flow/yt_dlp_plugins/extractor/bilibili_preview.py");
            ensure_no_links(&target)?;
            std::fs::create_dir_all(target.parent().unwrap()).map_err(err)?;
            std::fs::copy(source, target).map_err(err)?;
        }
        Ok(())
    }
    fn output_path(&self, value: &str, base: &Path) -> Result<(), String> {
        if value == "-" {
            return Ok(());
        }
        // Environment expansion and arbitrary path-valued template fields cannot be bounded statically.
        if value.contains('$') || value.contains('~') {
            return Err("保存路径请使用明确的绝对路径或相对文件名，不使用环境变量或 ~".into());
        }
        let mut safe = String::new();
        let mut rest = value;
        while let Some(start) = rest.find("%(") {
            safe.push_str(&rest[..start]);
            let tail = &rest[start + 2..];
            let end = tail.find(')').ok_or("文件名模板缺少右括号")?;
            let field = &tail[..end];
            if field.contains("filepath") || field.contains("filename") {
                return Err("模板不能使用动态完整路径字段；请选择外部保存目录和文件名模板".into());
            }
            safe.push_str("flow-template");
            rest = &tail[end + 1..];
        }
        safe.push_str(rest);
        if safe.contains('%') {
            return Err("保存路径不能使用环境变量展开".into());
        }
        let path = PathBuf::from(safe);
        let full = if path.is_absolute() {
            path
        } else {
            base.join(path)
        };
        self.validate_directory(&full).map(|_| ())
    }
    pub fn validate_args(&self, args: &[String], directory: &Path) -> Result<(), String> {
        let catalog = config::catalog();
        let definitions: Vec<_> = catalog["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["options"].as_array().unwrap())
            .collect();
        let mut bases = vec![self.validate_directory(directory)?];
        let mut outputs = Vec::<String>::new();
        let mut direct = Vec::<String>::new();
        let mut i = 0;
        while i < args.len() {
            let raw = &args[i];
            if raw == "--" {
                break;
            }
            if !raw.starts_with('-') || raw == "-" {
                i += 1;
                continue;
            }
            let (name, attached) = if let Some((n, v)) = raw.split_once('=') {
                (n, Some(v.to_owned()))
            } else if raw.starts_with("-o") && raw != "-o" {
                ("-o", Some(raw[2..].to_owned()))
            } else if raw.starts_with("-P") && raw != "-P" {
                ("-P", Some(raw[2..].to_owned()))
            } else {
                (raw.as_str(), None)
            };
            let def = definitions
                .iter()
                .find(|d| {
                    d["aliases"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|a| a.as_str() == Some(name))
                })
                .ok_or_else(|| {
                    format!("桌面工具无法验证 {name} 的输出路径；请使用完整参数名或独立命令行")
                })?;
            let canonical = def["name"].as_str().unwrap();
            let nargs = def["nargs"].as_u64().unwrap_or(0) as usize;
            let mut values = Vec::new();
            if let Some(v) = attached {
                values.push(v);
            }
            let take = nargs.saturating_sub(values.len());
            if i + take >= args.len() {
                return Err(format!("{name} 缺少参数"));
            }
            values.extend(args.iter().skip(i + 1).take(take).cloned());
            i += 1 + take;
            match canonical {
                "--config-locations" | "--alias" => {
                    return Err(format!(
                        "{canonical} 可隐藏输出路径，桌面工具不载入；请在独立命令行运行"
                    ))
                }
                "--paths" => {
                    let value = typed_path(&values[0]);
                    if value.contains(['%', '$', '~']) {
                        return Err("保存目录不能使用模板或环境变量；请填写明确路径".into());
                    }
                    let path = PathBuf::from(value);
                    let path = if path.is_absolute() {
                        path
                    } else {
                        directory.join(path)
                    };
                    bases.push(self.validate_directory(&path)?);
                }
                "--output" => outputs.push(typed_path(&values[0]).into()),
                "--download-archive" | "--cookies" => direct.push(values[0].clone()),
                "--print-to-file" => {
                    direct.push(values.get(1).ok_or("--print-to-file 缺少文件名")?.clone())
                }
                "--cache-dir" => {
                    let target = PathBuf::from(&values[0]);
                    if !Self::same_path(&target, &self.cache.join("yt-dlp")) {
                        self.output_path(&values[0], directory)?;
                    }
                }
                "--plugin-dirs" => {
                    if values[0] == "default" || !Path::new(&values[0]).is_absolute() {
                        return Err("插件仅可使用明确指定的绝对目录，不搜索共享默认目录".into());
                    }
                    ensure_no_links(Path::new(&values[0]))?;
                }
                _ => {}
            }
        }
        for value in outputs {
            for base in &bases {
                self.output_path(&value, base)?;
            }
        }
        for value in direct {
            self.output_path(&value, directory)?;
        }
        Ok(())
    }
}
fn creator_may_be_alive(pid: u32) -> bool {
    if pid == 0 {
        return true;
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let mut code = 0;
        let result = GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
        result == 0 || code == 259
    }
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // ESRCH proves it is gone. EPERM and other failures preserve data.
        (unsafe { libc::kill(pid, 0) }) == 0
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(not(any(windows, unix)))]
    {
        true
    }
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(err)
}
fn typed_path(value: &str) -> &str {
    if let Some((prefix, path)) = value.split_once(':') {
        if prefix.len() != 1 && !prefix.contains(['\\', '/']) {
            return path;
        }
    }
    value
}

pub(crate) struct RunDirectory {
    pub path: PathBuf,
    runtime: PathBuf,
    owner: String,
    token: String,
}
impl RunDirectory {
    pub fn apply(&self, command: &mut tokio::process::Command, cache: &Path) {
        command
            .env("TEMP", &self.path)
            .env("TMP", &self.path)
            .env("TMPDIR", &self.path)
            .env("XDG_CACHE_HOME", cache)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PYTHONPYCACHEPREFIX", self.path.join("pycache"))
            .env("NODE_COMPILE_CACHE", self.path.join("node-cache"));
    }
    fn cleanup(&self) -> Result<(), String> {
        if !self.path.exists() {
            return Ok(());
        }
        if self.path.parent() != Some(self.runtime.as_path())
            || self.path.file_name().and_then(|n| n.to_str())
                != Some(format!("run-{}", self.token).as_str())
        {
            return Err("临时目录不属于本进程".into());
        }
        ensure_no_links(&self.path)?;
        ensure_no_links(&self.path.join(RUN_OWNER))?;
        let marker: Value =
            serde_json::from_slice(&std::fs::read(self.path.join(RUN_OWNER)).map_err(err)?)
                .map_err(err)?;
        if marker["owner"] != self.owner
            || marker["token"] != self.token
            || marker["kind"] != "flow-runtime"
        {
            return Err("临时目录所有权不匹配，保留文件".into());
        }
        fn inspect(path: &Path) -> Result<(), String> {
            ensure_no_links(path)?;
            if path.is_dir() {
                for e in std::fs::read_dir(path).map_err(err)? {
                    inspect(&e.map_err(err)?.path())?;
                }
            }
            Ok(())
        }
        inspect(&self.path)?;
        std::fs::remove_dir_all(&self.path).map_err(err)
    }
}
impl Drop for RunDirectory {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

pub(crate) fn fingerprint(path: &Path) -> Result<Value, String> {
    ensure_no_links(path)?;
    let mut f = std::fs::File::open(path).map_err(err)?;
    let bytes = f.metadata().map_err(err)?.len();
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = f.read(&mut buffer).map_err(err)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(json!({"bytes":bytes,"sha256":format!("{:x}",hash.finalize())}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Storage) {
        let d = crate::test_directory();
        let store = Storage::new(
            d.path().join("state"),
            &d.path().join("engine"),
            d.path().join("videos"),
            vec![d.path().join("webview")],
        )
        .unwrap();
        (d, store)
    }
    #[test]
    fn separate_cache_is_owned_and_cannot_be_selected_for_media() {
        let d = crate::test_directory();
        let cache = d.path().join("cache/core");
        let state = d.path().join("state");
        let videos = d.path().join("videos");
        let s = Storage::new_with_cache_root(
            state.clone(),
            &d.path().join("engine"),
            videos.clone(),
            vec![],
            Some(cache.clone()),
        )
        .unwrap();
        assert_eq!(s.cache, cache.join("cache"));
        assert!(!s.runtime.starts_with(&s.managed));
        assert!(s.validate_directory(&s.runtime.join("film")).is_err());
        let run = s.private_run().unwrap();
        let temporary = run.path.clone();
        drop(run);
        assert!(!temporary.exists());
        let owner = s.owner.clone();
        drop(s);
        let reopened = Storage::new_with_cache_root(
            state,
            &d.path().join("engine"),
            videos,
            vec![],
            Some(cache.clone()),
        )
        .unwrap();
        assert_eq!(reopened.owner, owner);
        assert!(Storage::new_with_cache_root(
            d.path().join("another-state"),
            &d.path().join("engine"),
            d.path().join("other-videos"),
            vec![],
            Some(cache)
        )
        .is_err());
    }
    #[test]
    fn separate_cache_refuses_foreign_files_and_video_overlap() {
        let d = crate::test_directory();
        let cache = d.path().join("foreign-cache");
        std::fs::create_dir(&cache).unwrap();
        std::fs::write(cache.join("keep"), b"user file").unwrap();
        assert!(Storage::new_with_cache_root(
            d.path().join("state"),
            &d.path().join("engine"),
            d.path().join("videos"),
            vec![],
            Some(cache.clone())
        )
        .is_err());
        assert_eq!(std::fs::read(cache.join("keep")).unwrap(), b"user file");
        assert!(Storage::new_with_cache_root(
            d.path().join("state"),
            &d.path().join("engine"),
            d.path().join("videos"),
            vec![],
            Some(d.path().join("videos/temp"))
        )
        .is_err());
    }
    #[cfg(unix)]
    #[test]
    fn unix_paths_preserve_case_and_literal_backslashes() {
        assert_ne!(
            path_key(Path::new("/case/Film")),
            path_key(Path::new("/case/film"))
        );
        assert!(!within(
            Path::new("/media\\private/output"),
            Path::new("/media/private")
        ));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_accepts_only_known_system_aliases() {
        ensure_no_links(Path::new("/var/tmp/flow-missing-fixture")).unwrap();
        let d = crate::test_directory();
        let link = d.path().join("custom-link");
        std::os::unix::fs::symlink("/private/tmp", &link).unwrap();
        assert!(ensure_no_links(&link.join("anything")).is_err());
    }
    #[test]
    fn disposable_directories_cannot_receive_media_via_any_output_form() {
        let (_d, s) = fixture();
        let forbidden = s
            .managed
            .join("runtime/film.mp4")
            .to_string_lossy()
            .into_owned();
        for args in [
            vec!["--output".into(), forbidden.clone()],
            vec![format!("--output={forbidden}")],
            vec![format!("-o{forbidden}")],
            vec!["-P".into(), format!("temp:{}", s.managed.display())],
            vec![format!("--paths=home:{}", s.managed.display())],
            vec!["--output".into(), format!("thumbnail:{forbidden}")],
            vec!["--download-archive".into(), forbidden.clone()],
            vec!["--print-to-file".into(), "%(title)s".into(), forbidden],
        ] {
            assert!(
                s.validate_args(&args, &s.downloads).is_err(),
                "accepted unsafe args {args:?}"
            );
        }
        for value in ["%TEMP%", "$TMPDIR", "~/AppData"] {
            assert!(s
                .validate_args(&["--paths".into(), value.into()], &s.downloads)
                .is_err());
        }
        assert!(s.validate_directory(&s.root.join("downloads")).is_err());
        assert!(s
            .validate_directory(&s.root.parent().unwrap().join("webview/output"))
            .is_err());
        assert!(s
            .validate_args(
                &["--config-locations".into(), "hidden.conf".into()],
                &s.downloads
            )
            .is_err());
    }
    #[test]
    fn explicit_external_outputs_and_caches_stay_user_owned() {
        let (d, s) = fixture();
        let args = vec![
            "--paths".into(),
            format!("home:{}", d.path().join("media").display()),
            "--output".into(),
            "%(uploader)s/%(title)s.%(ext)s".into(),
            "--cache-dir".into(),
            d.path().join("chosen-cache").to_string_lossy().into_owned(),
        ];
        s.validate_args(&args, &s.downloads).unwrap();
        std::fs::create_dir_all(&s.downloads).unwrap();
        let video = s.downloads.join("keep.sqlite3");
        std::fs::write(
            &video,
            b"user media, extension does not determine ownership",
        )
        .unwrap();
        let scope = s.private_run().unwrap();
        let temporary = scope.path.clone();
        std::fs::write(temporary.join("private.bin"), b"temporary").unwrap();
        drop(scope);
        assert!(!temporary.exists());
        assert!(video.exists());
    }
    #[test]
    fn cleanup_requires_exact_runtime_ownership_and_does_not_claim_foreign_directories() {
        let (d, s) = fixture();
        let scope = s.private_run().unwrap();
        let temporary = scope.path.clone();
        std::fs::write(temporary.join(RUN_OWNER), "{}").unwrap();
        std::fs::write(temporary.join("keep.mp4"), b"not safely owned").unwrap();
        drop(scope);
        s.clean_stale_runs();
        assert!(temporary.join("keep.mp4").exists());
        let root = d.path().join("foreign-state");
        std::fs::create_dir_all(root.join("managed-v2")).unwrap();
        let foreign = root.join("managed-v2/keep.dat");
        std::fs::write(&foreign, b"user file").unwrap();
        assert!(Storage::new(
            root,
            &d.path().join("engine"),
            d.path().join("more-videos"),
            vec![]
        )
        .is_err());
        assert!(foreign.exists());
    }
    #[test]
    fn stale_cleanup_preserves_live_creators_and_removes_only_proven_dead_runs() {
        let (_d, s) = fixture();
        let live = s.private_run().unwrap();
        std::fs::write(live.path.join("working.tmp"), b"active").unwrap();
        s.clean_stale_runs();
        assert!(live.path.join("working.tmp").exists());
        #[cfg(any(windows, unix))]
        {
            let stale = s.private_run().unwrap();
            let marker_path = stale.path.join(RUN_OWNER);
            let mut marker: Value =
                serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
            // This is a synthetic fixture PID, not a discovered user process.
            assert!(!creator_may_be_alive(u32::MAX));
            marker["pid"] = json!(u32::MAX);
            std::fs::write(marker_path, marker.to_string()).unwrap();
            s.clean_stale_runs();
            assert!(!stale.path.exists());
            assert!(live.path.exists());
        }
    }
    #[tokio::test]
    async fn aborting_a_job_future_destroys_only_its_private_temp() {
        let (_d, s) = fixture();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            let scope = s.private_run().unwrap();
            std::fs::write(scope.path.join("worker.tmp"), b"private").unwrap();
            tx.send(scope.path.clone()).unwrap();
            std::future::pending::<()>().await;
            drop(scope);
        });
        let path = rx.await.unwrap();
        assert!(path.exists());
        worker.abort();
        let _ = worker.await;
        assert!(!path.exists());
    }
    #[cfg(windows)]
    #[test]
    fn junctions_in_parents_and_cleanup_descendants_are_refused() {
        use std::os::windows::process::CommandExt;
        let (d, s) = fixture();
        let target = d.path().join("real-media");
        std::fs::create_dir(&target).unwrap();
        let video = target.join("keep.mp4");
        std::fs::write(&video, b"media").unwrap();
        let scope = s.private_run().unwrap();
        let path = scope.path.clone();
        let link = path.join("linked");
        let result = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(s.validate_directory(&link.join("sub/new.mp4")).is_err());
        assert!(s.validate_directory(&link.join("../escape.mp4")).is_err());
        drop(scope);
        assert!(video.exists());
        assert!(path.exists());
        std::fs::remove_dir(&link).unwrap();
    }
}
