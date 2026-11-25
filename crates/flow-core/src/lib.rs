// SPDX-License-Identifier: Apache-2.0
mod config;
mod media;
mod process;
mod storage;

pub use config::{effective_override_names, effective_proxy};

use config::{defaults, normalize, strval};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Notify;

pub struct Core {
    pub(crate) data_dir: PathBuf,
    pub(crate) engine_dir: PathBuf,
    storage: storage::Storage,
    storage_migration: Value,
    db: Mutex<Connection>,
    pub(crate) tasks: Mutex<HashMap<String, Value>>,
    settings: Mutex<Value>,
    pub(crate) pids: Mutex<HashMap<String, u32>>,
    cancelled: Mutex<HashSet<String>>,
    emit: Arc<dyn Fn(Value) + Send + Sync>,
    started: AtomicBool,
    pub(crate) stopping: AtomicBool,
    wake: Notify,
    active: Mutex<HashSet<String>>,
    metadata_limit: Arc<tokio::sync::Semaphore>,
    submissions: Mutex<HashMap<String, Vec<String>>>,
    engine_gate: tokio::sync::RwLock<()>,
    queue_paused: AtomicBool,
    sequence: AtomicU64,
    metadata_cache: Mutex<HashMap<String, (std::time::Instant, Value)>>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Core {
    fn import_legacy_state(storage: &storage::Storage) -> Result<(), String> {
        let legacy = storage.root.join("flow.sqlite3");
        let destination = storage.managed.join("flow.sqlite3");
        storage::ensure_no_links(&destination)?;
        if destination.exists() || !legacy.exists() {
            return Ok(());
        }
        use std::io::Read;
        storage::ensure_no_links(&legacy)?;
        let mut header = [0u8; 16];
        std::fs::File::open(&legacy)
            .map_err(err)?
            .read_exact(&mut header)
            .map_err(err)?;
        if &header != b"SQLite format 3\0" {
            return Err("旧 flow.sqlite3 不是可识别数据库，未修改或删除任何旧文件".into());
        }
        let temporary = storage.private_run()?;
        let snapshot = temporary.path.join("legacy.sqlite3");
        let mut owned = Vec::new();
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let source = storage.root.join(format!("flow.sqlite3{suffix}"));
            if !source.exists() {
                continue;
            }
            let mut proof = storage::fingerprint(&source)?;
            if suffix != "-shm" {
                std::fs::copy(
                    &source,
                    temporary.path.join(format!("legacy.sqlite3{suffix}")),
                )
                .map_err(err)?;
                if storage::fingerprint(&source)? != proof {
                    return Err("旧数据库正在变化，请退出旧版后重试迁移".into());
                }
            }
            proof["path"] = json!(format!("flow.sqlite3{suffix}"));
            proof["role"] = json!(if suffix.is_empty() {
                "database"
            } else {
                "sqlite-sidecar"
            });
            owned.push(proof);
        }
        let source = Connection::open(&snapshot).map_err(err)?;
        let version: u32 = source
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(err)?;
        if version != 1 {
            return Err("旧数据库版本无法安全迁移，请保留原文件并使用对应版本".into());
        }
        for (table, expected) in [("kv", vec!["key", "value"]), ("tasks", vec!["id", "value"])] {
            let mut statement = source
                .prepare(&format!("PRAGMA table_info({table})"))
                .map_err(err)?;
            let columns: Vec<String> = statement
                .query_map([], |r| r.get(1))
                .map_err(err)?
                .collect::<Result<_, _>>()
                .map_err(err)?;
            if columns != expected {
                return Err("旧数据库结构无法确认，未修改原数据".into());
            }
        }
        let mut task_ids = Vec::new();
        let mut protected_media_roots = vec![storage.legacy_downloads.clone()];
        fn protect(paths: &mut Vec<PathBuf>, value: &Value) {
            if let Some(raw) = value.as_str() {
                let path = PathBuf::from(raw);
                if path.is_absolute() && !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
        let imported = temporary.path.join("imported.sqlite3");
        let mut db = Connection::open(&imported).map_err(err)?;
        db.execute_batch("CREATE TABLE kv(key TEXT PRIMARY KEY,value TEXT NOT NULL);CREATE TABLE tasks(id TEXT PRIMARY KEY,value TEXT NOT NULL);PRAGMA user_version=1;").map_err(err)?;
        {
            let tx = db.transaction().map_err(err)?;
            for (table, key) in [("kv", "key"), ("tasks", "id")] {
                let mut statement = source
                    .prepare(&format!("SELECT {key},value FROM {table}"))
                    .map_err(err)?;
                for row in statement
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(err)?
                {
                    let (id, value) = row.map_err(err)?;
                    let parsed: Value = serde_json::from_str(&value).map_err(err)?;
                    if table == "tasks" {
                        if !parsed.is_object() || parsed["id"] != id {
                            return Err("旧任务记录结构异常，原文件已保留".into());
                        }
                        if uuid::Uuid::parse_str(&id).is_ok() {
                            task_ids.push(id.clone());
                        }
                        protect(&mut protected_media_roots, &parsed["settings"]["directory"]);
                        if let Some(path) = parsed["output"].as_str().map(PathBuf::from) {
                            if path.is_absolute() {
                                if let Some(parent) = path.parent() {
                                    protect(&mut protected_media_roots, &json!(parent));
                                }
                            }
                        }
                    } else if id == "settings" {
                        protect(&mut protected_media_roots, &parsed["directory"]);
                    } else if id == "profiles" {
                        if let Some(profiles) = parsed.as_array() {
                            for profile in profiles {
                                protect(
                                    &mut protected_media_roots,
                                    &profile["settings"]["directory"],
                                );
                            }
                        }
                    }
                    tx.execute(
                        &format!("INSERT INTO {table}({key},value) VALUES(?1,?2)"),
                        params![id, value],
                    )
                    .map_err(err)?;
                }
            }
            tx.commit().map_err(err)?;
        }
        drop(db);
        drop(source);
        for id in task_ids {
            for extension in ["log", "stdout"] {
                let relative = format!("logs/{id}.{extension}");
                let path = storage.root.join(&relative);
                if !path.exists() {
                    continue;
                }
                storage::ensure_no_links(&path)?;
                if std::fs::metadata(&path).map_err(err)?.len() > 64 * 1024 * 1024
                    || std::fs::read_to_string(&path).is_err()
                {
                    continue;
                }
                let mut proof = storage::fingerprint(&path)?;
                let target = storage.managed.join(&relative);
                storage.owned_path(&target)?;
                std::fs::copy(&path, target).map_err(err)?;
                proof["path"] = json!(relative);
                proof["role"] = json!("task-log");
                owned.push(proof);
            }
        }
        for relative in [
            "engine/yt-dlp.exe",
            "engine/yt-dlp.previous",
            "engine/yt-dlp.exe.new",
            "engine/yt-dlp.exe.old",
            "engine/staging/yt-dlp.exe",
            "engine/staging/yt-dlp.exe.new",
            "engine/staging/yt-dlp.exe.old",
            "engine/native-update/yt-dlp.exe",
            "engine/native-update/yt-dlp.exe.new",
            "engine/native-update/yt-dlp.exe.old",
        ] {
            let path = storage.root.join(relative);
            if !path.is_file() {
                continue;
            }
            storage::ensure_no_links(&path)?;
            let mut signature = [0u8; 2];
            if std::fs::File::open(&path)
                .map_err(err)?
                .read_exact(&mut signature)
                .is_err()
                || &signature != b"MZ"
            {
                continue;
            }
            let mut proof = storage::fingerprint(&path)?;
            if relative == "engine/yt-dlp.exe" || relative == "engine/yt-dlp.previous" {
                let target = storage.managed.join(relative);
                storage.owned_path(&target)?;
                std::fs::create_dir_all(target.parent().unwrap()).map_err(err)?;
                std::fs::copy(&path, &target).map_err(err)?;
            }
            proof["path"] = json!(relative);
            proof["role"] = json!("engine-binary");
            owned.push(proof);
        }
        std::fs::rename(&imported, &destination).map_err(err)?;
        let manifest = storage.managed.join("legacy-owned-files.json");
        storage.owned_path(&manifest)?;
        std::fs::write(manifest,json!({"schema":1,"legacy_root":storage.root,"migrated_to":storage.managed,"files":owned,"protected_media_roots":protected_media_roots,"database_validation":{"sqlite_header":true,"user_version":1,"tables":["kv","tasks"]}}).to_string()).map_err(err)?;
        Ok(())
    }
    pub fn new(
        data_dir: PathBuf,
        engine_dir: PathBuf,
        emit: Arc<dyn Fn(Value) + Send + Sync>,
    ) -> Result<Arc<Self>, String> {
        let downloads = data_dir
            .parent()
            .ok_or("数据路径无父目录")?
            .join("流光下载");
        Self::new_with_paths(data_dir, engine_dir, downloads, Vec::new(), emit)
    }
    pub fn new_with_paths(
        data_root: PathBuf,
        engine_dir: PathBuf,
        default_download_dir: PathBuf,
        additional_reserved_roots: Vec<PathBuf>,
        emit: Arc<dyn Fn(Value) + Send + Sync>,
    ) -> Result<Arc<Self>, String> {
        Self::new_with_cache_root(
            data_root,
            engine_dir,
            default_download_dir,
            additional_reserved_roots,
            None,
            emit,
        )
    }

    /// Keep disposable state in a platform-specific private cache directory.
    /// None preserves the existing Windows and standalone-core layout.
    pub fn new_with_cache_root(
        data_root: PathBuf,
        engine_dir: PathBuf,
        default_download_dir: PathBuf,
        additional_reserved_roots: Vec<PathBuf>,
        cache_root: Option<PathBuf>,
        emit: Arc<dyn Fn(Value) + Send + Sync>,
    ) -> Result<Arc<Self>, String> {
        let storage = storage::Storage::new_with_cache_root(
            data_root,
            &engine_dir,
            default_download_dir,
            additional_reserved_roots,
            cache_root,
        )?;
        storage.copy_bundled_plugins(&engine_dir)?;
        let data_dir = storage.managed.clone();
        storage::ensure_no_links(&data_dir.join("logs"))?;
        std::fs::create_dir_all(data_dir.join("logs")).map_err(err)?;
        Self::import_legacy_state(&storage)?;
        let managed = data_dir.join("engine");
        storage::ensure_no_links(&managed)?;
        let managed_exe = managed.join(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        });
        if !managed_exe.exists() && managed.join("yt-dlp.previous").exists() {
            storage::ensure_no_links(&managed.join("yt-dlp.previous"))?;
            std::fs::copy(managed.join("yt-dlp.previous"), &managed_exe).map_err(err)?;
        }
        for suffix in ["", "-wal", "-shm", "-journal"] {
            storage::ensure_no_links(&data_dir.join(format!("flow.sqlite3{suffix}")))?;
        }
        let db = Connection::open(data_dir.join("flow.sqlite3")).map_err(err)?;
        let version: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(err)?;
        if version > 1 {
            return Err("此数据库由较新版本创建，请使用对应版本的流光".into());
        }
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY,value TEXT NOT NULL); PRAGMA user_version=1;").map_err(err)?;
        let mut settings: Value = db
            .query_row("SELECT value FROM kv WHERE key='settings'", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| {
                let mut s = defaults(&data_dir);
                s["directory"] = json!(storage.downloads);
                s
            });
        let mut storage_migration: Value = db
            .query_row(
                "SELECT value FROM kv WHERE key='storage_migration'",
                [],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        if storage::Storage::same_path(
            std::path::Path::new(strval(&settings, "directory")),
            &storage.legacy_downloads,
        ) {
            storage_migration = json!({"previous_directory":storage.legacy_downloads,"new_directory":storage.downloads,"files_preserved":true});
            settings["directory"] = json!(storage.downloads);
            db.execute(
                "INSERT OR REPLACE INTO kv(key,value) VALUES('settings',?1)",
                [settings.to_string()],
            )
            .map_err(err)?;
            db.execute(
                "INSERT OR REPLACE INTO kv(key,value) VALUES('storage_migration',?1)",
                [storage_migration.to_string()],
            )
            .map_err(err)?;
        }
        let mut tasks = HashMap::new();
        {
            let mut stmt = db.prepare("SELECT id,value FROM tasks").map_err(err)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(err)?;
            for row in rows {
                let (id, s) = row.map_err(err)?;
                let mut v: Value = serde_json::from_str(&s).map_err(err)?;
                if ["queued", "starting", "downloading", "processing"]
                    .contains(&strval(&v, "status"))
                {
                    v["status"] = json!("interrupted");
                    v["error"] = json!("程序已退出，任务未完成，可手动重试");
                }
                tasks.insert(id, v);
            }
        }
        let paused = db
            .query_row("SELECT value FROM kv WHERE key='queue_paused'", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
            .as_deref()
            == Some("true");
        let sequence = tasks
            .values()
            .filter_map(|t| t["sequence"].as_u64())
            .max()
            .unwrap_or(now())
            + 1;
        let core = Arc::new(Self {
            data_dir,
            engine_dir,
            storage,
            storage_migration,
            db: Mutex::new(db),
            tasks: Mutex::new(tasks),
            settings: Mutex::new(settings),
            pids: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashSet::new()),
            emit,
            started: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            wake: Notify::new(),
            active: Mutex::new(HashSet::new()),
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(2)),
            submissions: Mutex::new(HashMap::new()),
            engine_gate: tokio::sync::RwLock::new(()),
            queue_paused: AtomicBool::new(paused),
            sequence: AtomicU64::new(sequence),
            metadata_cache: Mutex::new(HashMap::new()),
        });
        for task in core.all_tasks() {
            core.persist_task(&task)?;
        }
        Ok(core)
    }
    pub fn storage_paths(&self) -> Value {
        self.storage.describe(&self.storage_migration)
    }
    pub(crate) fn event(&self, kind: &str, data: Value) {
        (self.emit)(json!({"kind":kind,"data":data}));
    }
    fn all_tasks(&self) -> Vec<Value> {
        let mut tasks: Vec<_> = self.tasks.lock().unwrap().values().cloned().collect();
        tasks.sort_by_key(|x| std::cmp::Reverse(x["created_at"].as_u64().unwrap_or(0)));
        tasks
    }
    fn persist_task(&self, task: &Value) -> Result<(), String> {
        self.db
            .lock()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO tasks(id,value) VALUES(?1,?2)",
                params![strval(task, "id"), task.to_string()],
            )
            .map_err(err)?;
        Ok(())
    }
    pub(crate) fn update(&self, id: &str, patch: Value, persist: bool) {
        let updated = {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(t) = tasks.get_mut(id) {
                // A late subprocess output may not resurrect a cancelled/interrupted task.
                if ["cancelled", "interrupted", "completed", "failed", "skipped"]
                    .contains(&strval(t, "status"))
                {
                    return;
                }
                if let Some(p) = patch.as_object() {
                    for (k, v) in p {
                        t[k] = v.clone();
                    }
                }
                if persist {
                    let _ = self.persist_task(t);
                }
                Some(t.clone())
            } else {
                None
            }
        };
        if let Some(t) = updated {
            self.event("task", t);
        }
    }
    pub(crate) fn settings_for(&self, incoming: &Value) -> Result<Value, String> {
        let empty = json!({});
        let mut settings = normalize(
            &self.settings.lock().unwrap(),
            if incoming.is_object() {
                incoming
            } else {
                &empty
            },
        )?;
        let directory = self
            .storage
            .validate_directory(std::path::Path::new(strval(&settings, "directory")))?;
        settings["directory"] = json!(directory);
        let args = config::advanced_args(&settings, "command")?;
        self.storage
            .validate_args(&args, std::path::Path::new(strval(&settings, "directory")))?;
        Ok(settings)
    }
    fn profiles(&self) -> Vec<Value> {
        self.db
            .lock()
            .unwrap()
            .query_row("SELECT value FROM kv WHERE key='profiles'", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save_kv(&self, k: &str, v: &Value) -> Result<(), String> {
        self.db
            .lock()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO kv(key,value) VALUES(?1,?2)",
                params![k, v.to_string()],
            )
            .map_err(err)?;
        Ok(())
    }
    pub(crate) fn is_cancelled(&self, id: &str) -> bool {
        self.stopping.load(Ordering::SeqCst) || self.cancelled.lock().unwrap().contains(id)
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.wake.notify_one();
        let pids: Vec<_> = self.pids.lock().unwrap().values().copied().collect();
        for pid in pids {
            process::kill_tree(pid);
        }
        let changed = {
            let mut ts = self.tasks.lock().unwrap();
            ts.values_mut()
                .filter_map(|t| {
                    if ["queued", "starting", "downloading", "processing"]
                        .contains(&strval(t, "status"))
                    {
                        t["status"] = json!("interrupted");
                        Some(t.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        for t in changed {
            let _ = self.persist_task(&t);
        }
    }
    fn ensure_scheduler(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let core = self.clone();
        tokio::spawn(async move {
            loop {
                if core.stopping.load(Ordering::SeqCst) {
                    break;
                }
                let limit = core.settings.lock().unwrap()["concurrency"]
                    .as_u64()
                    .unwrap_or(2) as usize;
                while core.active.lock().unwrap().len() < limit
                    && !core.queue_paused.load(Ordering::SeqCst)
                    && !core.stopping.load(Ordering::SeqCst)
                {
                    let next = {
                        let tasks = core.tasks.lock().unwrap();
                        tasks
                            .values()
                            .filter(|t| t["status"] == "queued")
                            .min_by_key(|t| {
                                (
                                    t["sequence"]
                                        .as_u64()
                                        .unwrap_or(t["created_at"].as_u64().unwrap_or(0)),
                                    t["index"].as_u64().unwrap_or(0),
                                )
                            })
                            .cloned()
                    };
                    let Some(task) = next else {
                        break;
                    };
                    let id = strval(&task, "id").to_owned();
                    core.active.lock().unwrap().insert(id.clone());
                    core.update(&id, json!({"status":"starting"}), true);
                    let worker = core.clone();
                    tokio::spawn(async move {
                        let result = worker.execute_task(&task).await;
                        if let Err(e) = result {
                            worker.update(&id, json!({"status":"failed","error":e}), true);
                        }
                        worker.active.lock().unwrap().remove(&id);
                        worker.wake.notify_one();
                    });
                }
                core.wake.notified().await;
            }
        });
    }

    pub async fn dispatch(self: &Arc<Self>, op: &str, p: Value) -> Result<Value, String> {
        self.ensure_scheduler();
        match op {
            "bootstrap" => {
                let settings = self.settings.lock().unwrap().clone();
                let engine = self.engine_info().await;
                Ok(
                    json!({"settings":settings,"tasks":self.all_tasks(),"profiles":self.profiles(),"engine":engine,"catalog":config::catalog(),"storage":self.storage_paths(),"queue":{"paused":self.queue_paused.load(Ordering::SeqCst)},"queue_paused":self.queue_paused.load(Ordering::SeqCst)}),
                )
            }
            "validate_download_directory" => {
                let path = self
                    .storage
                    .validate_directory(std::path::Path::new(strval(&p, "directory")))?;
                Ok(json!({"directory":path}))
            }
            "list_tasks" => Ok(json!(self.all_tasks())),
            "diagnostics" => {
                let processes: Vec<_> = self
                    .pids
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(id, pid)| json!({"task_id":id,"pid":pid}))
                    .collect();
                Ok(
                    json!({"processes":processes,"active_tasks":self.active.lock().unwrap().len(),"queue_paused":self.queue_paused.load(Ordering::SeqCst)}),
                )
            }
            "queue_state" => Ok(json!({"paused":self.queue_paused.load(Ordering::SeqCst)})),
            "queue_pause" | "queue_control" => {
                let paused = p["paused"].as_bool().ok_or("队列开关无效")?;
                self.save_kv("queue_paused", &json!(paused))?;
                self.queue_paused.store(paused, Ordering::SeqCst);
                self.wake.notify_one();
                let state = json!({"paused":paused});
                self.event("queue_state", state.clone());
                Ok(state)
            }
            "save_settings" => {
                let s = self.settings_for(&p["settings"])?;
                self.save_kv("settings", &s)?;
                *self.settings.lock().unwrap() = s.clone();
                self.event("settings", s.clone());
                self.wake.notify_one();
                Ok(s)
            }
            "validate_settings" => {
                let s = self.settings_for(&p["settings"])?;
                config::advanced_args(&s, p["context"].as_str().unwrap_or("download"))?;
                Ok(s)
            }
            "profiles_save" => {
                let name = strval(&p, "name").trim();
                if name.is_empty() {
                    return Err("请输入预设名称".into());
                }
                let s = self.settings_for(&p["settings"])?;
                let mut rows = self.profiles();
                if let Some(row) = rows.iter_mut().find(|r| r["name"] == name) {
                    row["settings"] = s;
                } else {
                    rows.push(json!({"id":uid(),"name":name,"settings":s}));
                }
                self.save_kv("profiles", &json!(rows))?;
                Ok(json!(rows))
            }
            "profiles_delete" => {
                let rows: Vec<_> = self
                    .profiles()
                    .into_iter()
                    .filter(|r| r["id"] != p["id"])
                    .collect();
                self.save_kv("profiles", &json!(rows))?;
                Ok(json!(rows))
            }
            "preview" | "estimate" => {
                let url = strval(&p, "url");
                media::valid_url(url)?;
                let s = self.settings_for(&p["settings"])?;
                let request = p["request_id"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(uid);
                self.cancelled.lock().unwrap().remove(&request);
                let _permit = self
                    .metadata_limit
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(err)?;
                if self.is_cancelled(&request) {
                    return Err("解析已取消".into());
                }
                let mode = if op == "estimate" {
                    "video"
                } else {
                    p["mode"].as_str().unwrap_or("auto")
                };
                if !["auto", "video", "playlist"].contains(&mode) {
                    return Err("无效的解析范围".into());
                }
                use sha2::{Digest, Sha256};
                let key = format!(
                    "{url}|{mode}|{:x}",
                    Sha256::digest(s.to_string().as_bytes())
                );
                let cached = if p["refresh"] == true {
                    None
                } else {
                    self.metadata_cache
                        .lock()
                        .unwrap()
                        .get(&key)
                        .filter(|(created, _)| {
                            created.elapsed() < std::time::Duration::from_secs(60)
                        })
                        .map(|(_, info)| info.clone())
                };
                let info = if let Some(info) = cached {
                    info
                } else {
                    let info = self.metadata(url, mode, &s, &request).await?;
                    if info.to_string().len() < 4 * 1024 * 1024 {
                        let mut cache = self.metadata_cache.lock().unwrap();
                        cache.retain(|_, (created, _)| {
                            created.elapsed() < std::time::Duration::from_secs(60)
                        });
                        if cache.len() >= 16 {
                            if let Some(oldest) = cache
                                .iter()
                                .min_by_key(|(_, v)| v.0)
                                .map(|(k, _)| k.clone())
                            {
                                cache.remove(&oldest);
                            }
                        }
                        cache.insert(key, (std::time::Instant::now(), info.clone()));
                    }
                    info
                };
                if op == "estimate" {
                    let (mut n, a) = media::size(&info);
                    let extra = config::advanced_args(&s, "download")?;
                    if extra.iter().any(|a| {
                        a == "--download-sections" || a.starts_with("--download-sections=")
                    }) {
                        n = None;
                    }
                    Ok(
                        json!({"size":n,"approximate":a,"disk_estimate":n.map(|n|n.saturating_mul(2)),"disk_estimate_approximate":true,"format_label":media::format_label(&info),"formats":media::formats(&info)}),
                    )
                } else {
                    let out = media::preview(&info, url);
                    self.event("preview_progress",json!({"request_id":request,"loaded":out["entries"].as_array().map(|x|x.len()).unwrap_or(0),"total":out["total"],"complete":true}));
                    Ok(out)
                }
            }
            "cancel_preview" => {
                let id = strval(&p, "request_id");
                self.cancelled.lock().unwrap().insert(id.into());
                if let Some(pid) = self.pids.lock().unwrap().get(id).copied() {
                    process::kill_tree(pid);
                }
                Ok(Value::Null)
            }
            "enqueue" => {
                let settings = self.settings_for(&p["settings"])?;
                config::advanced_args(&settings, "download")?;
                let entries = p["entries"].as_array().ok_or("未选择视频")?;
                if entries.is_empty() {
                    return Err("请先选择视频".into());
                }
                let mut submissions = self.submissions.lock().unwrap();
                let request = strval(&p, "request_id");
                if !request.is_empty() {
                    if let Some(ids) = submissions.get(request) {
                        let ts = self.tasks.lock().unwrap();
                        return Ok(json!(ids
                            .iter()
                            .filter_map(|id| ts.get(id))
                            .collect::<Vec<_>>()));
                    }
                }
                let batch = p["batch_id"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(uid);
                let mut created = Vec::new();
                for e in entries {
                    let url = strval(e, "url");
                    media::valid_url(url)?;
                    if e["available"] == false {
                        return Err(format!("{} 无法下载", strval(e, "title")));
                    }
                    let t = json!({"id":uid(),"title":e["title"],"url":url,"status":"queued","progress":null,"downloaded":0,"total":e["size"],"speed":0,"index":e["index"],"entry_id":e.get("entry_id").unwrap_or(&e["id"]),"collection_id":p["collection_id"],"batch_total":p["batch_total"],"batch_count":entries.len(),"sequence":self.sequence.fetch_add(1,Ordering::SeqCst),"engine_version":config::catalog()["version"],"batch_id":if entries.len()>1||p["batch_title"].is_string(){json!(batch)}else{Value::Null},"batch_title":p["batch_title"],"thumbnail":e["thumbnail"],"format_label":"","created_at":now(),"settings":settings,"error":null,"output":null});
                    created.push(t);
                }
                {
                    let mut db = self.db.lock().unwrap();
                    let tx = db.transaction().map_err(err)?;
                    for t in &created {
                        tx.execute(
                            "INSERT INTO tasks(id,value) VALUES(?1,?2)",
                            params![strval(t, "id"), t.to_string()],
                        )
                        .map_err(err)?;
                    }
                    tx.commit().map_err(err)?;
                }
                {
                    let mut ts = self.tasks.lock().unwrap();
                    for t in &created {
                        ts.insert(strval(t, "id").into(), t.clone());
                    }
                }
                if !request.is_empty() {
                    submissions.insert(
                        request.into(),
                        created.iter().map(|t| strval(t, "id").into()).collect(),
                    );
                }
                for t in &created {
                    self.event("task", t.clone());
                }
                self.wake.notify_one();
                Ok(json!(created))
            }
            "task_action" => {
                let action = strval(&p, "action");
                let ids = p["ids"].as_array().ok_or("未选择任务")?;
                for v in ids {
                    let id = v.as_str().ok_or("任务标识无效")?;
                    let task = self
                        .tasks
                        .lock()
                        .unwrap()
                        .get(id)
                        .cloned()
                        .ok_or("任务不存在")?;
                    match action {
                        "cancel" => {
                            if !["queued", "starting", "downloading", "processing"]
                                .contains(&strval(&task, "status"))
                            {
                                continue;
                            }
                            self.cancelled.lock().unwrap().insert(id.into());
                            self.update(id, json!({"status":"cancelled","speed":0}), true);
                            if let Some(pid) = self.pids.lock().unwrap().get(id).copied() {
                                process::kill_tree(pid);
                            }
                        }
                        "retry" | "retry_current" => {
                            if self.active.lock().unwrap().contains(id) {
                                return Err("任务尚未停止，请稍后重试".into());
                            }
                            if !["failed", "cancelled", "interrupted", "skipped"]
                                .contains(&strval(&task, "status"))
                            {
                                return Err("只能重试失败、取消、中断或跳过的任务".into());
                            }
                            let mut t = task;
                            t["status"] = json!("queued");
                            t["error"] = Value::Null;
                            t["progress"] = Value::Null;
                            t["output"] = Value::Null;
                            t["downloaded"] = json!(0);
                            t["speed"] = json!(0);
                            t["eta"] = Value::Null;
                            t["sequence"] = json!(self.sequence.fetch_add(1, Ordering::SeqCst));
                            if action == "retry_current" {
                                t["settings"] = self.settings.lock().unwrap().clone();
                            }
                            self.cancelled.lock().unwrap().remove(id);
                            self.persist_task(&t)?;
                            self.tasks.lock().unwrap().insert(id.into(), t.clone());
                            self.event("task", t);
                        }
                        "remove" => {
                            if self.active.lock().unwrap().contains(id)
                                || task["status"] == "queued"
                            {
                                return Err("请先取消任务再清理记录".into());
                            }
                            self.tasks.lock().unwrap().remove(id);
                            self.db
                                .lock()
                                .unwrap()
                                .execute("DELETE FROM tasks WHERE id=?1", [id])
                                .map_err(err)?;
                        }
                        _ => return Err("未知任务操作".into()),
                    }
                }
                self.event("tasks", json!(self.all_tasks()));
                self.wake.notify_one();
                Ok(Value::Null)
            }
            "logs" => {
                let id = strval(&p, "id");
                if !self.tasks.lock().unwrap().contains_key(id) {
                    return Err("任务不存在".into());
                }
                uuid::Uuid::parse_str(id).map_err(|_| "旧任务日志标识无法安全读取")?;
                self.storage
                    .owned_path(&self.data_dir.join("logs").join(format!("{id}.log")))?;
                self.storage
                    .owned_path(&self.data_dir.join("logs").join(format!("{id}.stdout")))?;
                let output =
                    std::fs::read_to_string(self.data_dir.join("logs").join(format!("{id}.log")))
                        .unwrap_or_default();
                let lines: Vec<_> = output
                    .lines()
                    .rev()
                    .take(500)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let stdout = std::fs::read_to_string(
                    self.data_dir.join("logs").join(format!("{id}.stdout")),
                )
                .unwrap_or(output.clone());
                Ok(json!({"lines":lines,"output":stdout}))
            }
            "run_command" => {
                let s = self.settings_for(&p["settings"])?;
                let args = strval(&p, "args");
                let mut effective = config::advanced_args(&s, "command")?;
                effective.extend(config::split_args(args)?);
                self.storage
                    .validate_args(&effective, std::path::Path::new(strval(&s, "directory")))?;
                if p["stdin"]
                    .as_str()
                    .map(|s| s.len() > 16 * 1024 * 1024)
                    .unwrap_or(false)
                {
                    return Err("标准输入过大，请使用 --batch-file 读取文件".into());
                }
                let t = json!({"id":uid(),"title":"命令工具","url":"","status":"queued","created_at":now(),"sequence":self.sequence.fetch_add(1,Ordering::SeqCst),"settings":s,"command_args":args,"command_stdin":p["stdin"],"progress":null});
                self.persist_task(&t)?;
                self.tasks
                    .lock()
                    .unwrap()
                    .insert(strval(&t, "id").into(), t.clone());
                self.event("task", t.clone());
                self.wake.notify_one();
                Ok(t)
            }
            "engine_info" => Ok(self.engine_info().await),
            "engine_verify" => self.engine_verify().await,
            "engine_update" => self.engine_update(false).await,
            "engine_rollback" => self.engine_update(true).await,
            _ => Err(format!("未知操作：{op}")),
        }
    }
}

#[cfg(test)]
fn test_directory() -> tempfile::TempDir {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-artifacts/core-unit");
    std::fs::create_dir_all(&base).unwrap();
    let base = std::fs::canonicalize(&base).unwrap();
    #[cfg(windows)]
    let base = PathBuf::from(
        base.to_string_lossy()
            .strip_prefix("\\\\?\\")
            .unwrap_or(&base.to_string_lossy()),
    );
    tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(base)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_migration_preserves_media_paths_and_only_lists_proven_app_files() {
        let d = crate::test_directory();
        let old = d.path().join("state");
        let videos = d.path().join("Videos").join("流光");
        std::fs::create_dir_all(old.join("downloads")).unwrap();
        std::fs::create_dir_all(old.join("logs")).unwrap();
        std::fs::create_dir_all(old.join("engine/staging")).unwrap();
        for relative in [
            "engine/yt-dlp.exe",
            "engine/yt-dlp.previous",
            "engine/staging/yt-dlp.exe.new",
        ] {
            std::fs::write(old.join(relative), b"MZ fixture binary, never executed").unwrap();
        }
        std::fs::write(old.join("engine/user-video.exe"), b"MZ user media stays").unwrap();
        let media = old.join("downloads/keep.noextension");
        std::fs::write(&media, b"user video").unwrap();
        let unknown = old.join("logs/user-video.mp4");
        std::fs::write(&unknown, b"another user video").unwrap();
        let id = uid();
        std::fs::write(old.join(format!("logs/{id}.log")), "previous diagnostic").unwrap();
        let db = Connection::open(old.join("flow.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE kv(key TEXT PRIMARY KEY,value TEXT NOT NULL);CREATE TABLE tasks(id TEXT PRIMARY KEY,value TEXT NOT NULL);PRAGMA user_version=1;").unwrap();
        let settings = defaults(&old);
        let task = json!({"id":id,"status":"completed","settings":settings,"output":media});
        db.execute(
            "INSERT INTO kv VALUES('settings',?1)",
            [settings.to_string()],
        )
        .unwrap();
        db.execute("INSERT INTO kv VALUES('profiles',?1)",[json!([{"id":"preset","name":"old","settings":{"directory":d.path().join("custom-videos")}}]).to_string()]).unwrap();
        db.execute(
            "INSERT INTO tasks VALUES(?1,?2)",
            params![id, task.to_string()],
        )
        .unwrap();
        drop(db);
        let before = std::fs::read(old.join("flow.sqlite3")).unwrap();
        let c = Core::new_with_paths(
            old.clone(),
            d.path().join("engine"),
            videos.clone(),
            vec![],
            Arc::new(|_| {}),
        )
        .unwrap();
        assert_eq!(c.settings.lock().unwrap()["directory"], json!(videos));
        assert_eq!(c.all_tasks()[0]["output"], json!(media));
        assert_eq!(
            c.all_tasks()[0]["settings"]["directory"],
            json!(old.join("downloads"))
        );
        assert_eq!(std::fs::read(&media).unwrap(), b"user video");
        assert!(unknown.exists());
        assert_eq!(std::fs::read(old.join("flow.sqlite3")).unwrap(), before);
        let manifest: Value = serde_json::from_slice(
            &std::fs::read(c.data_dir.join("legacy-owned-files.json")).unwrap(),
        )
        .unwrap();
        let files = manifest["files"].as_array().unwrap();
        assert!(files.iter().any(|f| f["path"] == "flow.sqlite3"));
        assert!(files.iter().any(|f| f["path"] == format!("logs/{id}.log")));
        assert!(files
            .iter()
            .any(|f| f["path"] == "engine/staging/yt-dlp.exe.new" && f["role"] == "engine-binary"));
        assert_eq!(
            std::fs::read(c.data_dir.join("engine/yt-dlp.exe")).unwrap(),
            b"MZ fixture binary, never executed"
        );
        assert!(old.join("engine/user-video.exe").exists());
        assert!(!files.iter().any(|f| f["path"] == "engine/user-video.exe"));
        assert!(!files
            .iter()
            .any(|f| f["path"].as_str().unwrap().contains("downloads")
                || f["path"].as_str().unwrap().contains("user-video")));
        assert!(manifest["protected_media_roots"]
            .as_array()
            .unwrap()
            .contains(&json!(old.join("downloads"))));
        assert!(manifest["protected_media_roots"]
            .as_array()
            .unwrap()
            .contains(&json!(d.path().join("custom-videos"))));
        assert_eq!(c.storage_paths()["migration"]["files_preserved"], true);
        assert!(c
            .settings_for(&json!({"directory":old.join("downloads")}))
            .is_err());
        drop(c);
        let reopened = Core::new_with_paths(
            old,
            d.path().join("engine"),
            videos.clone(),
            vec![],
            Arc::new(|_| {}),
        )
        .unwrap();
        assert_eq!(
            reopened.settings.lock().unwrap()["directory"],
            json!(videos)
        );
        assert_eq!(
            reopened.storage_paths()["migration"]["files_preserved"],
            true
        );
    }
    #[test]
    fn settings_and_profiles_survive_restart() {
        let d = crate::test_directory();
        let c = Core::new(d.path().into(), d.path().into(), Arc::new(|_| {})).unwrap();
        let s = c.settings_for(&json!({"quality":"2160"})).unwrap();
        c.save_kv("settings", &s).unwrap();
        drop(c);
        let c = Core::new(d.path().into(), d.path().into(), Arc::new(|_| {})).unwrap();
        assert_eq!(c.settings.lock().unwrap()["quality"], "2160");
    }
    #[test]
    fn interrupted_tasks_recovered() {
        let d = crate::test_directory();
        let c = Core::new(d.path().into(), d.path().into(), Arc::new(|_| {})).unwrap();
        c.persist_task(&json!({"id":"test","status":"downloading"}))
            .unwrap();
        drop(c);
        let c = Core::new(d.path().into(), d.path().into(), Arc::new(|_| {})).unwrap();
        assert_eq!(c.all_tasks()[0]["status"], "interrupted");
    }
    #[tokio::test]
    async fn paused_queue_is_persistent_and_submission_is_idempotent() {
        let d = crate::test_directory();
        let c = Core::new(d.path().into(), d.path().into(), Arc::new(|_| {})).unwrap();
        c.dispatch("queue_control", json!({"paused":true}))
            .await
            .unwrap();
        let request = json!({"entries":[{"id":"a","url":"http://localhost/a","title":"A","index":7},{"id":"b","url":"http://localhost/b","title":"B","index":8}],"batch_title":"series","batch_total":100,"collection_id":"source","request_id":"double-click"});
        let first = c.dispatch("enqueue", request.clone()).await.unwrap();
        let second = c.dispatch("enqueue", request).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(c.all_tasks().len(), 2);
        assert!(first[0]["sequence"].as_u64() < first[1]["sequence"].as_u64());
        assert_eq!(first[0]["batch_total"], 100);
        assert_eq!(first[0]["entry_id"], "a");
        c.shutdown();
        let other = Core::new(d.path().into(), d.path().into(), Arc::new(|_| {})).unwrap();
        assert!(other.queue_paused.load(Ordering::SeqCst));
        assert_eq!(other.all_tasks()[0]["status"], "interrupted");
    }
    #[tokio::test]
    async fn terminal_state_wins_cancel_finish_race() {
        let d = crate::test_directory();
        let c = Core::new(d.path().into(), d.path().into(), Arc::new(|_| {})).unwrap();
        c.tasks
            .lock()
            .unwrap()
            .insert("done".into(), json!({"id":"done","status":"completed"}));
        c.update("done", json!({"status":"cancelled"}), true);
        assert_eq!(c.all_tasks()[0]["status"], "completed");
        c.dispatch("task_action", json!({"ids":["done"],"action":"cancel"}))
            .await
            .unwrap();
        assert_eq!(c.all_tasks()[0]["status"], "completed");
        c.shutdown();
    }
    #[test]
    fn newer_database_is_not_downgraded() {
        let d = crate::test_directory();
        let db = Connection::open(d.path().join("flow.sqlite3")).unwrap();
        db.execute_batch("PRAGMA user_version=2;").unwrap();
        drop(db);
        assert!(Core::new(d.path().into(), d.path().into(), Arc::new(|_| {})).is_err());
        let db = Connection::open(d.path().join("flow.sqlite3")).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
    }
    #[tokio::test]
    async fn resource_integrity_detects_tampering_and_path_escape() {
        use sha2::{Digest, Sha256};
        let d = crate::test_directory();
        let engine = d.path().join("resources");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::write(engine.join("sample"), b"verified").unwrap();
        let manifest = json!({"files":[{"path":"sample","sha256":format!("{:x}",Sha256::digest(b"verified"))}]});
        std::fs::write(engine.join("manifest.json"), manifest.to_string()).unwrap();
        let c = Core::new(d.path().join("state"), engine.clone(), Arc::new(|_| {})).unwrap();
        assert_eq!(c.engine_verify().await.unwrap()["ok"], true);
        std::fs::write(engine.join("sample"), b"changed").unwrap();
        assert_eq!(c.engine_verify().await.unwrap()["ok"], false);
        std::fs::write(
            engine.join("manifest.json"),
            json!({"files":[{"path":"../sample","sha256":"invalid"}]}).to_string(),
        )
        .unwrap();
        assert!(c.engine_verify().await.is_err());
    }
    #[test]
    fn interrupted_engine_promotion_recovers_previous_file() {
        let d = crate::test_directory();
        let initial = Core::new(
            d.path().into(),
            d.path().join("resources"),
            Arc::new(|_| {}),
        )
        .unwrap();
        let managed = initial.data_dir.join("engine");
        initial.shutdown();
        drop(initial);
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::write(managed.join("yt-dlp.previous"), b"previous engine").unwrap();
        let c = Core::new(
            d.path().into(),
            d.path().join("resources"),
            Arc::new(|_| {}),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(managed.join(if cfg!(windows) {
                "yt-dlp.exe"
            } else {
                "yt-dlp"
            }))
            .unwrap(),
            b"previous engine"
        );
        c.shutdown();
    }
}
