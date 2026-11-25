// SPDX-License-Identifier: Apache-2.0
use crate::{
    config::{self, strval},
    err, media, Core,
};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

fn executable(dir: &std::path::Path, name: &str) -> PathBuf {
    dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    })
}
fn command(path: PathBuf) -> Command {
    let mut c = Command::new(path);
    c.kill_on_drop(true);
    #[cfg(windows)]
    c.creation_flags(0x08000000);
    // Assign the group before exec, rather than racing a running downloader.
    // FFmpeg and JavaScript children inherit this private group.
    #[cfg(unix)]
    c.process_group(0);
    c
}
#[cfg(unix)]
fn process_groups() -> &'static std::sync::Mutex<std::collections::HashSet<u32>> {
    static GROUPS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<u32>>> =
        std::sync::OnceLock::new();
    GROUPS.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}
#[cfg(windows)]
fn job_handles() -> &'static std::sync::Mutex<std::collections::HashMap<u32, isize>> {
    static JOBS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<u32, isize>>> =
        std::sync::OnceLock::new();
    JOBS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}
/// Closing the last job handle terminates every descendant, even after its parent exited.
struct ProcessTree {
    pid: u32,
}
struct ProcessRegistration<'a> {
    core: &'a Core,
    id: String,
    pid: u32,
}
impl Drop for ProcessRegistration<'_> {
    fn drop(&mut self) {
        let mut pids = self.core.pids.lock().unwrap();
        if pids.get(&self.id) == Some(&self.pid) {
            pids.remove(&self.id);
        }
    }
}
impl ProcessTree {
    fn attach(pid: u32) -> Result<Self, String> {
        #[cfg(unix)]
        {
            let native_pid = i32::try_from(pid).map_err(|_| "引擎进程编号无效")?;
            if native_pid <= 1 || unsafe { libc::getpgid(native_pid) } != native_pid {
                return Err("引擎没有建立独立进程组，已停止启动".into());
            }
            process_groups().lock().unwrap().insert(pid);
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, GetLastError},
                System::{
                    JobObjects::*,
                    Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
                },
            };
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(format!("无法创建进程组：{}", GetLastError()));
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                let e = GetLastError();
                CloseHandle(job);
                return Err(format!("无法配置进程组：{e}"));
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                let e = GetLastError();
                CloseHandle(job);
                return Err(format!("无法打开引擎进程：{e}"));
            }
            let ok = AssignProcessToJobObject(job, process);
            let e = GetLastError();
            CloseHandle(process);
            if ok == 0 {
                CloseHandle(job);
                return Err(format!("无法管理引擎进程树：{e}"));
            }
            job_handles().lock().unwrap().insert(pid, job as isize);
        }
        Ok(Self { pid })
    }
}
impl Drop for ProcessTree {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // Keep the registration until all remaining children have been
            // terminated, even when their original parent has already exited.
            let mut groups = process_groups().lock().unwrap();
            if groups.remove(&self.pid) {
                unsafe { libc::kill(-(self.pid as libc::pid_t), libc::SIGKILL) };
            }
        }
        #[cfg(windows)]
        if let Some(handle) = job_handles().lock().unwrap().remove(&self.pid) {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(handle as *mut _);
            }
        }
    }
}
pub(crate) fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        if let Some(handle) = job_handles().lock().unwrap().get(&pid).copied() {
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(handle as *mut _, 1);
            }
            return;
        }
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // Never signal an arbitrary/unregistered group or our own session.
        // Cancellation has the same forceful semantics as the Windows Job.
        let groups = process_groups().lock().unwrap();
        if groups.contains(&pid) {
            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        }
    }
}
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect::<String>()
        .trim_end_matches(['.', ' '])
        .to_owned()
}

fn section_output_template(template: String, args: &[String]) -> String {
    if args.iter().any(|arg| {
        arg.split_once('=').map(|(name, _)| name).unwrap_or(arg) == "--download-sections"
    }) {
        template.replace(".%(ext)s", " - %(section_start)s-%(section_end)s.%(ext)s")
    } else {
        template
    }
}

