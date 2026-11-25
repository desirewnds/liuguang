// SPDX-License-Identifier: Apache-2.0
//! Verified, restartable installation. Final paths are replaced only after a
//! complete same-directory temporary file has been flushed and checked.
use crate::lifecycle::{self, APP_ID};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub const PENDING: &str = ".flow-install-pending.json";

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn safe(path: &Path) -> Result<(), String> {
    lifecycle::safe_path(path)
}

fn child(root: &Path, name: &str) -> Result<PathBuf, String> {
    let relative = Path::new(name);
    if relative.is_absolute()
        || relative
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
        || name.contains(':')
        || name.contains('\\')
        || name.is_empty()
    {
        return Err("安装清单路径无效".into());
    }
    let path = root.join(relative);
    safe(&path)?;
    Ok(path)
}

fn hash(path: &Path) -> Result<String, String> {
    safe(path)?;
    let mut file = fs::File::open(path).map_err(error)?;
    let mut result = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(error)?;
        if count == 0 {
            break;
        }
        result.update(&buffer[..count]);
    }
    Ok(format!("{:x}", result.finalize()))
}

fn valid_hash(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
}

fn read_record(path: &Path) -> Result<Value, String> {
    safe(path)?;
    let metadata = fs::metadata(path).map_err(error)?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err("安装记录过大或格式无效".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)
}

pub(crate) fn pending_ownership(root: &Path) -> Result<Option<Value>, String> {
    let path = root.join(PENDING);
    safe(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let data = read_record(&path)?;
    if data["schema"] != 1
        || data["app"] != APP_ID
        || data["kind"] != "flow-pending-install"
        || data["version"] != env!("CARGO_PKG_VERSION")
        || data["transaction"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .is_none()
    {
        return Err("安装中断记录无法识别，请用同版本安装包修复".into());
    }
    let entries = data["files"].as_object().ok_or("安装中断清单无效")?;
    if entries.len() > 20000
        || !entries.contains_key("flow-cli.exe")
        || !entries.contains_key("flow-desktop.exe")
    {
        return Err("安装中断清单不完整".into());
    }
    for (name, value) in entries {
        child(root, name)?;
        if !valid_hash(value) {
            return Err("安装中断校验值无效".into());
        }
    }
    Ok(Some(data["files"].clone()))
}

fn scratch_path(root: &Path, name: &str, transaction: uuid::Uuid) -> Result<PathBuf, String> {
    let destination = child(root, name)?;
    let suffix = format!("{:x}", Sha256::digest(name.as_bytes()));
    let path = destination
        .parent()
        .ok_or("安装目标没有父目录")?
        .join(format!(".flow-install-{transaction}-{}.tmp", &suffix[..16]));
    safe(&path)?;
    Ok(path)
}

fn promote_new(source: &Path, destination: &Path) -> Result<(), String> {
    // Never replace an entry created after preflight. MoveFileEx without
    // REPLACE_EXISTING is atomic on the same volume and rejects collisions.
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH},
        };
        let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<_> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(error)
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(source, destination).map_err(error)?;
        fs::remove_file(source).map_err(error)
    }
}

