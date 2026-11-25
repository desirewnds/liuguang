// SPDX-License-Identifier: Apache-2.0
//! macOS data leases and a one-shot, GUI-authorized cleanup helper.
//!
//! Every normal entry point holds shared leases before initializing state.
//! Cleanup waits for its GUI parent to exit, then locks every current/historical
//! scope exclusively. A locked descriptor is never trusted without checking the
//! named inode again: cleanup intentionally unlinks these files as its last act.
use crate::lifecycle::{self, Paths};
use std::{
    collections::BTreeSet,
    ffi::{c_void, CStr},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const LOCK_NAME: &str = ".flow-maintenance-v1.lock";
const NATIVE_LOCK: &str = ".native-instance.lock";
const READY: u8 = 0x46;
const TIMEOUT: Duration = Duration::from_secs(30);
pub const HELPER_FLAG: &str = "--flow-macos-cleanup-after-exit";

pub struct SharedGuard {
    _locks: Vec<Lease>,
}

struct Lease {
    path: PathBuf,
    file: File,
    native: bool,
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn canonical_scope(path: &Path) -> Result<PathBuf, String> {
    lifecycle::safe_path(path)?;
    let mut current = path;
    let mut missing = Vec::new();
    while !current.exists() {
        missing.push(current.file_name().ok_or("应用数据位置无效")?.to_owned());
        current = current.parent().ok_or("应用数据位置无效")?;
    }
    let mut root = fs::canonicalize(current).map_err(error)?;
    for part in missing.into_iter().rev() {
        root.push(part);
    }
    Ok(root)
}

fn scopes(paths: &Paths, extra: &BTreeSet<PathBuf>) -> Result<BTreeSet<PathBuf>, String> {
    [&paths.roaming, &paths.local, &paths.data_base]
        .into_iter()
        .chain(extra.iter())
        .map(|root| canonical_scope(root))
        .collect()
}

fn owned_file(metadata: &fs::Metadata, native: bool) -> bool {
    metadata.is_file()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.nlink() == 1
        && metadata.mode() & 0o077 == 0
        && metadata.len() <= u64::from(native)
}

impl Lease {
    fn open(path: PathBuf, exclusive: bool, native: bool) -> Result<Option<Self>, String> {
        lifecycle::safe_path(&path)?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        if !native {
            options.create(true);
        }
        let file = options.open(&path).map_err(error)?;
        if !owned_file(&file.metadata().map_err(error)?, native) {
            return Err(format!("无法验证应用锁文件，已保留：{}", path.display()));
        }
        if native && file.metadata().map_err(error)?.len() == 1 {
            let mut byte = [0];
            if file.read_at(&mut byte, 0).map_err(error)? != 1 || byte != [1] {
                return Err(format!("实例锁内容无法识别，已保留：{}", path.display()));
            }
        }
        let operation = if exclusive {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        if unsafe { libc::flock(file.as_raw_fd(), operation | libc::LOCK_NB) } != 0 {
            let failure = io::Error::last_os_error();
            if failure.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Ok(None);
            }
            return Err(error(failure));
        }
        let lease = Self { path, file, native };
        if !lease.same_inode(false)? {
            return Err("应用数据锁已更换，请重新启动流光后重试".into());
        }
        Ok(Some(lease))
    }

    fn same_inode(&self, missing_ok: bool) -> Result<bool, String> {
        lifecycle::safe_path(&self.path)?;
        let named = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == io::ErrorKind::NotFound && missing_ok => return Ok(true),
            Err(e) => return Err(error(e)),
        };
        let opened = self.file.metadata().map_err(error)?;
        Ok(owned_file(&named, self.native)
            && named.dev() == opened.dev()
            && named.ino() == opened.ino())
    }
}

fn take_scopes(roots: &BTreeSet<PathBuf>, exclusive: bool) -> Result<Option<Vec<Lease>>, String> {
    let mut locks = Vec::new();
    for root in roots {
        lifecycle::safe_path(root)?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)
            .map_err(error)?;
        let metadata = fs::symlink_metadata(root).map_err(error)?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
        {
            return Err(format!(
                "应用数据目录的所有者或权限不正确：{}",
                root.display()
            ));
        }
        match Lease::open(root.join(LOCK_NAME), exclusive, false)? {
            Some(lease) => locks.push(lease),
            None => return Ok(None), // Drop this entire attempt before retrying.
        }
    }
    Ok(Some(locks))
}

