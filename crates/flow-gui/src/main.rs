// SPDX-License-Identifier: Apache-2.0
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod executor;
mod images;
mod modal;
mod stable_rows;
mod views;

use flow_app::Service;
use flow_gui::model::*;
use iced::{futures::SinkExt, window, Element, Subscription, Task, Theme};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Active,
    History,
    Add,
    Advanced,
    Settings,
    Tools,
    Detail,
}
#[derive(Debug, Clone)]
enum Msg {
    Ready(Result<Arc<Service>, String>),
    Image(u64, String, Result<iced::widget::image::Handle, String>),
    Reply(String, u64, Result<Value, String>),
    Events(Vec<Value>),
    Page(Page),
    ModalClose,
    FocusNext(bool),
    Text(String, String),
    Setting(String, Value),
    ToggleEntry(String, bool),
    SelectVisible(bool),
    Range,
    Preview,
    CancelPreview,
    Estimate,
    Submit,
    ToggleTask(String, bool),
    SelectTasks(bool),
    GroupSelect(Vec<String>, bool),
    Collapse(String),
    Action(String),
    Queue,
    AdvancedValue(String, String),
    AdvancedEnable(String, bool),
    AdvancedPick(String),
    OptionHelp(String),
    ToolOption(String),
    RepeatAdd(String),
    RepeatRemove(String, usize),
    RepeatSet(String, usize, usize, String),
    ResetOption(String),
    ResetAdvanced,
    AdvancedBack,
    SaveAdvancedDefault,
    ApplyProfile(String),
    SaveProfile,
    DeleteProfile(String),
    SaveSettings,
    ImportSettings,
    ExportSettings,
    RunCommand,
    Engine(String),
    Pick(String),
    Detail(String),
    DetailAction(String),
    RevealOutput,
    ExportLogs,
    CopyLogs,
    Open(String),
    Dismiss,
    Shortcut(iced::keyboard::Key, iced::keyboard::Modifiers),
    Scroll(String, f32, f32),
    Close(window::Id),
    ConfirmExit,
    #[cfg(target_os = "macos")]
    CleanApplicationData,
    Window(Option<window::Id>),
    SystemTheme(iced::theme::Mode),
    WindowEvent(window::Id, window::Event),
    EditArgs(iced::widget::text_editor::Action),
    EditStdin(iced::widget::text_editor::Action),
    EditRaw(iced::widget::text_editor::Action),
}

struct App {
    images: images::Images,
    service: Option<Arc<Service>>,
    initializing: bool,
    exit_after_init: bool,
    error: String,
    notice: String,
    page: Page,
    previous: Page,
    background_page: Page,
    settings: Value,
    draft: Value,
    preferences: Value,
    profiles: Vec<Value>,
    engine: Value,
    storage: Value,
    tasks: TaskStore,
    rows: Vec<TaskRow>,
    selected_tasks: HashSet<String>,
    collapsed: HashSet<String>,
    queue_paused: bool,
    fields: HashMap<String, String>,
    catalog: Vec<CatalogOption>,
    advanced_input: HashMap<String, String>,
    preview: Value,
    entries: Vec<Value>,
    selected_entries: HashSet<String>,
    preview_complete: bool,
    visible_entries: Vec<usize>,
    selection_summary: String,
    enqueue_queue: VecDeque<Value>,
    active_enqueue: Option<Value>,
    enqueued_count: usize,
    pending_urls: VecDeque<String>,
    imported_profiles: VecDeque<Value>,
    request: u64,
    request_id: String,
    estimate_id: String,
    preview_busy: bool,
    busy: bool,
    busy_operations: HashSet<String>,
    intent_cache: RefCell<(Value, Result<Vec<String>, String>)>,
    network_cache: Option<(Value, Result<(String, String), String>)>,
    estimate_queue: VecDeque<(String, String)>,
    estimate_total: usize,
    estimate_done: usize,
    restore_pending: bool,
    detail_revision: u64,
    offsets: HashMap<String, (f32, f32)>,
    detail_id: String,
    logs: Value,
    show_output: bool,
    tool_input: iced::widget::text_editor::Content,
    raw_input: iced::widget::text_editor::Content,
    stdin_input: iced::widget::text_editor::Content,
    window_id: Option<window::Id>,
    hidden: bool,
    system_dark: bool,
    window_state: Value,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = flow_platform::lifecycle::Paths::system().map_err(std::io::Error::other)?;
    #[cfg(target_os = "macos")]
    let _maintenance =
        flow_platform::mac_maintenance::acquire_shared(&paths).map_err(std::io::Error::other)?;
    let session =
        flow_platform::lifecycle::SessionTemp::create(&paths).map_err(std::io::Error::other)?;
    session.apply_to_current_process();
    iced::application(App::boot, App::update, App::view)
        .executor::<executor::SmallExecutor>()
        .title("流光")
        .theme(App::theme)
        .subscription(App::subscription)
        .default_font(if cfg!(target_os = "macos") {
            iced::Font::DEFAULT
        } else {
            iced::Font::with_name("Segoe UI Variable Text")
        })
        .window(window::Settings {
            size: iced::Size::new(1180., 820.),
            min_size: Some(iced::Size::new(620., 360.)),
            exit_on_close_request: false,
            icon: flow_platform::desktop::icon_rgba()
                .ok()
                .and_then(|(rgba, w, h)| window::icon::from_rgba(rgba, w, h).ok()),
            ..Default::default()
        })
        .run()
        .map_err(Into::into)
}