impl Core {
    fn isolated_command(
        &self,
        path: PathBuf,
    ) -> Result<(Command, crate::storage::RunDirectory), String> {
        crate::storage::ensure_no_links(&path)?;
        crate::storage::ensure_no_links(&self.storage.cache)?;
        let scope = self.storage.private_run()?;
        let mut cmd = command(path);
        scope.apply(&mut cmd, &self.storage.cache);
        Ok((cmd, scope))
    }
    async fn capture(
        &self,
        path: PathBuf,
        args: &[&str],
        timeout: u64,
    ) -> Result<std::process::Output, String> {
        let (mut cmd, _scope) = self.isolated_command(path)?;
        cmd.args(["--ignore-config", "--no-plugin-dirs", "--cache-dir"])
            .arg(self.storage.cache.join("yt-dlp"))
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd.spawn().map_err(err)?;
        let pid = child.id().ok_or("无法读取进程ID")?;
        let _tree = ProcessTree::attach(pid)?;
        let id = format!("helper-{}", uuid::Uuid::new_v4());
        self.pids.lock().unwrap().insert(id.clone(), pid);
        let _registered = ProcessRegistration {
            core: self,
            id: id.clone(),
            pid,
        };
        let result =
            tokio::time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await;
        self.pids.lock().unwrap().remove(&id);
        match result {
            Ok(output) => output.map_err(err),
            Err(_) => {
                kill_tree(pid);
                Err("引擎辅助进程超时".into())
            }
        }
    }
    fn base_args(&self, s: &Value) -> Vec<String> {
        let mut a = vec![
            "--ignore-config",
            "--no-colors",
            "--encoding",
            "utf-8",
            "--socket-timeout",
            "25",
            "--retries",
            "3",
            "--fragment-retries",
            "3",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        a.extend([
            "--no-plugin-dirs".into(),
            "--plugin-dirs".into(),
            self.storage.plugins.to_string_lossy().into_owned(),
            "--cache-dir".into(),
            self.storage
                .cache
                .join("yt-dlp")
                .to_string_lossy()
                .into_owned(),
        ]);
        a.extend([
            "--ffmpeg-location".into(),
            self.engine_dir.to_string_lossy().into_owned(),
        ]);
        let node = executable(&self.engine_dir, "node");
        if node.exists() {
            a.extend(["--js-runtimes".into(), format!("node:{}", node.display())]);
        }
        match strval(s, "proxy_mode") {
            "direct" => a.extend(["--proxy".into(), String::new()]),
            "custom" => a.extend(["--proxy".into(), strval(s, "proxy").into()]),
            _ => {}
        }
        if !strval(s, "cookie_file").is_empty() {
            a.extend(["--cookies".into(), strval(s, "cookie_file").into()]);
        } else if !strval(s, "cookie_browser").is_empty() {
            a.extend([
                "--cookies-from-browser".into(),
                strval(s, "cookie_browser").into(),
            ]);
        }
        a
    }
    pub(crate) async fn metadata(
        &self,
        url: &str,
        mode: &str,
        s: &Value,
        id: &str,
    ) -> Result<Value, String> {
        let _engine_guard = self.engine_gate.read().await;
        let mut args = self.base_args(s);
        args.extend(config::selection(s)?);
        args.extend(["--skip-download".into(), "--no-warnings".into()]);
        if mode == "video" {
            args.extend(["--no-playlist".into(), "--dump-single-json".into()]);
        } else {
            args.extend([
                "--yes-playlist".into(),
                "--flat-playlist".into(),
                "--lazy-playlist".into(),
                "--dump-json".into(),
            ]);
        }
        args.extend(["--".into(), media::metadata_url(url, mode)]);
        let exe = self.yt_dlp_path();
        let (mut cmd, _scope) = self.isolated_command(exe)?;
        let mut child = cmd
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("无法启动下载引擎：{e}"))?;
        let pid = child.id().unwrap_or(0);
        let _tree = ProcessTree::attach(pid)?;
        self.pids.lock().unwrap().insert(id.into(), pid);
        let _registered = ProcessRegistration {
            core: self,
            id: id.into(),
            pid,
        };
        if self.is_cancelled(id) {
            kill_tree(pid);
            self.pids.lock().unwrap().remove(id);
            return Err("解析已取消".into());
        }
        if mode != "video" {
            let stdout = child.stdout.take().ok_or("无法读取目录输出")?;
            let stderr = child.stderr.take().ok_or("无法读取目录错误")?;
            let errors = tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                let mut buffer = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    if buffer.len() > 65536 {
                        buffer.clear();
                    }
                    buffer.push_str(&line);
                    buffer.push('\n');
                }
                buffer
            });
            let mut entries = Vec::<Value>::new();
            let mut pending = Vec::<Value>::new();
            let mut last = Instant::now() - Duration::from_secs(1);
            let mut lines = BufReader::new(stdout).lines();
            let read_result=tokio::time::timeout(Duration::from_secs(1800),async {
                while let Some(line)=lines.next_line().await.map_err(err)? {
                    if self.is_cancelled(id) { return Err("解析已取消".to_owned()); }
                    let entry:Value=serde_json::from_str(&line).map_err(|e|format!("目录数据无效：{e}"))?;
                    pending.push(media::entry(&entry,url,Some(entries.len()+1)));
                    entries.push(entry);
                    if last.elapsed()>=Duration::from_millis(240) {
                        let first=&entries[0];
                        self.event("preview_progress",json!({"request_id":id,"loaded":entries.len(),"total":first["playlist_count"],"entries":pending,"title":first["playlist_title"],"is_playlist":!first["playlist"].is_null()||!first["playlist_id"].is_null(),"complete":false}));
                        pending.clear();last=Instant::now();
                    }
                }
                child.wait().await.map_err(err)
            }).await;
            if !matches!(&read_result, Ok(Ok(_))) {
                kill_tree(pid);
            }
            self.pids.lock().unwrap().remove(id);
            let diagnostic = errors.await.unwrap_or_default();
            if self.is_cancelled(id) {
                return Err("解析已取消".into());
            }
            let status = read_result.map_err(|_| "目录读取超时，可缩小范围后重试")??;
            if !status.success() {
                return Err(config::redact(&diagnostic, s));
            }
            if entries.is_empty() {
                return Err("没有读取到视频条目；请检查链接、筛选条件或登录状态".into());
            }
            if !pending.is_empty() {
                self.event("preview_progress",json!({"request_id":id,"loaded":entries.len(),"entries":pending,"complete":false}));
            }
            let first = &entries[0];
            if entries.len() == 1 && first["playlist_id"].is_null() && first["playlist"].is_null() {
                return Ok(entries.remove(0));
            }
            return Ok(
                json!({"_type":"playlist","id":first["playlist_id"],"title":first["playlist_title"].as_str().or(first["playlist"].as_str()).unwrap_or("合集"),"playlist_count":first["playlist_count"].as_u64().unwrap_or(entries.len()as u64),"entries":entries}),
            );
        }
        let output = tokio::time::timeout(Duration::from_secs(180), child.wait_with_output()).await;
        self.pids.lock().unwrap().remove(id);
        if self.is_cancelled(id) {
            return Err("解析已取消".into());
        }
        let out = match output {
            Ok(o) => o.map_err(err)?,
            Err(_) => {
                kill_tree(pid);
                return Err("解析超时。请检查网络、代理、登录状态，或缩小合集范围".into());
            }
        };
        if !out.status.success() {
            return Err(config::redact(&String::from_utf8_lossy(&out.stderr), s));
        }
        serde_json::from_slice(&out.stdout).map_err(|e| format!("引擎返回了无法识别的元数据：{e}"))
    }
    pub(crate) async fn engine_info(&self) -> Value {
        let exe = self.yt_dlp_path();
        let available = exe.exists();
        let version = match self.capture(exe.clone(), &["--version"], 8).await {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_owned(),
            _ => String::new(),
        };
        json!({"version":version,"path":exe,"available":available,"catalog_version":config::catalog()["version"],"ffmpeg":executable(&self.engine_dir,"ffmpeg").exists(),"ffprobe":executable(&self.engine_dir,"ffprobe").exists(),"js_runtime":executable(&self.engine_dir,"node").exists(),"can_rollback":self.data_dir.join("engine").join("yt-dlp.previous").exists(),"catalog_compatible":version==config::catalog()["version"].as_str().unwrap_or("")})
    }
    pub(crate) async fn engine_verify(&self) -> Result<Value, String> {
        let dir = self.engine_dir.clone();
        let managed = self.yt_dlp_path();
        tokio::task::spawn_blocking(move||{
            use sha2::{Digest,Sha256};use std::io::Read;
            let manifest:Value=serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).map_err(err)?).map_err(err)?;
            let mut results=Vec::new();
            for record in manifest["files"].as_array().ok_or("引擎清单无效")?{
                let relative=std::path::Path::new(record["path"].as_str().ok_or("资源路径无效")?);
                if relative.is_absolute()||relative.components().any(|p|!matches!(p,std::path::Component::Normal(_))){return Err("资源清单包含非法路径".to_owned());}
                let path=dir.join(relative);let result=(||->Result<bool,String>{let mut file=std::fs::File::open(&path).map_err(err)?;let mut digest=Sha256::new();let mut buffer=[0u8;65536];loop{let n=file.read(&mut buffer).map_err(err)?;if n==0{break;}digest.update(&buffer[..n]);}Ok(format!("{:x}",digest.finalize())==record["sha256"].as_str().unwrap_or(""))})();
                results.push(json!({"path":record["path"],"ok":result.as_ref().map(|v|*v).unwrap_or(false),"error":result.err()}));
            }
            Ok(json!({"ok":results.iter().all(|r|r["ok"]==true),"files":results,"active_engine":managed,"active_is_bundled":managed.parent()==Some(dir.as_path()),"message":"校验随安装包发布的资源；用户更新的引擎由更新器校验并单独通过协议验证"}))
        }).await.map_err(err)?
    }
    fn yt_dlp_path(&self) -> PathBuf {
        let managed = executable(&self.data_dir.join("engine"), "yt-dlp");
        if managed.exists() {
            managed
        } else {
            executable(&self.engine_dir, "yt-dlp")
        }
    }
    async fn verify_engine(&self, path: &std::path::Path) -> Result<(), String> {
        let version = self.capture(path.to_owned(), &["--version"], 15).await?;
        if !version.status.success() || version.stdout.is_empty() {
            return Err("引擎无法启动".into());
        }
        let help = self.capture(path.to_owned(), &["--help"], 15).await?;
        let text = String::from_utf8_lossy(&help.stdout);
        for name in [
            "--progress-template",
            "--dump-json",
            "--flat-playlist",
            "--lazy-playlist",
            "--js-runtimes",
            "--print",
        ] {
            if !help.status.success() || !text.contains(name) {
                return Err(format!("引擎缺少所需协议 {name}"));
            }
        }
        Ok(())
    }
    pub(crate) async fn engine_update(&self, rollback: bool) -> Result<Value, String> {
        let _engine_guard = self
            .engine_gate
            .try_write()
            .map_err(|_| "请等待所有任务和解析结束后更新引擎")?;
        if !self.pids.lock().unwrap().is_empty()
            || !self.active.lock().unwrap().is_empty()
            || self
                .tasks
                .lock()
                .unwrap()
                .values()
                .any(|t| t["status"] == "queued")
        {
            return Err("请等待所有任务和解析结束后更新引擎".into());
        }
        let dir = self.data_dir.join("engine");
        let staging = dir.join("staging");
        self.storage.owned_path(&staging)?;
        std::fs::create_dir_all(&staging).map_err(err)?;
        let current = self.yt_dlp_path();
        let destination = executable(&dir, "yt-dlp");
        let candidate = executable(&staging, "yt-dlp");
        let backup = dir.join("yt-dlp.previous");
        for path in [&candidate, &backup, &destination] {
            self.storage.owned_path(path)?;
        }
        crate::storage::ensure_no_links(&current)?;
        if rollback && !backup.exists() {
            return Err("没有可回退的引擎版本".into());
        }
        std::fs::copy(if rollback { &backup } else { &current }, &candidate)
            .map_err(|e| format!("无法暂存引擎：{e}"))?;
        if !rollback {
            let network = self.settings.lock().unwrap().clone();
            let (mut updater, _scope) = self.isolated_command(candidate.clone())?;
            updater.args([
                "--ignore-config",
                "--no-colors",
                "--encoding",
                "utf-8",
                "--socket-timeout",
                "25",
                "--update",
            ]);
            updater
                .args(["--no-plugin-dirs", "--cache-dir"])
                .arg(self.storage.cache.join("yt-dlp"));
            match strval(&network, "proxy_mode") {
                "direct" => {
                    updater.args(["--proxy", ""]);
                }
                "custom" => {
                    updater.args(["--proxy", strval(&network, "proxy")]);
                }
                _ => {}
            }
            let child = updater
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(err)?;
            let pid = child.id().unwrap_or(0);
            let _tree = ProcessTree::attach(pid)?;
            self.pids
                .lock()
                .unwrap()
                .insert("engine-update".into(), pid);
            let _registered = ProcessRegistration {
                core: self,
                id: "engine-update".into(),
                pid,
            };
            let result =
                tokio::time::timeout(Duration::from_secs(180), child.wait_with_output()).await;
            self.pids.lock().unwrap().remove("engine-update");
            match result {
                Ok(Ok(output)) if output.status.success() => {}
                Ok(Ok(output)) => {
                    let settings = self.settings.lock().unwrap().clone();
                    return Err(format!(
                        "引擎更新失败，原版本未改变：{}",
                        config::redact(&String::from_utf8_lossy(&output.stderr), &settings)
                    ));
                }
                other => {
                    kill_tree(pid);
                    return Err(format!(
                        "引擎更新失败，原版本未改变：{}",
                        match other {
                            Ok(Err(e)) => e.to_string(),
                            _ => "更新请求超过180秒".into(),
                        }
                    ));
                }
            }
        }
        self.verify_engine(&candidate)
            .await
            .map_err(|e| format!("新版本未通过兼容验证，原版本未改变：{e}"))?;
        // Promote only validated candidates. A crash between the two renames is recovered by Core::new.
        if destination.exists() {
            if backup.exists() {
                std::fs::remove_file(&backup).map_err(err)?;
            }
            std::fs::rename(&destination, &backup).map_err(err)?;
        } else {
            std::fs::copy(&current, &backup).map_err(err)?;
        }
        if let Err(e) = std::fs::rename(&candidate, &destination) {
            let restored = std::fs::copy(&backup, &destination);
            return Err(format!(
                "引擎切换失败：{e}；恢复原版本{}",
                if restored.is_ok() {
                    "成功"
                } else {
                    "失败，下次启动将重试恢复"
                }
            ));
        }
        self.metadata_cache.lock().unwrap().clear();
        Ok(
            json!({"message":if rollback{"已恢复上一版本"}else{"引擎更新完成；参数目录对应应用发布版本，新参数可在命令工具使用"},"engine":self.engine_info().await}),
        )
    }
    async fn probe(&self, path: &str, id: &str) -> Option<Value> {
        if self.is_cancelled(id) {
            return None;
        }
        let (mut probe, _scope) = self
            .isolated_command(executable(&self.engine_dir, "ffprobe"))
            .ok()?;
        let child = probe
            .args([
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
                path,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        let pid = child.id()?;
        let _tree = ProcessTree::attach(pid).ok()?;
        self.pids.lock().unwrap().insert(id.to_owned(), pid);
        let _registered = ProcessRegistration {
            core: self,
            id: id.to_owned(),
            pid,
        };
        if self.is_cancelled(id) {
            kill_tree(pid);
        }
        let result = tokio::time::timeout(Duration::from_secs(8), child.wait_with_output()).await;
        self.pids.lock().unwrap().remove(id);
        if result.is_err() {
            kill_tree(pid);
        }
        let o = result.ok()?.ok()?;
        let v: Value = serde_json::from_slice(&o.stdout).ok()?;
        if !o.status.success() {
            return None;
        }
        let stream = v["streams"]
            .as_array()?
            .iter()
            .find(|s| s["codec_type"] == "video")
            .or_else(|| {
                v["streams"]
                    .as_array()?
                    .iter()
                    .find(|s| s["codec_type"] == "audio")
            })?;
        let fps = stream["avg_frame_rate"]
            .as_str()
            .and_then(|s| s.split_once('/'))
            .and_then(|(a, b)| {
                let a = a.parse::<f64>().ok()?;
                let b = b.parse::<f64>().ok()?;
                if b > 0. {
                    Some(a / b)
                } else {
                    None
                }
            });
        Some(
            json!({"width":stream["width"],"height":stream["height"],"vcodec":if stream["codec_type"]=="video"{stream["codec_name"].clone()}else{json!("none")},"acodec":if stream["codec_type"]=="audio"{stream["codec_name"].clone()}else{Value::Null},"fps":fps,"duration":v["format"]["duration"],"size":v["format"]["size"],"verified":true}),
        )
    }
    pub(crate) async fn execute_task(&self, task: &Value) -> Result<(), String> {
        let id = strval(task, "id");
        let settings = &task["settings"];
        let directory = self
            .storage
            .validate_directory(std::path::Path::new(strval(settings, "directory")))?;
        self.storage
            .validate_args(&config::advanced_args(settings, "command")?, &directory)?;
        if self.is_cancelled(id) {
            return Ok(());
        }
        let native = task["command_args"].is_string();
        let mut native_args = if native {
            config::advanced_args(settings, "command")?
        } else {
            Vec::new()
        };
        if native {
            native_args.extend(config::split_args(strval(task, "command_args"))?);
        }
        if native {
            self.storage.validate_args(&native_args, &directory)?;
        }
        let updates_engine = native_args.iter().any(|s| {
            s == "-U" || s == "--update" || s == "--update-to" || s.starts_with("--update-to=")
        });
        let _write_guard = if updates_engine {
            Some(self.engine_gate.write().await)
        } else {
            None
        };
        let _read_guard = if !updates_engine {
            Some(self.engine_gate.read().await)
        } else {
            None
        };
        let mut executable = self.yt_dlp_path();
        if updates_engine {
            let stage = self.data_dir.join("engine").join("native-update");
            self.storage.owned_path(&stage)?;
            std::fs::create_dir_all(&stage).map_err(err)?;
            let candidate = stage.join(executable.file_name().ok_or("引擎路径无效")?);
            self.storage.owned_path(&candidate)?;
            std::fs::copy(&executable, &candidate).map_err(err)?;
            executable = candidate;
        }
        let mut args = self.base_args(settings);
        if native {
            args.extend(native_args);
        } else {
            std::fs::create_dir_all(strval(settings, "directory"))
                .map_err(|e| format!("无法创建保存目录：{e}"))?;
            let advanced = config::advanced_args(settings, "download")?;
            args.extend(config::selection_defaults(settings)?);
            let mut template = "%(title).160B [%(id)s].%(ext)s".to_owned();
            if let Some(index) = task["index"].as_u64() {
                if task["batch_id"].is_string() {
                    template = format!(
                        "{}/{index:03} - %(title).130B [%(id)s].%(ext)s",
                        safe_name(task["batch_title"].as_str().unwrap_or("合集"))
                    );
                }
            }
            template = section_output_template(template, &advanced);
            args.extend([
                "--continue".into(),
                "--no-overwrites".into(),
                "--windows-filenames".into(),
                "--trim-filenames".into(),
                "200".into(),
                "--paths".into(),
                strval(settings, "directory").into(),
                "--output".into(),
                template,
            ]);
            let format = strval(settings, "format");
            let explicit_container = advanced.iter().any(|a| {
                ["--recode-video", "--remux-video", "--merge-output-format"].contains(&a.as_str())
                    || [
                        "--recode-video=",
                        "--remux-video=",
                        "--merge-output-format=",
                    ]
                    .iter()
                    .any(|p| a.starts_with(p))
            });
            if ["mp3", "m4a", "flac", "wav", "opus"].contains(&format) {
                args.extend([
                    "--extract-audio".into(),
                    "--audio-format".into(),
                    format.into(),
                    "--audio-quality".into(),
                    "0".into(),
                ]);
            } else if ["mp4", "mkv", "webm"].contains(&format) && !explicit_container {
                args.extend([
                    "--merge-output-format".into(),
                    format.into(),
                    "--remux-video".into(),
                    format.into(),
                ]);
            }
            if settings["subtitles"] == true {
                args.extend([
                    "--write-subs".into(),
                    "--sub-langs".into(),
                    "zh.*,en.*".into(),
                ]);
            }
            if settings["thumbnail"] == true {
                args.push("--write-thumbnail".into());
            }
            args.extend(advanced);
            args.extend(["--no-playlist","--newline","--progress","--progress-delta","0.25","--no-simulate","--no-colors","--encoding","utf-8","--print","before_dl:FLOW_INFO:%(.{id,title,thumbnail,duration,width,height,fps,vcodec,acodec,filesize,filesize_approx,requested_formats,requested_downloads})j","--print","after_move:FLOW_FILE:%(filepath)j","--progress-template","download:FLOW_PROGRESS:%(progress)j","--progress-template","postprocess:FLOW_POST:%(progress.status)s","--no-quiet","--"].into_iter().map(str::to_owned));
            args.push(strval(task, "url").into());
        }
        std::fs::create_dir_all(&directory).map_err(err)?;
        let (mut cmd, _scope) = self.isolated_command(executable.clone())?;
        cmd.current_dir(&directory);
        let mut child = cmd
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("引擎启动失败：{e}"))?;
        let pid = child.id().unwrap_or(0);
        let _tree = ProcessTree::attach(pid)?;
        self.pids.lock().unwrap().insert(id.into(), pid);
        let _registered = ProcessRegistration {
            core: self,
            id: id.into(),
            pid,
        };
        if self.is_cancelled(id) {
            kill_tree(pid);
            let _ = child.wait().await;
            self.pids.lock().unwrap().remove(id);
            return Ok(());
        }
        let stdin = child.stdin.take();
        let input = task["command_stdin"].as_str().unwrap_or("").to_owned();
        let input_writer = tokio::spawn(async move {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(input.as_bytes()).await;
            }
        });
        let version = self.engine_info().await;
        self.update(
            id,
            json!({"status":"downloading","engine_version":version["version"]}),
            true,
        );
        let stdout = child.stdout.take().ok_or("无法读取引擎输出")?;
        let stderr = child.stderr.take().ok_or("无法读取引擎错误")?;
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(bool, String)>(100);
        let tx2 = tx.clone();
        let a = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send((true, line)).await.is_err() {
                    break;
                }
            }
        });
        let b = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx2.send((false, line)).await.is_err() {
                    break;
                }
            }
        });
        let log_path = self.data_dir.join("logs").join(format!("{id}.log"));
        uuid::Uuid::parse_str(id).map_err(|_| "旧任务标识无效，请重新添加任务")?;
        self.storage.owned_path(&log_path)?;
        self.storage
            .owned_path(&self.data_dir.join("logs").join(format!("{id}.stdout")))?;
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(err)?;
        let mut native_stdout = if native {
            Some(
                std::fs::File::create(self.data_dir.join("logs").join(format!("{id}.stdout")))
                    .map_err(err)?,
            )
        } else {
            None
        };
        let mut error = String::new();
        let mut log_bytes = log.metadata().map(|m| m.len()).unwrap_or(0);
        let mut log_settings = settings.clone();
        if native {
            let mut effective = config::split_args(strval(settings, "raw_args"))?;
            effective.extend(config::split_args(strval(task, "command_args"))?);
            log_settings["raw_args"] = json!(serde_json::to_string(&effective).map_err(err)?);
        }
        let mut output = None;
        let mut last = Instant::now() - Duration::from_secs(1);
        let mut stream_finished = 0u64;
        let mut stream_last = 0u64;
        let mut stream_bytes = std::collections::HashMap::<String, u64>::new();
        let mut estimated_total = None;
        let mut multiple_streams = false;
        let mut skipped = false;
        while let Some((is_stdout, line)) = rx.recv().await {
            if self.is_cancelled(id) {
                kill_tree(pid);
                break;
            }
            if is_stdout {
                if let Some(ref mut f) = native_stdout {
                    let _ = writeln!(f, "{line}");
                }
            }
            if let Some(raw) = line.strip_prefix("FLOW_INFO:") {
                if let Ok(info) = serde_json::from_str::<Value>(raw) {
                    estimated_total = media::size(&info).0;
                    multiple_streams = info["requested_formats"]
                        .as_array()
                        .map(|a| a.len() > 1)
                        .unwrap_or(false);
                    self.update(id,json!({"title":info["title"],"thumbnail":info["thumbnail"],"format_label":media::format_label(&info),"total":estimated_total}),true);
                }
            } else if let Some(raw) = line.strip_prefix("FLOW_FILE:") {
                if let Ok(path) = serde_json::from_str::<String>(raw) {
                    output = Some(path.clone());
                    self.update(id, json!({"output":path}), true);
                }
            } else if let Some(raw) = line.strip_prefix("FLOW_PROGRESS:") {
                if let Ok(p) = serde_json::from_str::<Value>(raw) {
                    let downloaded = p["downloaded_bytes"].as_u64().unwrap_or(0);
                    if downloaded < stream_last {
                        stream_finished += stream_last;
                    }
                    stream_last = downloaded;
                    let total = p["total_bytes"]
                        .as_u64()
                        .or_else(|| p["total_bytes_estimate"].as_u64());
                    let all_downloaded = if let Some(file) = p["filename"].as_str() {
                        stream_bytes.insert(file.to_owned(), downloaded);
                        stream_bytes
                            .values()
                            .fold(0u64, |a, b| a.saturating_add(*b))
                    } else {
                        stream_finished.saturating_add(downloaded)
                    };
                    let known = if multiple_streams {
                        estimated_total
                    } else {
                        estimated_total.or(total.map(|n| stream_finished + n))
                    };
                    if last.elapsed() > Duration::from_millis(240) || p["status"] == "finished" {
                        self.update(id,json!({"status":"downloading","downloaded":all_downloaded,"total":known,"progress":known.filter(|n|*n>0).map(|n|(all_downloaded as f64/n as f64*100.).min(100.)),"speed":p["speed"],"eta":p["eta"]}),false);
                        last = Instant::now();
                    }
                }
            } else if line.starts_with("FLOW_POST:")
                || line.starts_with("[Merger]")
                || line.starts_with("[ExtractAudio]")
                || line.starts_with("[VideoRemuxer]")
            {
                self.update(
                    id,
                    json!({"status":"processing","progress":null,"speed":0}),
                    true,
                );
            } else {
                let clean = config::redact(&line, &log_settings);
                if log_bytes > 8 * 1024 * 1024 {
                    let _ = log.set_len(0);
                    let _ = writeln!(log, "[较早诊断已清理；每项最多保留约 8 MiB 日志]");
                    log_bytes = 0;
                }
                let _ = writeln!(log, "{clean}");
                log_bytes += clean.len() as u64 + 1;
                if clean.contains("ERROR:") {
                    error = clean;
                }
                if line.contains("has already been downloaded")
                    || line.contains("already been recorded in the archive")
                    || line.contains("does not pass filter")
                {
                    skipped = true;
                }
            }
        }
        drop(rx);
        let _ = a.await;
        let _ = b.await;
        let _ = input_writer.await;
        let status = child.wait().await.map_err(err)?;
        self.pids.lock().unwrap().remove(id);
        if self.is_cancelled(id) {
            return Ok(());
        }
        if !status.success() {
            return Err(if error.is_empty() {
                format!("引擎退出代码 {:?}；请查看日志", status.code())
            } else {
                error
            });
        }
        if updates_engine {
            self.verify_engine(&executable).await?;
            self.metadata_cache.lock().unwrap().clear();
            if self.is_cancelled(id) {
                return Ok(());
            }
            let dir = self.data_dir.join("engine");
            let destination = dir.join(executable.file_name().ok_or("引擎路径无效")?);
            let backup = dir.join("yt-dlp.previous");
            self.storage.owned_path(&destination)?;
            self.storage.owned_path(&backup)?;
            if destination.exists() {
                if backup.exists() {
                    std::fs::remove_file(&backup).map_err(err)?;
                }
                std::fs::rename(&destination, &backup).map_err(err)?;
            } else {
                std::fs::copy(self.yt_dlp_path(), &backup).map_err(err)?;
            }
            if let Err(e) = std::fs::rename(&executable, &destination) {
                let _ = std::fs::copy(&backup, &destination);
                return Err(format!("新引擎切换失败：{e}"));
            }
        }
        let mut patch = json!({"status":if skipped{"skipped"}else{"completed"},"progress":100,"speed":0,"eta":0,"error":null});
        if let Some(ref path) = output {
            patch["output"] = json!(path);
            if let Ok(m) = std::fs::metadata(path) {
                patch["total"] = json!(m.len());
                patch["downloaded"] = json!(m.len());
            }
            if let Some(media) = self.probe(path, id).await {
                patch["format_label"] = json!(media::format_label(&media));
                patch["output_media"] = media;
            } else if !self.is_cancelled(id) {
                return Err(
                    "输出文件无法通过 FFprobe 验证；请查看后处理设置或文件是否已被移动".into(),
                );
            }
        }
        if !native && output.is_none() {
            patch["status"] = json!("skipped");
            patch["error"] = json!("操作已完成，未生成视频文件；请查看日志确认筛选或附属文件结果");
        }
        self.update(id, patch, true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_engine_command_uses_private_cache_temp_and_plugin_search() {
        let d = crate::test_directory();
        let c = Core::new_with_paths(
            d.path().join("state"),
            d.path().join("engine"),
            d.path().join("videos"),
            vec![],
            std::sync::Arc::new(|_| {}),
        )
        .unwrap();
        let (command, scope) = c
            .isolated_command(d.path().join("engine/yt-dlp.exe"))
            .unwrap();
        let env: std::collections::HashMap<_, _> = command
            .as_std()
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.unwrap().to_string_lossy().into_owned(),
                )
            })
            .collect();
        for key in ["TEMP", "TMP", "TMPDIR"] {
            assert_eq!(env[key], scope.path.to_string_lossy());
        }
        assert_eq!(env["XDG_CACHE_HOME"], c.storage.cache.to_string_lossy());
        assert_eq!(env["PYTHONDONTWRITEBYTECODE"], "1");
        let args = c.base_args(&config::defaults(d.path()));
        let cache = args.iter().position(|a| a == "--cache-dir").unwrap();
        assert_eq!(
            args[cache + 1],
            c.storage.cache.join("yt-dlp").to_string_lossy()
        );
        let plugins = args.iter().position(|a| a == "--plugin-dirs").unwrap();
        assert!(args[..plugins].contains(&"--no-plugin-dirs".into()));
        assert_eq!(args[plugins + 1], c.storage.plugins.to_string_lossy());
        assert!(!args.contains(&"default".into()));
    }

    #[test]
    fn raw_section_forms_produce_the_same_distinct_segment_names() {
        let template = "合集/007 - %(title)s [%(id)s].%(ext)s";
        let expected = "合集/007 - %(title)s [%(id)s] - %(section_start)s-%(section_end)s.%(ext)s";
        for raw in [
            "--download-sections *0-5 --download-sections *10-15",
            "--download-sections=*0-5 --download-sections=*10-15",
        ] {
            let mut settings = config::defaults(std::path::Path::new("."));
            settings["raw_args"] = json!(raw);
            let args = config::advanced_args(&settings, "download").unwrap();
            assert_eq!(section_output_template(template.into(), &args), expected);
        }
        let mut settings = config::defaults(std::path::Path::new("."));
        settings["advanced"] = json!({"--download-sections":["*0-5","*10-15"]});
        assert_eq!(
            section_output_template(
                template.into(),
                &config::advanced_args(&settings, "download").unwrap()
            ),
            expected
        );
        assert_eq!(section_output_template(template.into(), &[]), template);
    }
}