/// Hold until the process has stopped all application work and session cleanup.
pub fn acquire_shared(paths: &Paths) -> Result<SharedGuard, String> {
    let roots = scopes(paths, &BTreeSet::new())?;
    let locks = take_scopes(&roots, false)?.ok_or("流光正在清理应用数据，请稍后重新打开")?;
    Ok(SharedGuard { _locks: locks })
}

/// Start only after the GUI's explicit destructive-action confirmation.
/// No shell, temporary authorization file, global service or PATH lookup is used.
pub fn spawn_cleanup_after_exit(paths: &Paths) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(error)?;
    let helper = executable
        .parent()
        .ok_or("无法取得应用位置")?
        .join("flow-cli");
    lifecycle::safe_path(&helper)?;
    if !fs::symlink_metadata(&helper).map_err(error)?.is_file() {
        return Err("应用包中的清理工具不存在".into());
    }
    let nonce = uuid::Uuid::new_v4().to_string();
    let mut child = Command::new(&helper)
        .arg(HELPER_FLAG)
        .arg(std::process::id().to_string())
        .arg(&nonce)
        .arg(&paths.roaming)
        .arg(&paths.local)
        .arg(&paths.data_base)
        .arg(&paths.downloads)
        .current_dir(helper.parent().ok_or("无法取得清理工具位置")?)
        // The GUI's private session directory is about to be removed.
        .env_remove("TEMP")
        .env_remove("TMP")
        .env_remove("TMPDIR")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("NODE_COMPILE_CACHE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(error)?;
    let ready = (|| {
        let mut output = child.stdout.take().ok_or("清理工具没有响应通道")?;
        let mut event = libc::pollfd {
            fd: output.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let polled = unsafe { libc::poll(&mut event, 1, 5000) };
        if polled <= 0 {
            return Err("清理工具未及时启动，应用数据未删除".to_owned());
        }
        let mut response = [0];
        output.read_exact(&mut response).map_err(error)?;
        if response[0] != READY {
            return Err("清理工具响应无效".to_owned());
        }
        let mut input = child.stdin.take().ok_or("清理工具没有授权通道")?;
        input.write_all(nonce.as_bytes()).map_err(error)?;
        input.flush().map_err(error)?;
        Ok(())
    })();
    if ready.is_err() {
        // Only this newly spawned, not-yet-authorized helper is terminated.
        // Never terminate another GUI, CLI download, or unrelated process.
        let _ = child.kill();
        let _ = child.wait();
    }
    ready
}

#[link(name = "proc")]
unsafe extern "C" {
    fn proc_pidpath(pid: libc::c_int, buffer: *mut c_void, size: u32) -> libc::c_int;
}

fn authorize_helper(parent: u32, nonce: &str) -> Result<(), String> {
    if parent <= 1
        || parent != unsafe { libc::getppid() } as u32
        || uuid::Uuid::parse_str(nonce).is_err()
        || nonce.len() != 36
        || unsafe { libc::isatty(libc::STDIN_FILENO) } != 0
    {
        return Err("清理工具只能由流光设置中的确认操作启动".into());
    }
    let mut buffer = [0u8; 4096];
    let length = unsafe {
        proc_pidpath(
            parent as i32,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
        )
    };
    if length <= 0 {
        return Err("无法验证发起清理的应用进程".into());
    }
    let name = CStr::from_bytes_until_nul(&buffer).map_err(error)?;
    let parent_path = Path::new(std::ffi::OsStr::from_bytes(name.to_bytes()));
    let executable = std::env::current_exe().map_err(error)?;
    let expected = executable
        .parent()
        .ok_or("无法取得应用位置")?
        .join("flow-desktop");
    let actual = fs::metadata(parent_path).map_err(error)?;
    let expected = fs::metadata(expected).map_err(error)?;
    if actual.dev() != expected.dev() || actual.ino() != expected.ino() {
        return Err("清理请求不是由当前应用包的图形界面发起".into());
    }
    io::stdout().write_all(&[READY]).map_err(error)?;
    io::stdout().flush().map_err(error)?;
    let mut approval = [0u8; 36];
    io::stdin().read_exact(&mut approval).map_err(error)?;
    if approval.as_slice() != nonce.as_bytes() {
        return Err("清理请求未获得本次授权".into());
    }
    Ok(())
}

fn parent_gone(parent: u32) -> Result<bool, String> {
    let pid = libc::pid_t::try_from(parent).map_err(error)?;
    if unsafe { libc::kill(pid, 0) } == 0 {
        return Ok(false);
    }
    match io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Ok(true),
        Some(libc::EPERM) => Ok(false), // PID reuse is handled conservatively.
        _ => Err("无法确认原应用进程是否已经退出".into()),
    }
}