impl App {
    fn boot() -> (Self, Task<Msg>) {
        let app = Self {
            images: images::Images::default(),
            service: None,
            initializing: true,
            exit_after_init: false,
            error: String::new(),
            notice: String::new(),
            page: Page::Active,
            previous: Page::Add,
            background_page: Page::Active,
            settings: defaults(),
            draft: defaults(),
            preferences: defaults(),
            profiles: Vec::new(),
            engine: Value::Null,
            storage: Value::Null,
            tasks: TaskStore::default(),
            rows: Vec::new(),
            selected_tasks: HashSet::new(),
            collapsed: HashSet::new(),
            queue_paused: false,
            fields: HashMap::from([
                ("mode".into(), "auto".into()),
                ("filter".into(), "all".into()),
                ("advanced_view".into(), "common".into()),
                ("advanced_group".into(), "全部".into()),
                ("args".into(), "--help".into()),
            ]),
            catalog: Vec::new(),
            advanced_input: HashMap::new(),
            preview: Value::Null,
            entries: Vec::new(),
            selected_entries: HashSet::new(),
            preview_complete: false,
            visible_entries: Vec::new(),
            selection_summary: "大小未知".into(),
            enqueue_queue: VecDeque::new(),
            active_enqueue: None,
            enqueued_count: 0,
            pending_urls: VecDeque::new(),
            imported_profiles: VecDeque::new(),
            request: 0,
            request_id: String::new(),
            estimate_id: String::new(),
            preview_busy: false,
            busy: false,
            busy_operations: HashSet::new(),
            intent_cache: RefCell::new((Value::Null, Ok(Vec::new()))),
            network_cache: None,
            estimate_queue: VecDeque::new(),
            estimate_total: 0,
            estimate_done: 0,
            restore_pending: false,
            detail_revision: 0,
            offsets: HashMap::new(),
            detail_id: String::new(),
            logs: Value::Null,
            show_output: false,
            tool_input: iced::widget::text_editor::Content::with_text("--help"),
            raw_input: iced::widget::text_editor::Content::new(),
            stdin_input: iced::widget::text_editor::Content::new(),
            window_id: None,
            hidden: false,
            system_dark: false,
            window_state: json!({"width":1180.,"height":820.,"x":null,"y":null,"maximized":false}),
        };
        let engine = std::env::var_os("FLOW_ENGINE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let beside = std::env::current_exe()
                    .ok()
                    .and_then(|p| flow_platform::lifecycle::engine_directory(&p).ok())
                    .unwrap_or_default();
                if !cfg!(debug_assertions) || beside.is_dir() {
                    beside
                } else {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../engine")
                }
            });
        let init = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || Service::new(engine))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r)
            },
            Msg::Ready,
        );
        (
            app,
            Task::batch([
                init,
                window::oldest().map(Msg::Window),
                iced::system::theme().map(Msg::SystemTheme),
            ]),
        )
    }
    fn field(&self, key: &str) -> &str {
        self.fields.get(key).map(String::as_str).unwrap_or("")
    }
    fn active_settings(&self) -> &Value {
        if self.page == Page::Settings
            || (self.page == Page::Advanced && self.previous == Page::Settings)
        {
            &self.preferences
        } else {
            &self.draft
        }
    }
    fn active_settings_mut(&mut self) -> &mut Value {
        if self.page == Page::Settings
            || (self.page == Page::Advanced && self.previous == Page::Settings)
        {
            &mut self.preferences
        } else {
            &mut self.draft
        }
    }
    fn call(&self, op: &str, payload: Value) -> Task<Msg> {
        self.call_tag(op, op, payload, self.request)
    }
    fn call_tag(&self, tag: &str, op: &str, payload: Value, request: u64) -> Task<Msg> {
        let Some(service) = self.service.clone() else {
            return Task::none();
        };
        let op = op.to_string();
        let tag = tag.to_string();
        Task::perform(
            async move { service.dispatch(&op, payload).await },
            move |r| Msg::Reply(tag.clone(), request, r),
        )
    }
    fn rebuild_rows(&mut self) {
        self.rows = self.tasks.rows(
            self.base_page() == Page::History,
            self.field("filter"),
            self.field("search"),
            &self.collapsed,
        );
    }
    fn is_modal(&self) -> bool {
        matches!(self.page, Page::Add | Page::Advanced | Page::Detail)
    }
    fn base_page(&self) -> Page {
        if self.is_modal() {
            self.background_page
        } else {
            self.page
        }
    }
    fn rebuild_entries(&mut self) {
        let query = self.field("entry_search").to_lowercase();
        self.visible_entries = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| query.is_empty() || s(e, "title").to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        self.selection_summary = selected_size(&self.entries, &self.selected_entries);
    }
    fn clean_settings(&self, settings: &Value) -> Value {
        let mut clean = exportable(settings);
        if let Some(values) = clean["advanced"].as_object_mut() {
            for option in self.catalog.iter().filter(|o| b(&o.data, "sensitive")) {
                values.remove(s(&option.data, "name"));
            }
        }
        clean
    }
    fn override_names(&self) -> Result<Vec<String>, String> {
        let settings = self.active_settings();
        let mut cache = self.intent_cache.borrow_mut();
        if cache.0 != *settings {
            *cache = (
                settings.clone(),
                flow_app::effective_override_names(settings),
            );
        }
        cache.1.clone()
    }
    fn enqueue_next(&mut self) -> Task<Msg> {
        if let Some(payload) = self.enqueue_queue.pop_front() {
            self.active_enqueue = Some(payload.clone());
            self.call("enqueue", payload)
        } else {
            self.active_enqueue = None;
            self.busy = false;
            self.page = Page::Active;
            self.rebuild_rows();
            self.notice = format!("已添加 {} 个下载任务", self.enqueued_count);
            Task::none()
        }
    }
    fn preview_next(&mut self) -> Task<Msg> {
        let Some(url) = self.pending_urls.pop_front() else {
            self.preview_busy = false;
            self.preview_complete = true;
            return Task::none();
        };
        self.request_id = format!("preview-{}-{}", self.request, uuid::Uuid::new_v4());
        self.call("preview",json!({"url":url,"mode":self.field("mode"),"settings":self.draft,"request_id":self.request_id}))
    }
    fn invalidate_estimate(&mut self) -> Task<Msg> {
        self.estimate_queue.clear();
        self.fields.remove("estimate_status");
        if !self.estimate_id.is_empty() {
            self.busy = false;
            let id = std::mem::take(&mut self.estimate_id);
            self.call("cancel_preview", json!({"request_id":id}))
        } else {
            Task::none()
        }
    }
    fn draft_changed(&mut self, clear: bool) -> Task<Msg> {
        invalidate_pending_submissions(
            &mut self.enqueue_queue,
            self.active_enqueue.is_some(),
            &self.draft,
            &self.selected_entries,
        );
        self.request += 1;
        let cancel = if self.preview_busy {
            self.preview_busy = false;
            self.preview_complete = false;
            self.pending_urls.clear();
            let id = std::mem::take(&mut self.request_id);
            self.call("cancel_preview", json!({"request_id":id}))
        } else {
            Task::none()
        };
        if clear {
            if self.active_enqueue.is_none() {
                self.enqueue_queue.clear();
            }
            self.entries.clear();
            self.selected_entries.clear();
            self.preview = Value::Null;
            self.preview_complete = false;
        } else {
            for item in &mut self.entries {
                item["size"] = Value::Null;
            }
            if self.preview.is_object() {
                for key in ["size", "disk_estimate", "formats", "format_label"] {
                    self.preview[key] = Value::Null;
                }
            }
        }
        self.rebuild_entries();
        Task::batch([cancel, self.invalidate_estimate()])
    }
    fn estimate_next(&mut self) -> Task<Msg> {
        let Some((entry, url)) = self.estimate_queue.pop_front() else {
            self.busy = false;
            self.estimate_id.clear();
            return Task::none();
        };
        self.estimate_id = format!("size-{}-{}", self.request, uuid::Uuid::new_v4());
        self.call_tag(
            &format!("estimate:{entry}"),
            "estimate",
            json!({"url":url,"settings":self.draft,"request_id":self.estimate_id,"refresh":true}),
            self.request,
        )
    }
    fn restore_window(&mut self) -> Task<Msg> {
        let Some(id) = self.window_id.filter(|_| self.restore_pending) else {
            return Task::none();
        };
        self.restore_pending = false;
        let width = self.window_state["width"].as_f64().unwrap_or(1180.) as f32;
        let height = self.window_state["height"].as_f64().unwrap_or(820.) as f32;
        let mut task = window::resize(id, iced::Size::new(width.max(620.), height.max(360.)));
        if let (Some(x), Some(y)) = (
            self.window_state["x"].as_f64(),
            self.window_state["y"].as_f64(),
        ) {
            task = task.chain(window::move_to(id, iced::Point::new(x as f32, y as f32)));
        }
        if b(&self.window_state, "maximized") {
            task = task.chain(window::maximize(id, true));
        }
        task
    }
    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Image(generation, url, result) => {
                self.images.finish(generation, url, result);
                return self.load_images();
            }
            Msg::Ready(result) => {
                self.initializing = false;
                match result {
                    Ok(service) => {
                        self.service = Some(service);
                        if self.exit_after_init {
                            return self.update(Msg::ConfirmExit);
                        }
                        return self.call("bootstrap", json!({}));
                    }
                    Err(e) => {
                        if e == "FLOW_ALREADY_RUNNING" || self.exit_after_init {
                            flow_platform::desktop::prepare_exit();
                            return iced::exit();
                        }
                        self.error = e
                    }
                }
            }
            Msg::Window(id) => {
                self.window_id = id;
                return self.restore_window();
            }
            Msg::SystemTheme(mode) => self.system_dark = mode == iced::theme::Mode::Dark,
            Msg::WindowEvent(id, event) => {
                self.window_id = Some(id);
                match event {
                    window::Event::Resized(size) => {
                        self.window_state["width"] = size.width.into();
                        self.window_state["height"] = size.height.into();
                    }
                    window::Event::Moved(point) => {
                        self.window_state["x"] = point.x.into();
                        self.window_state["y"] = point.y.into();
                    }
                    _ => {}
                }
            }
            Msg::Reply(tag, request, result) => {
                if (tag == "preview"
                    || tag.starts_with("estimate:")
                    || tag == "advanced_apply"
                    || tag.starts_with("pick:"))
                    && request != self.request
                {
                    return Task::none();
                }
                if tag == "logs" && request != self.detail_revision {
                    return Task::none();
                }
                self.busy_operations.remove(&tag);
                let value = match result {
                    Ok(v) => v,
                    Err(e) => {
                        self.error = e;
                        // Window placement is best-effort; it must never trap
                        // the user in a window when the storage is unavailable.
                        if tag == "save_before_close" {
                            return self.call("request_close", json!({}));
                        }
                        if tag == "save_before_exit" {
                            return self.call("exit", json!({}));
                        }
                        if tag == "enqueue" || tag.starts_with("estimate:") {
                            self.busy = false;
                        }
                        if tag == "preview" {
                            self.preview_busy = false;
                            self.preview_complete = false;
                            self.pending_urls.clear()
                        }
                        if tag == "enqueue" {
                            if let Some(payload) = self.active_enqueue.take() {
                                self.enqueue_queue.push_front(payload);
                            }
                            self.notice = format!(
                                "已添加 {} 项；未完成的 {} 组已保留，点击下载重试",
                                self.enqueued_count,
                                self.enqueue_queue.len()
                            );
                            self.error = format!("{}\n{}", self.error, self.notice);
                            self.rebuild_rows();
                        }
                        if tag.starts_with("estimate:") {
                            return self.invalidate_estimate();
                        }
                        return Task::none();
                    }
                };
                match tag.as_str() {
                    "bootstrap" => {
                        self.settings = normalized(&value["settings"]);
                        self.draft = self.settings.clone();
                        self.preferences = self.settings.clone();
                        self.tasks.replace(
                            &value["tasks"],
                            value["tasks_revision"].as_u64().unwrap_or(0),
                        );
                        self.profiles = value["profiles"].as_array().cloned().unwrap_or_default();
                        self.engine = value["engine"].clone();
                        self.storage = value["storage"].clone();
                        self.catalog = CatalogOption::flatten(&value["catalog"]);
                        self.queue_paused =
                            b(&value, "queue_paused") || b(&value["queue"], "paused");
                        self.rebuild_rows();
                        if value["shell"]["window_state"].is_object() {
                            self.window_state = value["shell"]["window_state"].clone();
                            self.restore_pending = true;
                            return self.restore_window();
                        }
                    }
                    "task_snapshot" => {
                        self.tasks
                            .replace(&value["tasks"], value["revision"].as_u64().unwrap_or(0));
                        if self.page == Page::Detail
                            && !self.tasks.tasks.contains_key(&self.detail_id)
                        {
                            self.page = Page::History;
                        }
                        self.rebuild_rows()
                    }
                    "preview" => {
                        // The successful final response, not a progress batch, defines selectable entries.
                        let mut items = value["entries"].as_array().cloned().unwrap_or_default();
                        if items.is_empty() && !b(&value, "is_playlist") {
                            items.push(value.clone());
                        }
                        self.entries
                            .retain(|e| s(e, "_preview_request") != self.request_id);
                        let prefix = self.request_id.clone();
                        for (i, item) in items.iter_mut().enumerate() {
                            let original = id(item);
                            item["entry_id"] = item
                                .get("entry_id")
                                .cloned()
                                .unwrap_or_else(|| original.clone().into());
                            item["id"] = format!("{prefix}:{i}:{original}").into();
                            item["_batch_id"] = prefix.clone().into();
                            item["_batch_total"] = value["total"].clone();
                            item["_is_playlist"] = value["is_playlist"].clone();
                            item["collection_title"] = value["title"].clone();
                            item["collection_id"] = value["id"].clone();
                            if item["available"] != false {
                                self.selected_entries.insert(id(item));
                            }
                        }
                        self.entries.extend(items);
                        self.preview = value;
                        self.rebuild_entries();
                        let next = self.preview_next();
                        return Task::batch([next, self.load_images()]);
                    }
                    t if t.starts_with("estimate:") => {
                        if self.estimate_id.is_empty() {
                            return Task::none();
                        };
                        let target = t.trim_start_matches("estimate:");
                        if let Some(entry) = self.entries.iter_mut().find(|e| s(e, "id") == target)
                        {
                            for key in [
                                "size",
                                "approximate",
                                "format_label",
                                "disk_estimate",
                                "formats",
                            ] {
                                entry[key] = value[key].clone();
                            }
                        }
                        if self.entries.len() == 1 {
                            for key in [
                                "size",
                                "approximate",
                                "format_label",
                                "disk_estimate",
                                "formats",
                            ] {
                                self.preview[key] = value[key].clone();
                            }
                        }
                        self.estimate_done += 1;
                        self.fields.insert(
                            "estimate_status".into(),
                            format!("{} / {}", self.estimate_done, self.estimate_total),
                        );
                        self.rebuild_entries();
                        return self.estimate_next();
                    }
                    "enqueue" => {
                        if let Some(payload) = self.active_enqueue.take() {
                            acknowledge_submission(&payload, &mut self.selected_entries);
                        }
                        for task in value.as_array().into_iter().flatten() {
                            self.enqueued_count += 1;
                            self.tasks.upsert(task.clone());
                        }
                        self.rebuild_entries();
                        return self.enqueue_next();
                    }
                    "advanced_apply" => {
                        *self.active_settings_mut() = normalized(&value);
                        self.page = self.previous;
                        self.error.clear();
                    }
                    "save_settings" => {
                        self.settings = normalized(&value);
                        self.preferences = self.settings.clone();
                        self.notice = "默认设置已保存".into();
                        return self.import_next_profile();
                    }
                    "import_profile" => {
                        self.profiles = value.as_array().cloned().unwrap_or_default();
                        return self.import_next_profile();
                    }
                    "profiles_save" | "profiles_delete" => {
                        self.profiles = value.as_array().cloned().unwrap_or_default();
                        self.notice = "预设已更新".into()
                    }
                    "logs" => self.logs = value,
                    "run_command" => {
                        self.tasks.upsert(value.clone());
                        return self.update(Msg::Detail(id(&value)));
                    }
                    "engine_info" => {
                        self.engine = value;
                    }
                    "open_licenses" => {}
                    "engine_update" | "engine_rollback" | "engine_verify" | "diagnostics" => {
                        if value.get("engine").is_some() {
                            self.engine = value["engine"].clone();
                        }
                        self.fields.insert(
                            "tool_result".into(),
                            serde_json::to_string_pretty(&value).unwrap_or_default(),
                        );
                    }
                    "import_file" => {
                        if let Some(content) = value["content"].as_str() {
                            match serde_json::from_str::<Value>(content) {
                                Ok(v) if v["settings"].is_object() => {
                                    self.preferences = normalized(&v["settings"]);
                                    self.imported_profiles = v["profiles"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .filter(|p| {
                                            p["settings"].is_object()
                                                && !s(p, "name").trim().is_empty()
                                        })
                                        .cloned()
                                        .collect();
                                    self.notice = format!(
                                        "设置与 {} 个预设已载入草稿，检查后保存",
                                        self.imported_profiles.len()
                                    );
                                }
                                _ => self.error = "配置文件格式无效".into(),
                            }
                        }
                    }
                    t if t.starts_with("pick:advanced:") => {
                        if let Some(path) = value.as_str() {
                            let key = t.trim_start_matches("pick:advanced:");
                            let repeat = self
                                .catalog
                                .iter()
                                .find(|o| s(&o.data, "name") == key)
                                .is_some_and(|o| b(&o.data, "repeatable"));
                            if repeat {
                                let mut rows = self.repeat_rows(key);
                                rows.retain(|r| r.iter().any(|x| !x.is_empty()));
                                rows.push(vec![path.into()]);
                                self.store_repeat(key, rows);
                            } else {
                                self.active_settings_mut()["advanced"][key] = path.into();
                            }
                            return self.draft_changed(false);
                        }
                    }
                    t if t.starts_with("pick:") => {
                        if let Some(path) = value.as_str() {
                            let key = t.trim_start_matches("pick:");
                            self.active_settings_mut()[key] = path.into();
                            return self.draft_changed(false);
                        }
                    }
                    "export_file" => {
                        if !value.is_null() {
                            self.notice = "文件已导出".into()
                        }
                    }
                    "task_action" => {
                        self.selected_tasks.clear();
                        return self.call("task_snapshot", json!({}));
                    }
                    "queue_control" => self.queue_paused = b(&value, "paused"),
                    "save_before_close" => return self.call("request_close", json!({})),
                    "save_before_exit" => return self.call("exit", json!({})),
                    "request_close" => {
                        if b(&value, "cancelled") {
                            return Task::none();
                        }
                        if s(&value, "action") == "hide" {
                            self.hidden = true;
                            self.images.cancel();
                            flow_platform::desktop::hide_application();
                            if let Some(id) = self.window_id {
                                return Task::batch([
                                    window::set_mode(id, window::Mode::Hidden),
                                    self.call("window_visible", json!({"visible":false})),
                                ]);
                            }
                        } else if s(&value, "action") == "exit" {
                            flow_platform::desktop::prepare_exit();
                            return iced::exit();
                        }
                    }
                    "exit" | "clean_application_data" => {
                        if b(&value, "cancelled") {
                            return Task::none();
                        }
                        if let Some(service) = &self.service {
                            service.shutdown()
                        }
                        flow_platform::desktop::prepare_exit();
                        return iced::exit();
                    }
                    _ => {}
                }
            }
            Msg::Events(events) => {
                let mut refresh = false;
                let mut entries_changed = false;
                let mut commands = Vec::new();
                for event in events {
                    match s(&event, "kind") {
                        "task" => {
                            refresh |= self.tasks.upsert(event["data"].clone());
                        }
                        "tasks" | "tasks_invalidated" => {
                            commands.push(self.call("task_snapshot", json!({})))
                        }
                        "settings" => {
                            let settings = normalized(&event["data"]);
                            if self.draft == self.settings {
                                self.draft = settings.clone();
                            }
                            self.settings = settings;
                        }
                        "queue_state" => self.queue_paused = b(&event["data"], "paused"),
                        "notice" => self.notice = s(&event["data"], "message").into(),
                        "preview_progress" => {
                            if self.preview_busy
                                && s(&event["data"], "request_id") == self.request_id
                            {
                                entries_changed = true;
                                let data = &event["data"];
                                self.fields.insert(
                                    "preview_status".into(),
                                    format!(
                                        "已读取 {} 项{}",
                                        data["loaded"],
                                        if b(data, "snapshot_required") {
                                            " · 正在同步完整目录"
                                        } else {
                                            ""
                                        }
                                    ),
                                );
                                if !s(data, "title").is_empty() {
                                    if !self.preview.is_object() {
                                        self.preview = json!({});
                                    }
                                    self.preview["title"] = data["title"].clone();
                                }
                                for mut entry in
                                    data["entries"].as_array().cloned().unwrap_or_default()
                                {
                                    let original = id(&entry);
                                    entry["entry_id"] = original.clone().into();
                                    entry["id"] = format!(
                                        "partial:{}:{}:{}",
                                        self.request_id,
                                        self.entries.len(),
                                        original
                                    )
                                    .into();
                                    entry["_preview_request"] = self.request_id.clone().into();
                                    entry["_partial"] = true.into();
                                    self.entries.push(entry);
                                }
                            }
                        }
                        "shell" => match s(&event["data"], "action") {
                            "show" | "new_download" | "settings" => {
                                self.hidden = false;
                                flow_platform::desktop::activate_application();
                                if let Some(id) = self.window_id {
                                    commands.push(Task::batch([
                                        window::set_mode(id, window::Mode::Windowed),
                                        window::gain_focus(id),
                                        self.call("window_visible", json!({"visible":true})),
                                    ]));
                                }
                                match s(&event["data"], "action") {
                                    "new_download" => {
                                        commands.push(self.update(Msg::Page(Page::Add)))
                                    }
                                    "settings" => {
                                        commands.push(self.update(Msg::Page(Page::Settings)))
                                    }
                                    _ => {}
                                }
                            }
                            "exit" => commands.push(self.update(Msg::ConfirmExit)),
                            "hidden" => {
                                self.hidden = true;
                                self.images.cancel();
                                commands
                                    .push(self.call("window_visible", json!({"visible":false})));
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
                if refresh {
                    self.rebuild_rows();
                }
                if entries_changed {
                    self.rebuild_entries();
                }
                commands.push(self.load_images());
                return Task::batch(commands);
            }
            Msg::Page(page) => {
                let cancel = if self.is_modal()
                    && !matches!(page, Page::Add | Page::Advanced | Page::Detail)
                {
                    self.draft_changed(false)
                } else {
                    Task::none()
                };
                if matches!(page, Page::Add | Page::Advanced) && !self.is_modal() {
                    self.background_page = self.page;
                }
                if page == Page::Add && !self.is_modal() {
                    self.draft = self.settings.clone();
                    self.entries.clear();
                    self.selected_entries.clear();
                    self.preview = Value::Null;
                    self.preview_complete = false;
                    self.rebuild_entries();
                }
                if page == Page::Settings && self.page != Page::Advanced {
                    self.preferences = self.settings.clone();
                }
                if page == Page::Advanced {
                    self.previous = self.page;
                    self.advanced_input.clear();
                    self.raw_input = iced::widget::text_editor::Content::with_text(s(
                        self.active_settings(),
                        "raw_args",
                    ));
                }
                self.page = page;
                self.error.clear();
                self.rebuild_rows();
                let focus = if page == Page::Add {
                    iced::widget::operation::focus("url")
                } else if page == Page::Advanced {
                    iced::widget::operation::focus("advanced_search")
                } else {
                    Task::none()
                };
                return Task::batch([cancel, focus, self.load_images()]);
            }
            Msg::ModalClose => {
                if self.page == Page::Advanced {
                    if !self.field("option_help").is_empty() {
                        self.fields.remove("option_help");
                        return iced::widget::operation::focus("advanced_search");
                    }
                    if !self.field("repeat_editor").is_empty() {
                        self.fields.remove("repeat_editor");
                        return iced::widget::operation::focus("advanced_search");
                    }
                    // The original advanced editor writes into the parent's
                    // draft immediately; returning preserves edits and selection.
                    self.page = self.previous;
                    self.error.clear();
                    self.rebuild_rows();
                    return iced::widget::operation::focus(if self.page == Page::Add {
                        "url"
                    } else {
                        "preset_name"
                    });
                }
                let cancellation = if self.page == Page::Add {
                    self.draft_changed(false)
                } else {
                    Task::none()
                };
                self.page = self.background_page;
                self.fields.remove("repeat_editor");
                self.fields.remove("option_help");
                self.error.clear();
                self.rebuild_rows();
                return Task::batch([cancellation, iced::widget::operation::focus_next()]);
            }
            Msg::FocusNext(backwards) => {
                return if backwards {
                    iced::widget::operation::focus_previous()
                } else {
                    iced::widget::operation::focus_next()
                }
            }
            Msg::Text(key, value) => {
                if key == "args" {
                    self.tool_input = iced::widget::text_editor::Content::with_text(&value);
                }
                self.fields.insert(key.clone(), value);
                if key == "url" || key == "mode" {
                    return self.draft_changed(true);
                }
                if key == "search" || key == "filter" {
                    self.offsets.remove("tasks");
                    self.rebuild_rows();
                }
                if key == "entry_search" {
                    self.offsets.remove("entries");
                    self.rebuild_entries();
                }
                if key.starts_with("advanced_") {
                    self.offsets.remove("advanced");
                }
                if key == "advanced_native" {
                    self.fields.insert("advanced_group".into(), "全部".into());
                }
            }
            Msg::Setting(key, value) => {
                self.active_settings_mut()[&key] = value;
                if self.page == Page::Add || self.page == Page::Advanced {
                    return self.draft_changed(false);
                }
            }
            Msg::Preview => {
                if self.preview_busy
                    || (self.busy && self.estimate_id.is_empty())
                    || self.active_enqueue.is_some()
                {
                    return Task::none();
                };
                let cancel = self.draft_changed(true);
                self.error.clear();
                self.preview_busy = true;
                self.fields.remove("preview_status");
                self.offsets.remove("entries");
                self.pending_urls = self
                    .field("url")
                    .split_whitespace()
                    .filter(|x| x.starts_with("http://") || x.starts_with("https://"))
                    .map(str::to_owned)
                    .collect();
                if self.pending_urls.is_empty() {
                    self.preview_busy = false;
                    self.error = "请输入视频或合集链接".into();
                    return cancel;
                } else {
                    let next = self.preview_next();
                    return Task::batch([cancel, next]);
                }
            }
            Msg::CancelPreview => return self.draft_changed(false),
            Msg::Estimate => {
                if !self.estimate_id.is_empty() {
                    self.request += 1;
                    return self.invalidate_estimate();
                }
                if self.busy || !self.preview_complete || self.selected_entries.is_empty() {
                    return Task::none();
                }
                self.request += 1;
                self.busy = true;
                self.estimate_queue = self
                    .entries
                    .iter()
                    .filter(|e| self.selected_entries.contains(s(e, "id")))
                    .map(|e| (id(e), s(e, "url").into()))
                    .collect();
                self.estimate_total = self.estimate_queue.len();
                self.estimate_done = 0;
                return self.estimate_next();
            }
            Msg::ToggleEntry(id, selected) => {
                if selected {
                    self.selected_entries.insert(id);
                } else {
                    self.selected_entries.remove(&id);
                }
                self.rebuild_entries();
            }
            Msg::SelectVisible(selected) => {
                for &index in &self.visible_entries {
                    let item = &self.entries[index];
                    if !b(item, "_partial") && item["available"] != false {
                        if selected {
                            self.selected_entries.insert(id(item));
                        } else {
                            self.selected_entries.remove(s(item, "id"));
                        }
                    }
                }
                self.rebuild_entries();
            }
            Msg::Range => {
                match range_selection(self.field("range"), &self.entries) {
                    Ok(s) => self.selected_entries = s,
                    Err(e) => self.error = e,
                }
                self.rebuild_entries();
            }
            Msg::Submit => {
                if self.busy || self.active_enqueue.is_some() {
                    return Task::none();
                }
                invalidate_pending_submissions(
                    &mut self.enqueue_queue,
                    false,
                    &self.draft,
                    &self.selected_entries,
                );
                if !self.enqueue_queue.is_empty() {
                    self.busy = true;
                    return self.enqueue_next();
                }
                if !self.preview_complete || self.selected_entries.is_empty() {
                    return Task::none();
                };
                self.busy = true;
                self.enqueued_count = 0;
                self.enqueue_queue =
                    submission_batches(&self.entries, &self.selected_entries, &self.draft, || {
                        uuid::Uuid::new_v4().to_string()
                    });
                return self.enqueue_next();
            }
            Msg::ToggleTask(id, on) => {
                if on {
                    self.selected_tasks.insert(id);
                } else {
                    self.selected_tasks.remove(&id);
                }
            }
            Msg::SelectTasks(on) => {
                if on {
                    for row in &self.rows {
                        match row {
                            TaskRow::Item(id) => {
                                self.selected_tasks.insert(id.clone());
                            }
                            TaskRow::Group { ids, .. } => {
                                self.selected_tasks.extend(ids.iter().cloned())
                            }
                        }
                    }
                } else {
                    self.selected_tasks.clear();
                }
            }
            Msg::GroupSelect(ids, on) => {
                if on {
                    self.selected_tasks.extend(ids);
                } else {
                    for id in ids {
                        self.selected_tasks.remove(&id);
                    }
                }
            }
            Msg::Collapse(id) => {
                if !self.collapsed.remove(&id) {
                    self.collapsed.insert(id);
                }
                self.rebuild_rows();
            }
            Msg::Action(action) => {
                let ids: Vec<_> = self.selected_tasks.iter().cloned().collect();
                return self.call("task_action", json!({"ids":ids,"action":action}));
            }
            Msg::Queue => return self.call("queue_control", json!({"paused":!self.queue_paused})),
            Msg::AdvancedValue(name, input) => {
                self.advanced_input.insert(name.clone(), input.clone());
                if let Some(option) = self.catalog.iter().find(|o| s(&o.data, "name") == name) {
                    let value = if b(&option.data, "repeatable")
                        || option.data["nargs"].as_u64().unwrap_or(1) > 1
                    {
                        match serde_json::from_str::<Value>(&input) {
                            Ok(v) if v.is_array() => v,
                            _ => {
                                self.error =
                                    "重复或多值参数请使用 JSON 数组，例如 [\"en\",\"zh\"]".into();
                                return Task::none();
                            }
                        }
                    } else {
                        Value::String(input)
                    };
                    self.active_settings_mut()["advanced"][&name] = value;
                }
                return self.draft_changed(false);
            }
            Msg::AdvancedEnable(name, on) => {
                if !on {
                    return self.update(Msg::ResetOption(name));
                }
                if let Some(data) = self
                    .catalog
                    .iter()
                    .find(|o| s(&o.data, "name") == name)
                    .map(|o| o.data.clone())
                {
                    let count = data["nargs"].as_u64().unwrap_or(0);
                    let value = if count == 0 {
                        json!(true)
                    } else if count > 1 {
                        let values = vec![String::new(); count as usize];
                        if b(&data, "repeatable") {
                            json!([values])
                        } else {
                            json!(values)
                        }
                    } else if b(&data, "repeatable") {
                        json!([""])
                    } else {
                        data["choices"][0].as_str().unwrap_or("").into()
                    };
                    self.active_settings_mut()["advanced"][&name] = value;
                }
                return self.draft_changed(false);
            }
            Msg::AdvancedPick(name) => {
                let directory = self
                    .catalog
                    .iter()
                    .find(|o| s(&o.data, "name") == name)
                    .is_some_and(|o| {
                        matches!(name.as_str(), "--paths" | "--cache-dir" | "--plugin-dirs")
                            || s(&o.data, "metavar").to_lowercase().contains("dir")
                    });
                return self.call_tag(
                    &format!("pick:advanced:{name}"),
                    if directory {
                        "pick_folder"
                    } else {
                        "pick_file"
                    },
                    json!({"initial":self.active_settings()["advanced"][&name]}),
                    self.request,
                );
            }
            Msg::OptionHelp(name) => {
                if self.field("option_help") == name {
                    self.fields.remove("option_help");
                } else {
                    self.fields.insert("option_help".into(), name);
                }
            }
            Msg::ToolOption(name) => {
                if let Some(data) = self
                    .catalog
                    .iter()
                    .find(|o| s(&o.data, "name") == name)
                    .map(|o| o.data.clone())
                {
                    let args = if data["nargs"] == 0 {
                        name
                    } else {
                        format!(
                            "{} {}",
                            name,
                            serde_json::to_string(s(&data, "example")).unwrap_or_default()
                        )
                    };
                    self.tool_input = iced::widget::text_editor::Content::with_text(&args);
                    self.fields.insert("args".into(), args);
                    return self.update(Msg::Page(Page::Tools));
                }
            }
            Msg::RepeatAdd(name) => {
                let count = self
                    .catalog
                    .iter()
                    .find(|o| s(&o.data, "name") == name)
                    .and_then(|o| o.data["nargs"].as_u64())
                    .unwrap_or(1)
                    .max(1) as usize;
                let mut rows = self.repeat_rows(&name);
                rows.push(vec![String::new(); count]);
                self.store_repeat(&name, rows);
                return self.draft_changed(false);
            }
            Msg::RepeatRemove(name, index) => {
                let mut rows = self.repeat_rows(&name);
                if index < rows.len() {
                    rows.remove(index);
                }
                self.store_repeat(&name, rows);
                return self.draft_changed(false);
            }
            Msg::RepeatSet(name, index, part, value) => {
                let mut rows = self.repeat_rows(&name);
                if let Some(row) = rows.get_mut(index) {
                    if part < row.len() {
                        row[part] = value;
                    }
                }
                self.store_repeat(&name, rows);
                return self.draft_changed(false);
            }
            Msg::ResetOption(name) => {
                if let Some(o) = self.active_settings_mut()["advanced"].as_object_mut() {
                    o.remove(&name);
                }
                self.advanced_input.remove(&name);
                return self.draft_changed(false);
            }
            Msg::ResetAdvanced => {
                self.active_settings_mut()["advanced"] = json!({});
                self.active_settings_mut()["raw_args"] = "".into();
                self.raw_input = iced::widget::text_editor::Content::new();
                self.advanced_input.clear();
                return self.draft_changed(false);
            }
            Msg::AdvancedBack => {
                return self.call_tag(
                    "advanced_apply",
                    "validate_settings",
                    json!({"settings":self.active_settings(),"context":"download"}),
                    self.request,
                )
            }
            Msg::ApplyProfile(id) => {
                if let Some(profile) = self.profiles.iter().find(|p| s(p, "id") == id) {
                    let mut settings = normalized(&profile["settings"]);
                    settings["directory"] = self.active_settings()["directory"].clone();
                    *self.active_settings_mut() = settings;
                    self.advanced_input.clear();
                    self.raw_input = iced::widget::text_editor::Content::with_text(s(
                        self.active_settings(),
                        "raw_args",
                    ));
                    return self.draft_changed(false);
                }
            }
            Msg::SaveProfile => {
                let name = self.field("preset_name").trim();
                if name.is_empty() {
                    self.error = "请输入预设名称".into()
                } else {
                    return self.call(
                        "profiles_save",
                        json!({"name":name,"settings":self.active_settings()}),
                    );
                }
            }
            Msg::DeleteProfile(id) => return self.call("profiles_delete", json!({"id":id})),
            Msg::SaveSettings => {
                if self.busy_operations.contains("save_settings") {
                    return Task::none();
                }
                self.busy_operations.insert("save_settings".into());
                return self.call("save_settings", json!({"settings":self.preferences}));
            }
            Msg::SaveAdvancedDefault => {
                if self.busy_operations.contains("save_settings") {
                    return Task::none();
                }
                self.busy_operations.insert("save_settings".into());
                return self.call("save_settings", json!({"settings":self.active_settings()}));
            }
            Msg::ImportSettings => return self.call("import_file", json!({})),
            Msg::ExportSettings => {
                let profiles:Vec<_>=self.profiles.iter().map(|p|json!({"id":p["id"],"name":p["name"],"settings":self.clean_settings(&p["settings"])})).collect();
                return self.call("export_file",json!({"name":"流光设置.json","content":serde_json::to_string_pretty(&json!({"version":1,"settings":self.clean_settings(&self.preferences),"profiles":profiles})).unwrap_or_default()}));
            }
            Msg::RunCommand => {
                if self.busy_operations.contains("run_command") {
                    return Task::none();
                }
                self.busy_operations.insert("run_command".into());
                return self.call("run_command",json!({"args":self.field("args"),"stdin":self.field("stdin"),"settings":self.settings}));
            }
            Msg::Engine(op) => {
                if self
                    .busy_operations
                    .iter()
                    .any(|operation| operation.starts_with("engine_") || operation == &op)
                {
                    return Task::none();
                }
                self.busy_operations.insert(op.clone());
                return self.call(&op, json!({}));
            }
            Msg::Pick(key) => {
                let op = if key == "directory" {
                    "pick_folder"
                } else if key == "cookie_file" {
                    "pick_cookie"
                } else {
                    "pick_file"
                };
                return self.call_tag(
                    &format!("pick:{key}"),
                    op,
                    json!({"initial":self.active_settings()[&key]}),
                    self.request,
                );
            }
            Msg::Detail(id) => {
                if !self.is_modal() {
                    self.background_page = self.page;
                }
                self.detail_id = id;
                self.page = Page::Detail;
                self.detail_revision += 1;
                self.logs = Value::Null;
                self.show_output = false;
                return self.call_tag(
                    "logs",
                    "logs",
                    json!({"id":self.detail_id}),
                    self.detail_revision,
                );
            }
            Msg::DetailAction(action) => {
                return self.call(
                    "task_action",
                    json!({"ids":[self.detail_id],"action":action}),
                )
            }
            Msg::RevealOutput => self.show_output = !self.show_output,
            Msg::ExportLogs => {
                let content = if self.show_output {
                    s(&self.logs, "output").to_string()
                } else {
                    self.logs["lines"]
                        .as_array()
                        .map(|v| {
                            v.iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default()
                };
                return self.call(
                    "export_file",
                    json!({"name":"流光诊断.txt","content":content}),
                );
            }
            Msg::CopyLogs => {
                let content = if self.show_output {
                    s(&self.logs, "output").to_string()
                } else {
                    self.logs["lines"]
                        .as_array()
                        .map(|v| {
                            v.iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default()
                };
                return iced::clipboard::write(content);
            }
            Msg::Open(path) => return self.call("open_path", json!({"path":path})),
            Msg::Dismiss => {
                self.notice.clear();
                self.error.clear();
            }
            Msg::Scroll(key, y, h) => {
                self.offsets.insert(key, (y, h));
                return self.load_images();
            }
            Msg::Close(id) => {
                // AppKit initialization may be awaiting the main queue. Keep
                // that queue alive until the worker can finish, then quit via
                // the same Service shutdown path as a normal menu quit.
                if cfg!(target_os = "macos") && self.initializing {
                    self.exit_after_init = true;
                    return Task::none();
                }
                if self
                    .service
                    .as_ref()
                    .is_none_or(|service| service.is_closed())
                {
                    flow_platform::desktop::prepare_exit();
                    return iced::exit();
                }
                self.window_id = Some(id);
                return self.call_tag(
                    "save_before_close",
                    "save_window_state",
                    self.window_state.clone(),
                    self.request,
                );
            }
            Msg::ConfirmExit => {
                if cfg!(target_os = "macos") && self.initializing {
                    self.exit_after_init = true;
                    return Task::none();
                }
                if self
                    .service
                    .as_ref()
                    .is_none_or(|service| service.is_closed())
                {
                    flow_platform::desktop::prepare_exit();
                    return iced::exit();
                }
                return self.call_tag(
                    "save_before_exit",
                    "save_window_state",
                    self.window_state.clone(),
                    self.request,
                );
            }
            #[cfg(target_os = "macos")]
            Msg::CleanApplicationData => {
                return self.call("clean_application_data", json!({}));
            }
            Msg::EditArgs(action) => {
                self.tool_input.perform(action);
                self.fields.insert("args".into(), self.tool_input.text());
            }
            Msg::EditStdin(action) => {
                self.stdin_input.perform(action);
                self.fields.insert("stdin".into(), self.stdin_input.text());
            }
            Msg::EditRaw(action) => {
                let edit = action.is_edit();
                self.raw_input.perform(action);
                if edit {
                    self.active_settings_mut()["raw_args"] = self.raw_input.text().into();
                    return self.draft_changed(false);
                }
            }
            Msg::Shortcut(key, modifiers) => {
                if key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)
                    && self.is_modal()
                {
                    return self.update(Msg::ModalClose);
                }
                if modifiers.command() {
                    match key.as_ref() {
                        iced::keyboard::Key::Character("n") => {
                            return self.update(Msg::Page(Page::Add))
                        }
                        iced::keyboard::Key::Character(",") => {
                            return self.update(Msg::Page(Page::Settings))
                        }
                        iced::keyboard::Key::Character("q") if cfg!(target_os = "macos") => {
                            return self.update(Msg::ConfirmExit)
                        }
                        iced::keyboard::Key::Character("w") if cfg!(target_os = "macos") => {
                            if let Some(id) = self.window_id {
                                return self.update(Msg::Close(id));
                            }
                        }
                        iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter)
                            if self.page == Page::Add =>
                        {
                            return self.update(if self.preview_complete {
                                Msg::Submit
                            } else {
                                Msg::Preview
                            })
                        }
                        _ => {}
                    }
                }
            }
        }
        self.load_images()
    }
    fn load_images(&mut self) -> Task<Msg> {
        if self.hidden {
            self.images.cancel();
            return Task::none();
        }
        let settings = if self.page == Page::Add {
            &self.draft
        } else {
            &self.settings
        };
        if self
            .network_cache
            .as_ref()
            .is_none_or(|(cached, _)| cached != settings)
        {
            self.network_cache = Some((settings.clone(), flow_app::effective_proxy(settings)));
        }
        let (mode, proxy) = match self
            .network_cache
            .as_ref()
            .expect("initialized above")
            .1
            .clone()
        {
            Ok(network) => network,
            Err(_) => {
                self.images.configure("invalid-settings".into());
                self.images.cancel();
                return Task::none();
            }
        };
        self.images.configure(format!("{mode}\n{proxy}"));
        let urls: Vec<String> = if self.page == Page::Add {
            let (y, h) = self.offsets.get("entries").copied().unwrap_or((0., 700.));
            let start = ((y / 60.) as usize).saturating_sub(3);
            self.visible_entries
                .iter()
                .skip(start)
                .take((h / 60.) as usize + 8)
                .map(|&i| s(&self.entries[i], "thumbnail").to_string())
                .collect()
        } else if matches!(self.page, Page::Active | Page::History) {
            let (y, h) = self.offsets.get("tasks").copied().unwrap_or((0., 700.));
            let start = ((y / 76.) as usize).saturating_sub(3);
            self.rows
                .iter()
                .skip(start)
                .take((h / 76.) as usize + 8)
                .filter_map(|r| {
                    if let TaskRow::Item(id) = r {
                        self.tasks.tasks.get(id)
                    } else {
                        None
                    }
                })
                .map(|e| s(e, "thumbnail").to_string())
                .collect()
        } else {
            Vec::new()
        };
        let generation = self.images.generation;
        Task::batch(
            self.images
                .request(urls)
                .into_iter()
                .map(|(url, registration)| {
                    let future = images::load(url.clone(), mode.clone(), proxy.clone());
                    Task::perform(
                        async move {
                            iced::futures::future::Abortable::new(future, registration)
                                .await
                                .unwrap_or_else(|_| Err("cancelled".into()))
                        },
                        move |r| Msg::Image(generation, url.clone(), r),
                    )
                }),
        )
    }
    fn repeat_rows(&self, name: &str) -> Vec<Vec<String>> {
        let option = self.catalog.iter().find(|o| s(&o.data, "name") == name);
        let multi = option.is_some_and(|o| o.data["nargs"].as_u64().unwrap_or(1) > 1);
        let repeat = option.is_some_and(|o| b(&o.data, "repeatable"));
        let value = &self.active_settings()["advanced"][name];
        if multi && !repeat {
            return vec![value
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|v| v.as_str().unwrap_or("").to_string())
                        .collect()
                })
                .unwrap_or_else(|| {
                    vec![
                        String::new();
                        option.and_then(|o| o.data["nargs"].as_u64()).unwrap_or(1) as usize
                    ]
                })];
        }
        value
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|v| {
                        if let Some(values) = v.as_array() {
                            values
                                .iter()
                                .map(|v| v.as_str().unwrap_or("").to_string())
                                .collect()
                        } else {
                            vec![v.as_str().unwrap_or("").to_string()]
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    fn store_repeat(&mut self, name: &str, rows: Vec<Vec<String>>) {
        let option = self.catalog.iter().find(|o| s(&o.data, "name") == name);
        let multi = option.and_then(|o| o.data["nargs"].as_u64()).unwrap_or(1) > 1;
        let repeat = option.is_some_and(|o| b(&o.data, "repeatable"));
        if rows.is_empty() {
            if let Some(values) = self.active_settings_mut()["advanced"].as_object_mut() {
                values.remove(name);
            }
        } else if multi {
            self.active_settings_mut()["advanced"][name] =
                if repeat { json!(rows) } else { json!(rows[0]) };
        } else {
            self.active_settings_mut()["advanced"][name] = json!(rows
                .into_iter()
                .map(|r| r.into_iter().next().unwrap_or_default())
                .collect::<Vec<_>>());
        }
        self.advanced_input.remove(name);
    }
    fn import_next_profile(&mut self) -> Task<Msg> {
        if let Some(profile) = self.imported_profiles.pop_front() {
            self.call_tag(
                "import_profile",
                "profiles_save",
                json!({"name":profile["name"],"settings":profile["settings"]}),
                self.request,
            )
        } else {
            Task::none()
        }
    }
    fn subscription(&self) -> Subscription<Msg> {
        let mut subscriptions = vec![
            window::close_requests().map(Msg::Close),
            window::events().filter_map(|(id, event)| match event {
                window::Event::Moved(_) | window::Event::Resized(_) => {
                    Some(Msg::WindowEvent(id, event))
                }
                _ => None,
            }),
            iced::system::theme_changes().map(Msg::SystemTheme),
            iced::event::listen_with(|event, status, _| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                    modifiers,
                    ..
                }) if status == iced::event::Status::Ignored => {
                    Some(Msg::FocusNext(modifiers.shift()))
                }
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    modifiers,
                    ..
                }) if status == iced::event::Status::Ignored => Some(Msg::Shortcut(
                    iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    modifiers,
                )),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key, modifiers, ..
                }) if modifiers.command() => Some(Msg::Shortcut(key, modifiers)),
                _ => None,
            }),
        ];
        if let Some(service) = &self.service {
            subscriptions.push(Subscription::run_with(
                EventSource(service.clone()),
                event_stream,
            ));
        }
        Subscription::batch(subscriptions)
    }
    fn theme(&self) -> Theme {
        let dark = match s(&self.settings, "theme") {
            "dark" => true,
            "light" => false,
            _ => self.system_dark,
        };
        if dark {
            Theme::custom(
                "流光·深色",
                iced::theme::Palette {
                    background: iced::Color::from_rgb8(35, 36, 36),
                    text: iced::Color::from_rgb8(233, 231, 226),
                    primary: iced::Color::from_rgb8(188, 104, 77),
                    success: iced::Color::from_rgb8(135, 167, 187),
                    warning: iced::Color::from_rgb8(200, 151, 88),
                    danger: iced::Color::from_rgb8(237, 149, 132),
                },
            )
        } else {
            Theme::custom(
                "流光",
                iced::theme::Palette {
                    background: iced::Color::from_rgb8(251, 250, 248),
                    text: iced::Color::from_rgb8(41, 43, 44),
                    primary: iced::Color::from_rgb8(168, 77, 54),
                    success: iced::Color::from_rgb8(88, 123, 144),
                    warning: iced::Color::from_rgb8(166, 120, 55),
                    danger: iced::Color::from_rgb8(167, 66, 53),
                },
            )
        }
    }
    fn view(&self) -> Element<'_, Msg> {
        self.shell_view()
    }
}
impl Drop for App {
    fn drop(&mut self) {
        if let Some(service) = &self.service {
            service.shutdown();
        }
    }
}

struct EventSource(Arc<Service>);
impl std::hash::Hash for EventSource {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&(Arc::as_ptr(&self.0) as usize), state);
    }
}
fn event_stream(source: &EventSource) -> impl iced::futures::Stream<Item = Msg> {
    let service = source.0.clone();
    iced::stream::channel(
        8,
        move |mut output: iced::futures::channel::mpsc::Sender<Msg>| async move {
            loop {
                service.wait_events().await;
                if service.is_closed() {
                    break;
                }
                let events = service.drain_events();
                if !events.is_empty() && output.send(Msg::Events(events)).await.is_err() {
                    break;
                }
            }
        },
    )
}
