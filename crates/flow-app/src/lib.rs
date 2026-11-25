// SPDX-License-Identifier: Apache-2.0
//! Native application boundary. No web server or WebView; UI events are bounded.
use flow_core::Core;
pub use flow_core::{effective_override_names, effective_proxy};
use flow_platform::desktop::{NativeShell, ShellEvent, WindowState};
use flow_platform::lifecycle::Paths;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Notify;

const TASK_LIMIT: usize = 512;
const REQUEST_LIMIT: usize = 16;
const PREVIEW_ENTRY_LIMIT: usize = 4096;
const NOTICE_LIMIT: usize = 64;

#[derive(Default)]
struct Pending {
    tasks: BTreeMap<String, Value>,
    previews: BTreeMap<String, Value>,
    state: BTreeMap<String, Value>,
    notices: VecDeque<Value>,
    shell: BTreeMap<String, Value>,
    resync: bool,
}

#[derive(Default)]
struct Inbox {
    pending: Mutex<Pending>,
    wake: Notify,
    last_drain: Mutex<Option<std::time::Instant>>,
}

impl Inbox {
    fn push(&self, event: Value) {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        match event["kind"].as_str().unwrap_or_default() {
            "shell" => {
                let action = event["data"]["action"]
                    .as_str()
                    .unwrap_or("show")
                    .to_owned();
                pending.shell.insert(action, event);
            }
            "task" => {
                if let Some(id) = event["data"]["id"].as_str() {
                    let id = id.to_owned();
                    if pending.tasks.get(&id).is_some_and(|previous| {
                        match (
                            previous["data"]["_revision"].as_u64(),
                            event["data"]["_revision"].as_u64(),
                        ) {
                            (Some(previous), Some(next)) => previous > next,
                            _ => false,
                        }
                    }) {
                        return;
                    }
                    if pending.tasks.len() >= TASK_LIMIT && !pending.tasks.contains_key(&id) {
                        pending.tasks.clear();
                        pending.resync = true;
                    }
                    pending.tasks.insert(id, event);
                }
            }
            "tasks" => {
                // Do not retain another complete history snapshot while the UI is busy.
                pending.tasks.clear();
                pending.resync = true;
            }
            "preview_progress" => {
                let id = event["data"]["request_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if pending.previews.len() >= REQUEST_LIMIT && !pending.previews.contains_key(&id) {
                    pending.previews.pop_first();
                }
                let mut next = event;
                if let Some(previous) = pending.previews.remove(&id) {
                    let mut data = previous["data"].as_object().cloned().unwrap_or_default();
                    let mut entries = data
                        .remove("entries")
                        .and_then(|v| v.as_array().cloned())
                        .unwrap_or_default();
                    let skipped = data
                        .get("snapshot_required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if let Some(new) = next["data"].as_object() {
                        data.extend(new.clone());
                    }
                    if let Some(new) = next["data"]["entries"].as_array() {
                        entries.extend(new.iter().cloned());
                    }
                    if skipped || entries.len() > PREVIEW_ENTRY_LIMIT {
                        data.remove("entries");
                        data.insert("snapshot_required".into(), json!(true));
                    } else if !entries.is_empty() {
                        data.insert("entries".into(), json!(entries));
                    }
                    next["data"] = Value::Object(data);
                }
                if next["data"]["entries"]
                    .as_array()
                    .is_some_and(|entries| entries.len() > PREVIEW_ENTRY_LIMIT)
                {
                    if let Some(data) = next["data"].as_object_mut() {
                        data.remove("entries");
                        data.insert("snapshot_required".into(), json!(true));
                    }
                }
                pending.previews.insert(id, next);
            }
            "settings" | "queue_state" => {
                pending
                    .state
                    .insert(event["kind"].as_str().unwrap().into(), event);
            }
            _ => {
                if pending.notices.len() >= NOTICE_LIMIT {
                    pending.notices.pop_front();
                }
                pending.notices.push_back(event);
            }
        }
        drop(pending);
        self.wake.notify_one();
    }

    fn drain(&self) -> Vec<Value> {
        *self.last_drain.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(std::time::Instant::now());
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let mut result: Vec<_> = pending.notices.drain(..).collect();
        result.extend(std::mem::take(&mut pending.state).into_values());
        result.extend(std::mem::take(&mut pending.shell).into_values());
        if std::mem::take(&mut pending.resync) {
            result.push(json!({"kind":"tasks_invalidated","data":{}}));
        }
        result.extend(std::mem::take(&mut pending.tasks).into_values());
        result.extend(std::mem::take(&mut pending.previews).into_values());
        result
    }

    fn ready(&self) -> bool {
        let p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.resync
            || !p.tasks.is_empty()
            || !p.previews.is_empty()
            || !p.notices.is_empty()
            || !p.state.is_empty()
            || !p.shell.is_empty()
    }