fn native_locks(
    paths: &Paths,
    registered: &BTreeSet<PathBuf>,
) -> Result<Option<Vec<Lease>>, String> {
    let mut names = BTreeSet::from([canonical_scope(&paths.shell)?.join(NATIVE_LOCK)]);
    for root in registered.iter().chain(std::iter::once(&paths.data_base)) {
        names.insert(canonical_scope(root)?.join(NATIVE_LOCK));
    }
    let mut locks = Vec::new();
    for path in names {
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(error(e)),
            Ok(_) => (),
        }
        match Lease::open(path, true, true)? {
            Some(lease) => locks.push(lease),
            None => return Ok(None),
        }
    }
    Ok(Some(locks))
}

fn ensure_locks_survive_cleanup(
    paths: &Paths,
    roots: &BTreeSet<PathBuf>,
    registered: &BTreeSet<PathBuf>,
) -> Result<(), String> {
    let mut removed = vec![
        canonical_scope(&paths.shell)?,
        canonical_scope(&paths.cache_base())?,
    ];
    for root in registered {
        removed.push(canonical_scope(&root.join("managed-v2"))?);
    }
    if roots
        .iter()
        .any(|root| removed.iter().any(|tree| root.starts_with(tree)))
    {
        return Err("应用数据目录互相嵌套，无法安全清理；已保留全部文件".into());
    }
    Ok(())
}