fn atomic_file(
    root: &Path,
    name: &str,
    source: &Path,
    expected: &str,
    transaction: uuid::Uuid,
) -> Result<(), String> {
    let destination = child(root, name)?;
    let temporary = scratch_path(root, name, transaction)?;
    if destination.exists() {
        if !destination.is_file() || hash(&destination)? != expected {
            return Err(format!(
                "安装目标已被其他文件占用：{}",
                destination.display()
            ));
        }
        if temporary.exists() {
            fs::remove_file(&temporary).map_err(error)?;
        }
        return Ok(());
    }
    let parent = destination.parent().ok_or("安装目标没有父目录")?;
    fs::create_dir_all(parent).map_err(error)?;
    safe(parent)?;
    // The pending record owns this transaction-specific scratch name. A crash
    // can leave incomplete bytes here, never at the final executable path.
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(error)?;
    }
    let result = (|| {
        let mut input = fs::File::open(source).map_err(error)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(error)?;
        std::io::copy(&mut input, &mut output).map_err(error)?;
        output.sync_all().map_err(error)?;
        drop(output);
        if hash(&temporary)? != expected {
            return Err("安装文件复制校验失败".into());
        }
        safe(&destination)?;
        promote_new(&temporary, &destination)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn pending_record(
    root: &Path,
    files: &Map<String, Value>,
    transaction: uuid::Uuid,
) -> Result<(), String> {
    let data = json!({"schema":1,"app":APP_ID,"kind":"flow-pending-install",
                      "version":env!("CARGO_PKG_VERSION"),"transaction":transaction.to_string(),"files":files});
    let destination = root.join(PENDING);
    safe(&destination)?;
    let temp = root.join(format!(".flow-install-record-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(error)?;
        file.write_all(&serde_json::to_vec_pretty(&data).map_err(error)?)
            .map_err(error)?;
        file.sync_all().map_err(error)?;
        drop(file);
        promote_new(&temp, &destination)
    })();
    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// The caller is the package's extracted helper. Sources and their manifest live
/// in NSIS's private plugin directory, never in the selected installation root.
pub fn install(root: &Path, payload: &Path, manifest: &Path) -> Result<(), String> {
    safe(root)?;
    safe(payload)?;
    safe(manifest)?;
    let target = lifecycle::normalized(root);
    let staging = lifecycle::normalized(payload);
    if target == staging
        || target.starts_with(&(staging.clone() + "/"))
        || staging.starts_with(&(target + "/"))
    {
        return Err("安装暂存与目标目录必须分开".into());
    }
    let data = read_record(manifest)?;
    if data["owner"] != "org.flowdown.desktop.native-package.v1"
        || data["schema"] != 1
        || data["version"] != env!("CARGO_PKG_VERSION")
        || data["platform"] != "windows-x86_64"
    {
        return Err("安装载荷身份不匹配".into());
    }
    let mut files = Map::new();
    let rows = data["files"].as_array().ok_or("安装载荷清单无效")?;
    if rows.len() > 20000 {
        return Err("安装载荷清单条目过多".into());
    }
    for row in rows {
        let name = row["path"].as_str().ok_or("安装载荷路径无效")?;
        let source = child(payload, name)?;
        child(root, name)?;
        if !valid_hash(&row["sha256"])
            || files.insert(name.into(), row["sha256"].clone()).is_some()
            || fs::metadata(&source).map_err(error)?.len()
                != row["bytes"].as_u64().ok_or("安装载荷大小无效")?
            || hash(&source)? != row["sha256"].as_str().unwrap_or_default()
        {
            return Err(format!("安装载荷校验失败：{name}"));
        }
    }
    let expected: BTreeSet<_> = lifecycle::package_files()?
        .into_iter()
        .filter(|p| p != "uninstall.exe")
        .collect();
    if files.keys().cloned().collect::<BTreeSet<_>>() != expected {
        return Err("安装载荷不符合此版本的完整文件清单".into());
    }
    // NSIS generates its own uninstaller after extraction. Hash it before
    // adoption; the pending record never accepts an arbitrary MZ-only match.
    let uninstaller = child(payload, "uninstall.exe")?;
    let mut signature = [0_u8; 2];
    fs::File::open(&uninstaller)
        .map_err(error)?
        .read_exact(&mut signature)
        .map_err(error)?;
    if signature != *b"MZ" {
        return Err("卸载程序格式无效".into());
    }
    files.insert("uninstall.exe".into(), json!(hash(&uninstaller)?));
    apply_files(root, payload, &files)
}

fn apply_files(root: &Path, payload: &Path, files: &Map<String, Value>) -> Result<(), String> {
    lifecycle::check_install_root(root)?;
    let transaction = if let Some(pending) = pending_ownership(root)? {
        if pending != Value::Object(files.clone()) {
            return Err("中断安装与当前安装包不一致，请先使用原安装包修复".into());
        }
        let record = read_record(&root.join(PENDING))?;
        uuid::Uuid::parse_str(record["transaction"].as_str().ok_or("安装事务记录无效")?)
            .map_err(error)?
    } else {
        fs::create_dir_all(root).map_err(error)?;
        let transaction = uuid::Uuid::new_v4();
        pending_record(root, files, transaction)?;
        transaction
    };
    for name in ["flow-cli.exe", "uninstall.exe"] {
        atomic_file(
            root,
            name,
            &child(payload, name)?,
            files[name].as_str().ok_or("安装校验值无效")?,
            transaction,
        )?;
    }
    for (name, expected) in files {
        if name == "flow-cli.exe" || name == "uninstall.exe" {
            continue;
        }
        atomic_file(
            root,
            name,
            &child(payload, name)?,
            expected.as_str().ok_or("安装校验值无效")?,
            transaction,
        )?;
    }
    Ok(())
}

pub(crate) fn finish(root: &Path) -> Result<(), String> {
    if pending_ownership(root)?.is_some() {
        fs::remove_file(root.join(PENDING)).map_err(error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        payload: PathBuf,
        files: Map<String, Value>,
    }
    impl Fixture {
        fn new() -> Self {
            let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../test-artifacts/installer-payload");
            fs::create_dir_all(&parent).unwrap();
            let parent = fs::canonicalize(parent).unwrap();
            let base = parent.join(uuid::Uuid::new_v4().to_string());
            let root = base.join("install");
            let payload = base.join("payload");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&payload).unwrap();
            let mut files = Map::new();
            for name in ["flow-cli.exe", "flow-desktop.exe", "uninstall.exe"] {
                fs::write(payload.join(name), format!("MZ fixture {name}")).unwrap();
                files.insert(name.into(), json!(hash(&payload.join(name)).unwrap()));
            }
            fs::write(base.join("keep-video.mp4"), b"user media").unwrap();
            Self {
                base,
                root,
                payload,
                files,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let parent = fs::canonicalize(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../test-artifacts/installer-payload"),
            )
            .unwrap();
            assert_eq!(self.base.parent(), Some(parent.as_path()));
            fs::remove_dir_all(&self.base).unwrap();
        }
    }

    #[test]
    fn resumes_partial_copy_without_adopting_user_media() {
        let f = Fixture::new();
        let transaction = uuid::Uuid::new_v4();
        pending_record(&f.root, &f.files, transaction).unwrap();
        atomic_file(
            &f.root,
            "flow-cli.exe",
            &f.payload.join("flow-cli.exe"),
            f.files["flow-cli.exe"].as_str().unwrap(),
            transaction,
        )
        .unwrap();
        let partial = scratch_path(&f.root, "flow-desktop.exe", transaction).unwrap();
        fs::write(&partial, b"interrupted bytes").unwrap();
        assert!(!f.root.join("flow-desktop.exe").exists());
        apply_files(&f.root, &f.payload, &f.files).unwrap();
        assert!(!partial.exists());
        for (name, expected) in &f.files {
            assert_eq!(
                hash(&f.root.join(name)).unwrap(),
                expected.as_str().unwrap()
            );
        }
        assert!(pending_ownership(&f.root).unwrap().is_some());
        assert_eq!(
            fs::read(f.base.join("keep-video.mp4")).unwrap(),
            b"user media"
        );
    }

    #[test]
    fn unowned_same_named_file_is_preserved() {
        let f = Fixture::new();
        fs::write(f.root.join("flow-cli.exe"), b"personal data").unwrap();
        assert!(apply_files(&f.root, &f.payload, &f.files).is_err());
        assert!(!f.root.join(PENDING).exists());
        assert_eq!(
            fs::read(f.root.join("flow-cli.exe")).unwrap(),
            b"personal data"
        );
    }

    #[test]
    fn changed_owned_file_and_different_retry_payload_are_preserved() {
        let f = Fixture::new();
        pending_record(&f.root, &f.files, uuid::Uuid::new_v4()).unwrap();
        let mut different = f.files.clone();
        different.insert("flow-cli.exe".into(), json!("0".repeat(64)));
        assert!(apply_files(&f.root, &f.payload, &different).is_err());
        fs::write(f.root.join("flow-cli.exe"), b"changed user bytes").unwrap();
        assert!(apply_files(&f.root, &f.payload, &f.files).is_err());
        assert_eq!(
            fs::read(f.root.join("flow-cli.exe")).unwrap(),
            b"changed user bytes"
        );
    }

    #[test]
    fn failed_copy_leaves_no_partial_final_file_and_remains_retryable() {
        let f = Fixture::new();
        fs::write(f.payload.join("flow-desktop.exe"), b"bad source").unwrap();
        assert!(apply_files(&f.root, &f.payload, &f.files).is_err());
        assert!(!f.root.join("flow-desktop.exe").exists());
        assert!(pending_ownership(&f.root).unwrap().is_some());
        assert_eq!(
            fs::read(f.base.join("keep-video.mp4")).unwrap(),
            b"user media"
        );
        fs::write(
            f.payload.join("flow-desktop.exe"),
            b"MZ fixture flow-desktop.exe",
        )
        .unwrap();
        apply_files(&f.root, &f.payload, &f.files).unwrap();
    }

    #[test]
    fn atomic_promotion_never_replaces_a_late_collision() {
        let f = Fixture::new();
        let scratch = f.root.join("scratch");
        let final_path = f.root.join("late-user-file");
        fs::write(&scratch, b"app").unwrap();
        fs::write(&final_path, b"user").unwrap();
        assert!(promote_new(&scratch, &final_path).is_err());
        assert_eq!(fs::read(final_path).unwrap(), b"user");
    }
}