    fn progress_delay(&self) -> std::time::Duration {
        let p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let critical = p.resync
            || !p.state.is_empty()
            || !p.notices.is_empty()
            || !p.shell.is_empty()
            || p.tasks.values().any(|e| {
                !matches!(
                    e["data"]["status"].as_str(),
                    Some("downloading" | "processing")
                )
            })
            || p.previews.values().any(|e| e["data"]["complete"] == true);
        drop(p);
        if critical {
            return std::time::Duration::ZERO;
        }
        self.last_drain
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .map_or(std::time::Duration::ZERO, |last| {
                std::time::Duration::from_millis(100).saturating_sub(last.elapsed())
            })
    }
}

pub struct Service {
    core: Arc<Core>,
    paths: Paths,
    engine: PathBuf,
    inbox: Arc<Inbox>,
    closed: AtomicBool,
    close_to_tray: AtomicBool,
    first_close: AtomicBool,
    exit_pending: AtomicBool,
    hidden: Arc<AtomicBool>,
    notifications: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
    tray_available: bool,
    _shell: NativeShell,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl Service {
    /// Call in a blocking worker: initialization opens the database and verifies paths.
    pub fn new(engine_dir: PathBuf) -> Result<Arc<Self>, String> {
        if !engine_dir.is_absolute() || !engine_dir.is_dir() {
            return Err("运行组件目录不存在".into());
        }
        let paths = Paths::system()?;
        paths.initialize()?;
        let inbox = Arc::new(Inbox::default());
        let shell_sender = inbox.clone();
        let shell = NativeShell::new(
            &paths,
            Arc::new(move |event| {
                shell_sender.push(json!({"kind":"shell","data":{"action": match event {
                    ShellEvent::Show => "show",
                    ShellEvent::Exit => "exit",
                    ShellEvent::NewDownload => "new_download",
                    ShellEvent::Settings => "settings",
                    ShellEvent::Hidden => "hidden",
                }}}));
            }),
        )?;
        if !shell.is_primary() {
            return Err("FLOW_ALREADY_RUNNING".into());
        }
        let notifier = shell.notifications();
        let tray_available = shell.tray_available();
        let sender = inbox.clone();
        let revision = Arc::new(AtomicU64::new(0));
        let event_revision = revision.clone();
        let hidden = Arc::new(AtomicBool::new(false));
        let notifications = Arc::new(AtomicBool::new(true));
        let (hidden_flag, notification_flag) = (hidden.clone(), notifications.clone());
        let previous = Mutex::new(BTreeMap::<String, String>::new());
        let core = Core::new_with_cache_root(
            paths.data_base.clone(),
            engine_dir.clone(),
            paths.downloads.clone(),
            vec![paths.local.clone(), paths.roaming.clone()],
            paths.core_cache_root(),
            Arc::new(move |mut event| {
                let revision = event_revision.fetch_add(1, Ordering::AcqRel) + 1;
                event["_revision"] = json!(revision);
                if event["kind"] == "task" {
                    event["data"]["_revision"] = json!(revision);
                    let id = event["data"]["id"].as_str().unwrap_or_default();
                    let state = event["data"]["status"].as_str().unwrap_or_default();
                    let mut states = previous.lock().unwrap_or_else(|p| p.into_inner());
                    if states.len() >= TASK_LIMIT && !states.contains_key(id) {
                        states.pop_first();
                    }
                    let changed = states.insert(id.into(), state.into()).as_deref() != Some(state);
                    drop(states);
                    if changed
                        && hidden_flag.load(Ordering::Acquire)
                        && notification_flag.load(Ordering::Acquire)
                        && matches!(state, "completed" | "failed")
                    {
                        let title = if state == "completed" {
                            "流光 · 下载完成"
                        } else {
                            "流光 · 下载失败"
                        };
                        let _ = notifier
                            .notify(title, event["data"]["title"].as_str().unwrap_or("下载任务"));
                    }
                }
                sender.push(event);
            }),
        )?;
        paths.register_data()?;
        Ok(Arc::new(Self {
            core,
            paths,
            engine: engine_dir,
            inbox,
            closed: AtomicBool::new(false),
            close_to_tray: AtomicBool::new(true),
            first_close: AtomicBool::new(true),
            exit_pending: AtomicBool::new(false),
            hidden,
            notifications,
            revision,
            tray_available,
            _shell: shell,
        }))
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    pub fn drain_events(&self) -> Vec<Value> {
        self.inbox.drain()
    }
    pub async fn wait_events(&self) {
        loop {
            let notified = self.inbox.wake.notified();
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            if self.inbox.ready() {
                let delay = self.inbox.progress_delay();
                if delay.is_zero() {
                    return;
                }
                tokio::select! { _ = notified => continue, _ = tokio::time::sleep(delay) => return }
            }
            notified.await;
        }
    }

    pub async fn dispatch(&self, op: &str, payload: Value) -> Result<Value, String> {
        #[cfg(target_os = "macos")]
        if op == "clean_application_data" {
            if self.is_closed() || self.exit_pending.swap(true, Ordering::AcqRel) {
                return Ok(json!({"cancelled":true}));
            }
            let approved = tokio::task::spawn_blocking(flow_platform::shell::confirm_cleanup)
                .await
                .unwrap_or(false);
            if !approved {
                self.exit_pending.store(false, Ordering::Release);
                return Ok(json!({"cancelled":true}));
            }
            let paths = self.paths.clone();
            let started = tokio::task::spawn_blocking(move || {
                flow_platform::mac_maintenance::spawn_cleanup_after_exit(&paths)
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            if let Err(error) = started {
                self.exit_pending.store(false, Ordering::Release);
                return Err(error);
            }
            // The helper waits for this process and every participating CLI to
            // exit before taking exclusive locks and removing owned data.
            self.shutdown();
            return Ok(json!({"action":"exit","cancelled":false}));
        }
        if self.closed.load(Ordering::Acquire) {
            return Err("流光正在退出".into());
        }
        match op {
            "window_state" => {
                return serde_json::to_value(flow_platform::desktop::load_window_state(&self.paths))
                    .map_err(|e| e.to_string())
            }
            "save_window_state" => {
                let state: WindowState =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                let paths = self.paths.clone();
                tokio::task::spawn_blocking(move || {
                    let state = flow_platform::desktop::capture_window_state(&state);
                    flow_platform::desktop::save_window_state(&paths, &state)
                })
                .await
                .map_err(|e| e.to_string())??;
                return Ok(Value::Null);
            }
            "window_visible" => {
                self.hidden.store(
                    !payload["visible"].as_bool().unwrap_or(true),
                    Ordering::Release,
                );
                return Ok(Value::Null);
            }
            "request_close"
                if self.tray_available && self.close_to_tray.load(Ordering::Acquire) =>
            {
                if self.first_close.swap(false, Ordering::AcqRel) {
                    tokio::task::spawn_blocking(flow_platform::shell::explain_tray)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                self.hidden.store(true, Ordering::Release);
                return Ok(json!({"action":"hide","cancelled":false}));
            }
            "exit" | "request_close" => {
                if self.exit_pending.swap(true, Ordering::AcqRel) {
                    return Ok(json!({"cancelled":true}));
                }
                let tasks = self.core.dispatch("list_tasks", json!({})).await;
                let active = tasks
                    .as_ref()
                    .ok()
                    .and_then(Value::as_array)
                    .is_some_and(|tasks| {
                        tasks.iter().any(|t| {
                            matches!(
                                t["status"].as_str(),
                                Some("queued" | "starting" | "downloading" | "processing")
                            )
                        })
                    });
                let approved = if active {
                    tokio::task::spawn_blocking(flow_platform::shell::confirm_exit)
                        .await
                        .unwrap_or(false)
                } else {
                    tasks.is_ok()
                };
                self.exit_pending.store(false, Ordering::Release);
                if !approved {
                    return Ok(json!({"cancelled":true}));
                }
                self.shutdown();
                return Ok(json!({"action":"exit","cancelled":false}));
            }
            _ => {}
        }
        if matches!(
            op,
            "pick_folder"
                | "pick_cookie"
                | "pick_file"
                | "open_path"
                | "export_file"
                | "import_file"
                | "open_licenses"
        ) {
            let engine = self.engine.clone();
            let op = op.to_owned();
            return tokio::task::spawn_blocking(move || {
                flow_platform::shell::dispatch(&op, payload, &engine)
            })
            .await
            .map_err(|e| e.to_string())??
            .ok_or_else(|| "未知系统操作".into());
        }
        let snapshot_revision = self.revision.load(Ordering::Acquire);
        let core_op = if op == "task_snapshot" {
            "list_tasks"
        } else {
            op
        };
        let mut result = self.core.dispatch(core_op, payload).await?;
        if matches!(op, "bootstrap" | "save_settings") {
            let settings = if op == "bootstrap" {
                &result["settings"]
            } else {
                &result
            };
            self.close_to_tray.store(
                settings["close_to_tray"].as_bool().unwrap_or(true),
                Ordering::Release,
            );
            self.notifications.store(
                settings["notifications"].as_bool().unwrap_or(true),
                Ordering::Release,
            );
        }
        if op == "bootstrap" {
            if let Some(tasks) = result["tasks"].as_array_mut() {
                for task in tasks {
                    task["_revision"] = json!(snapshot_revision);
                }
            }
            result["tasks_revision"] = json!(snapshot_revision);
            result["shell"] = json!({"tray_available":self.tray_available,"window_state":flow_platform::desktop::load_window_state(&self.paths)});
        }
        if core_op == "list_tasks" {
            if let Some(tasks) = result.as_array_mut() {
                for task in tasks {
                    task["_revision"] = json!(snapshot_revision);
                }
            }
            if op == "task_snapshot" {
                result = json!({"tasks":result,"revision":snapshot_revision});
            }
        }
        Ok(result)
    }

    pub fn shutdown(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.core.shutdown();
            self.inbox.wake.notify_one();
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_flood_is_bounded_and_requires_authoritative_resync() {
        let inbox = Inbox::default();
        for id in 0..10_000 {
            inbox.push(json!({"kind":"task","data":{"id":id.to_string(),"status":"completed"}}));
        }
        let events = inbox.drain();
        assert!(events.len() <= TASK_LIMIT + 1);
        assert!(events.iter().any(|e| e["kind"] == "tasks_invalidated"));
    }
    #[test]
    fn updates_for_one_task_keep_latest_terminal_state() {
        let inbox = Inbox::default();
        for status in ["starting", "downloading", "processing", "completed"] {
            inbox.push(json!({"kind":"task","data":{"id":"a","status":status}}));
        }
        let events = inbox.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["data"]["status"], "completed");
    }
    #[test]
    fn late_progress_cannot_replace_a_newer_completed_revision() {
        let inbox = Inbox::default();
        inbox.push(json!({"kind":"task","data":{"id":"a","status":"completed","_revision":2}}));
        inbox.push(json!({"kind":"task","data":{"id":"a","status":"downloading","_revision":1}}));
        assert_eq!(inbox.drain()[0]["data"]["status"], "completed");
    }
    #[test]
    fn preview_batches_append_and_preserve_partial_metadata() {
        let inbox = Inbox::default();
        inbox.push(json!({"kind":"preview_progress","data":{"request_id":"a","title":"合集","entries":[{"id":"1"}],"loaded":1}}));
        inbox.push(json!({"kind":"preview_progress","data":{"request_id":"a","entries":[{"id":"2"}],"loaded":2}}));
        let events = inbox.drain();
        assert_eq!(events[0]["data"]["entries"].as_array().unwrap().len(), 2);
        assert_eq!(events[0]["data"]["title"], "合集");
    }
    #[test]
    fn large_first_preview_batch_requires_final_snapshot_without_truncating_result() {
        let inbox = Inbox::default();
        let entries: Vec<_> = (0..=PREVIEW_ENTRY_LIMIT)
            .map(|id| json!({"id":id}))
            .collect();
        inbox.push(json!({"kind":"preview_progress","data":{"request_id":"large","entries":entries,"loaded":PREVIEW_ENTRY_LIMIT+1}}));
        let events = inbox.drain();
        assert!(events[0]["data"]["entries"].is_null());
        assert_eq!(events[0]["data"]["snapshot_required"], true);
        assert_eq!(events[0]["data"]["loaded"], PREVIEW_ENTRY_LIMIT + 1);
    }
    #[test]
    fn notice_overflow_cannot_discard_queue_or_settings_state() {
        let inbox = Inbox::default();
        inbox.push(json!({"kind":"queue_state","data":{"paused":true}}));
        inbox.push(json!({"kind":"settings","data":{"quality":"1080"}}));
        for index in 0..NOTICE_LIMIT + 5 {
            inbox.push(json!({"kind":"notice","data":{"message":index}}));
        }
        let events = inbox.drain();
        assert_eq!(events.len(), NOTICE_LIMIT + 2);
        assert!(events
            .iter()
            .any(|e| e["kind"] == "queue_state" && e["data"]["paused"] == true));
        assert!(events
            .iter()
            .any(|e| e["kind"] == "settings" && e["data"]["quality"] == "1080"));
    }
    #[test]
    fn progress_is_throttled_but_terminal_and_shell_are_immediate() {
        let inbox = Inbox::default();
        inbox.drain();
        inbox.push(json!({"kind":"task","data":{"id":"1","status":"downloading"}}));
        assert!(!inbox.progress_delay().is_zero());
        inbox.push(json!({"kind":"task","data":{"id":"1","status":"completed"}}));
        assert!(inbox.progress_delay().is_zero());
        inbox.drain();
        inbox.push(json!({"kind":"shell","data":{"action":"exit"}}));
        assert!(inbox.progress_delay().is_zero());
    }
    #[tokio::test]
    async fn wake_is_not_lost_when_event_arrives_before_wait() {
        let inbox = Inbox::default();
        inbox.push(json!({"kind":"notice","data":{"message":"ok"}}));
        inbox.wake.notified().await;
        assert!(inbox.ready());
    }
}