fn cleanup_when_idle(paths: &Paths, parent: u32) -> Result<Vec<String>, String> {
    let deadline = Instant::now() + TIMEOUT;
    let mut registered = BTreeSet::new();
    loop {
        if Instant::now() >= deadline {
            return Err(
                "流光或命令行任务仍在使用应用数据，未执行清理。请退出相关任务后重试。".into(),
            );
        }
        if !parent_gone(parent)? {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let roots = scopes(paths, &registered)?;
        if let Some(locks) = take_scopes(&roots, true)? {
            // The roaming lease now excludes registration writes. If another
            // profile added a root before this attempt, release and lock it too.
            let current = lifecycle::registered_roots(paths)?;
            if scopes(paths, &current)? != roots {
                registered = current;
                drop(locks);
                continue;
            }
            registered = current;
            ensure_locks_survive_cleanup(paths, &roots, &registered)?;
            if let Some(native) = native_locks(paths, &registered)? {
                // Earlier leases may have lost their named inode while later
                // scopes were being acquired. Check the complete set before
                // the first deletion, not only after cleanup.
                if !parent_gone(parent)? {
                    return Err("原应用进程编号仍在使用，已取消清理".into());
                }
                for lock in locks.iter().chain(&native) {
                    if !lock.same_inode(false)? {
                        return Err("应用数据锁已更换，未执行清理".into());
                    }
                }
                let report = lifecycle::cleanup(paths, false)?;
                let mut preserved: BTreeSet<String> = report["preserved_changed_legacy_files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect();
                // Validate every remaining named inode before unlinking any.
                for lock in locks.iter().chain(&native) {
                    if !lock.same_inode(lock.native)? {
                        return Err("应用锁文件已被替换，已保留剩余文件；请重新打开流光检查".into());
                    }
                }
                // From this point onward, only exact lock unlinks and empty
                // directory removal are allowed. Never recursively delete again.
                for lock in native.iter().chain(&locks) {
                    if lock.native {
                        match fs::symlink_metadata(&lock.path) {
                            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                            Err(e) => return Err(error(e)),
                            Ok(_) => (),
                        }
                    }
                    if !lock.same_inode(false)? {
                        return Err("应用锁已更换，已停止清理".into());
                    }
                    match fs::remove_file(&lock.path) {
                        Ok(()) => (),
                        Err(e) if e.kind() == io::ErrorKind::NotFound && lock.native => (),
                        Err(e) => return Err(error(e)),
                    }
                }
                for root in roots.iter().rev() {
                    if fs::remove_dir(root).is_err() && root.exists() {
                        preserved.insert(root.to_string_lossy().into_owned());
                    }
                }
                return Ok(preserved.into_iter().collect());
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Private CLI entry point. An interactive/manual invocation cannot authorize
/// itself: the verified sibling GUI must send this run's nonce through stdin.
pub fn run_cleanup_helper(paths: &Paths, parent: u32, nonce: &str) -> Result<(), String> {
    authorize_helper(parent, nonce)?;
    let result = cleanup_when_idle(paths, parent);
    let message = match &result {
        Ok(preserved) => {
            let mut message = "已清理可确认归属的流光数据；视频及无法确认归属的文件已保留。现在可以把应用移入废纸篓。".to_owned();
            if !preserved.is_empty() {
                message.push_str("\n以下位置包含视频、未知或已变更文件，已保留：\n");
                message.push_str(&preserved.iter().take(8).cloned().collect::<Vec<_>>().join("\n"));
                if preserved.len() > 8 { message.push_str(&format!("\n另有 {} 处", preserved.len() - 8)); }
            }
            message
        }
        Err(reason) => format!("未完成清理：{reason}\n已下载的视频不会被删除。若已清理部分应用数据，可重新打开流光后重试。"),
    };
    show_result(&message, result.is_ok());
    result.map(|_| ())
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithBytes(
        allocator: *const c_void,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> *const c_void;
    fn CFRelease(value: *const c_void);
    fn CFUserNotificationDisplayAlert(
        timeout: f64,
        flags: usize,
        icon: *const c_void,
        sound: *const c_void,
        localization: *const c_void,
        title: *const c_void,
        message: *const c_void,
        default_button: *const c_void,
        alternate: *const c_void,
        other: *const c_void,
        response: *mut usize,
    ) -> i32;
}

fn show_result(message: &str, success: bool) {
    let texts = ["流光", message, "好"];
    let strings: Vec<_> = texts
        .iter()
        .map(|text| unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                text.as_ptr(),
                text.len() as isize,
                0x08000100,
                0,
            )
        })
        .collect();
    if strings.iter().all(|value| !value.is_null()) {
        let mut response = 0;
        unsafe {
            CFUserNotificationDisplayAlert(
                0.0,
                usize::from(success),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                strings[0],
                strings[1],
                strings[2],
                std::ptr::null(),
                std::ptr::null(),
                &mut response,
            );
        }
    }
    for value in strings {
        if !value.is_null() {
            unsafe {
                CFRelease(value);
            }
        }
    }
}
