// SPDX-License-Identifier: Apache-2.0
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};

/// Carry state only between rows with the same identity, including when a
/// virtual window shifts without changing its length. Offscreen state is dropped.
pub fn reconcile_visible_rows<T>(
    previous_keys: Vec<String>,
    previous_states: Vec<T>,
    next_keys: &[String],
    mut new_state: impl FnMut(usize) -> T,
) -> Vec<T> {
    let mut previous: HashMap<String, T> = previous_keys.into_iter().zip(previous_states).collect();
    next_keys
        .iter()
        .enumerate()
        .map(|(index, key)| previous.remove(key).unwrap_or_else(|| new_state(index)))
        .collect()
}

/// A failed batch can reuse its idempotency key only while its reviewed draft
/// and selected entries still match. Never replace an in-flight request.
pub fn invalidate_pending_submissions(
    pending: &mut VecDeque<Value>,
    in_flight: bool,
    settings: &Value,
    selected: &HashSet<String>,
) -> bool {
    if in_flight || pending.is_empty() {
        return false;
    }
    let pending_ids: HashSet<String> = pending
        .iter()
        .flat_map(|batch| batch["entries"].as_array().into_iter().flatten())
        .map(id)
        .collect();
    if pending.iter().all(|batch| batch["settings"] == *settings) && pending_ids == *selected {
        return false;
    }
    pending.clear();
    true
}

pub fn acknowledge_submission(batch: &Value, selected: &mut HashSet<String>) {
    for entry in batch["entries"].as_array().into_iter().flatten() {
        selected.remove(s(entry, "id"));
    }
}

/// Build source groups once from the remaining selection. IDs are injected so
/// recovery can be verified without a GUI, filesystem, or random generator.
pub fn submission_batches(
    entries: &[Value],
    selected: &HashSet<String>,
    settings: &Value,
    mut new_id: impl FnMut() -> String,
) -> VecDeque<Value> {
    let mut groups: Vec<Vec<Value>> = Vec::new();
    let mut lookup = HashMap::new();
    let mut totals: HashMap<&str, usize> = HashMap::new();
    for entry in entries {
        let source = s(entry, "_batch_id");
        *totals.entry(source).or_default() += 1;
        if !selected.contains(s(entry, "id")) || b(entry, "_partial") || entry["available"] == false
        {
            continue;
        }
        let index = *lookup.entry(source).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[index].push(entry.clone());
    }
    groups.into_iter().map(|items| {
        let first=&items[0];
        let title=if b(first,"_is_playlist") || items.len()>1 { first["collection_title"].clone() } else { Value::Null };
        let token=new_id();
        json!({"entries":items,"settings":settings,"batch_title":title,"batch_id":token,"batch_total":first["_batch_total"].as_u64().unwrap_or(totals[s(first,"_batch_id")]as u64),"collection_id":first["collection_id"],"request_id":format!("submit-{token}")})
    }).collect()
}

pub fn defaults() -> Value {
    json!({"directory":"","quality":"best","format":"auto","concurrency":2,
        "proxy_mode":"system","proxy":"","cookie_file":"","cookie_browser":"",
        "subtitles":false,"thumbnail":false,"theme":"system","close_to_tray":true,
        "notifications":true,"auto_fallback":false,"advanced":{},"raw_args":""})
}
pub fn normalized(value: &Value) -> Value {
    let mut result = defaults();
    if let Some(fields) = value.as_object() {
        for (key, value) in fields {
            result[key] = value.clone();
        }
    }
    result
}
pub fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
pub fn b(value: &Value, key: &str) -> bool {
    value[key].as_bool().unwrap_or(false)
}
pub fn id(value: &Value) -> String {
    s(value, "id").to_string()
}
pub fn terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "cancelled" | "interrupted" | "skipped"
    )
}
pub fn status(status: &str) -> &str {
    match status {
        "queued" => "排队中",
        "starting" => "准备中",
        "downloading" => "下载中",
        "processing" => "处理中",
        "completed" => "已完成",
        "failed" => "失败",
        "cancelled" => "已取消",
        "interrupted" => "已中断",
        "skipped" => "已跳过",
        _ => status,
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}
pub fn choice(value: &str) -> Choice {
    Choice {
        value: value.into(),
        label: match value {
            "best" => "最佳画质",
            "auto" => "自动",
            "video" => "仅当前视频",
            "playlist" => "整个合集",
            "system" => "跟随系统",
            "direct" => "直接连接",
            "custom" => "自定义代理",
            "light" => "浅色",
            "dark" => "深色",
            "all" => "全部",
            "common" => "常用",
            "changed" => "已修改",
            "" => "不使用",
            v if terminal(v) => status(v),
            v => v,
        }
        .into(),
    }
}
pub fn bytes(value: Option<f64>) -> String {
    let Some(value) = value.filter(|x| x.is_finite() && *x >= 0.) else {
        return "未知".into();
    };
    if value < 1024. {
        return format!("{value:.0} B");
    }
    let units = ["KiB", "MiB", "GiB", "TiB"];
    let mut n = value / 1024.;
    let mut i = 0;
    while n >= 1024. && i < 3 {
        n /= 1024.;
        i += 1;
    }
    format!("{n:.2} {}", units[i])
}
pub fn size(value: &Value) -> String {
    let n = value["size"].as_f64();
    format!(
        "{}{}",
        if n.is_some() && b(value, "approximate") {
            "约 "
        } else {
            ""
        },
        bytes(n)
    )
}
pub fn duration(seconds: Option<f64>) -> String {
    match seconds {
        None => "—".into(),
        Some(n) => {
            let n = n.max(0.) as u64;
            format!("{}:{:02}:{:02}", n / 3600, n / 60 % 60, n % 60)
        }
    }
}
pub fn selected_size(entries: &[Value], selected: &HashSet<String>) -> String {
    let mut sum = 0.;
    let mut unknown = 0;
    let mut approx = false;
    let mut count = 0;
    for e in entries.iter().filter(|e| selected.contains(s(e, "id"))) {
        count += 1;
        if let Some(n) = e["size"].as_f64() {
            sum += n;
            approx |= b(e, "approximate")
        } else {
            unknown += 1
        }
    }
    if unknown == count {
        "大小未知".into()
    } else if unknown > 0 {
        format!("已知 {}，另有 {unknown} 项未知", bytes(Some(sum)))
    } else {
        format!("{}{}", if approx { "约 " } else { "" }, bytes(Some(sum)))
    }
}
pub fn range_selection(input: &str, entries: &[Value]) -> Result<HashSet<String>, String> {
    let mut ranges = Vec::new();
    for token in input
        .replace('，', ",")
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
    {
        let parts = token.split('-').map(str::trim).collect::<Vec<_>>();
        if parts.len() > 2 {
            return Err(format!("无效范围：{token}"));
        }
        let start = parts[0]
            .parse::<u64>()
            .map_err(|_| format!("无效集号：{token}"))?;
        let end = parts
            .get(1)
            .map_or(Ok(start), |x| x.parse::<u64>())
            .map_err(|_| format!("无效集号：{token}"))?;
        if start == 0 || end < start || end - start > 100_000 {
            return Err(format!("无效范围：{token}"));
        }
        ranges.push((start, end));
    }
    if ranges.is_empty() {
        return Err("请输入集号，例如 1-10,15".into());
    }
    Ok(entries
        .iter()
        .filter(|e| {
            !b(e, "_partial")
                && e["available"] != false
                && e["index"]
                    .as_u64()
                    .is_some_and(|n| ranges.iter().any(|&(a, b)| n >= a && n <= b))
        })
        .map(id)
        .collect())
}
pub fn exportable(settings: &Value) -> Value {
    let mut value = settings.clone();
    value["proxy"] = "".into();
    value["cookie_file"] = "".into();
    value["cookie_browser"] = "".into();
    value["raw_args"] = "".into();
    if let Some(fields) = value["advanced"].as_object_mut() {
        fields.retain(|key, _| {
            ![
                "password",
                "username",
                "cookie",
                "netrc",
                "header",
                "token",
                "proxy",
                "ap-mso",
                "certificate",
            ]
            .iter()
            .any(|part| key.contains(part))
        });
    }
    value
}
pub fn section(group: &str, name: &str) -> &'static str {
    if name.contains("audio") {
        return "音频";
    }
    match group {
        "Video Format Options" => "画质",
        "Subtitle Options" => "字幕",
        "Filesystem Options" | "Thumbnail Options" | "Internet Shortcut Options" => "文件",
        "Video Selection" => "合集",
        "Network Options" | "Download Options" => "网络",
        "Authentication Options" | "Geo-restriction" | "Workarounds" => "访问与认证",
        "Post-Processing Options" | "SponsorBlock Options" => "后处理",
        _ => "其他",
    }
}
#[derive(Debug, Clone)]
pub struct CatalogOption {
    pub data: Value,
    pub group: String,
    pub section: String,
    pub search: String,
}
impl CatalogOption {
    pub fn flatten(catalog: &Value) -> Vec<Self> {
        let mut result = Vec::new();
        for group in catalog["groups"].as_array().into_iter().flatten() {
            for data in group["options"].as_array().into_iter().flatten() {
                let mut data = data.clone();
                data["common"] = matches!(
                    s(&data, "name"),
                    "--format"
                        | "--format-sort"
                        | "--merge-output-format"
                        | "--extract-audio"
                        | "--audio-format"
                        | "--audio-quality"
                        | "--write-subs"
                        | "--write-auto-subs"
                        | "--sub-langs"
                        | "--embed-subs"
                        | "--convert-subs"
                        | "--output"
                        | "--paths"
                        | "--download-archive"
                        | "--playlist-items"
                        | "--dateafter"
                        | "--datebefore"
                        | "--match-filter"
                        | "--max-downloads"
                        | "--restrict-filenames"
                        | "--windows-filenames"
                        | "--trim-filenames"
                        | "--proxy"
                        | "--limit-rate"
                        | "--retries"
                        | "--fragment-retries"
                        | "--socket-timeout"
                        | "--concurrent-fragments"
                        | "--cookies"
                        | "--cookies-from-browser"
                        | "--extractor-args"
                        | "--embed-thumbnail"
                        | "--embed-metadata"
                        | "--embed-chapters"
                        | "--sponsorblock-remove"
                        | "--remux-video"
                        | "--download-sections"
                )
                .into();
                result.push(Self {
                    group: s(group, "label").into(),
                    section: section(s(group, "id"), s(&data, "name")).into(),
                    search: format!(
                        "{} {} {} {} {}",
                        s(&data, "name"),
                        s(&data, "label"),
                        s(&data, "description"),
                        s(&data, "help"),
                        data["aliases"]
                    )
                    .to_lowercase(),
                    data,
                });
            }
        }
        result
    }
}
#[derive(Debug, Clone)]
pub enum TaskRow {
    Group {
        id: String,
        title: String,
        count: usize,
        total: usize,
        ids: Vec<String>,
    },
    Item(String),
}
#[derive(Default)]
pub struct TaskStore {
    pub tasks: HashMap<String, Value>,
    pub order: Vec<String>,
    pub active_count: usize,
}
impl TaskStore {
    pub fn replace(&mut self, items: &Value, revision: u64) {
        let newer: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| self.tasks.get(id))
            .filter(|v| v["_revision"].as_u64().unwrap_or(0) > revision)
            .cloned()
            .collect();
        self.tasks.clear();
        self.order.clear();
        self.active_count = 0;
        for task in items.as_array().into_iter().flatten() {
            self.upsert(task.clone());
        }
        for task in newer {
            self.upsert(task);
        }
    }
    pub fn upsert(&mut self, task: Value) -> bool {
        let key = id(&task);
        if self.tasks.get(&key).is_some_and(|existing| {
            existing["_revision"].as_u64().unwrap_or(0) > task["_revision"].as_u64().unwrap_or(0)
        }) {
            return false;
        }
        let membership_changed = self.tasks.get(&key).is_none_or(|previous| {
            ["status", "title", "url", "batch_id", "batch_title", "index"]
                .iter()
                .any(|key| previous[*key] != task[*key])
        });
        if self
            .tasks
            .get(&key)
            .is_some_and(|previous| !terminal(s(previous, "status")))
        {
            self.active_count = self.active_count.saturating_sub(1);
        }
        if !terminal(s(&task, "status")) {
            self.active_count += 1;
        }
        if !self.tasks.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.tasks.insert(key, task);
        membership_changed
    }
    pub fn rows(
        &self,
        history: bool,
        filter: &str,
        search: &str,
        collapsed: &HashSet<String>,
    ) -> Vec<TaskRow> {
        let query = search.to_lowercase();
        let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
        let mut index = HashMap::new();
        for key in self.order.iter().rev() {
            let Some(task) = self.tasks.get(key) else {
                continue;
            };
            if terminal(s(task, "status")) != history
                || (history && filter != "all" && s(task, "status") != filter)
            {
                continue;
            }
            if !query.is_empty()
                && !format!("{} {}", s(task, "title"), s(task, "url"))
                    .to_lowercase()
                    .contains(&query)
            {
                continue;
            }
            let group = s(task, "batch_id");
            let group = if group.is_empty() {
                key.as_str()
            } else {
                group
            };
            let i = *index.entry(group.to_string()).or_insert_with(|| {
                groups.push((group.into(), Vec::new()));
                groups.len() - 1
            });
            groups[i].1.push(task);
        }
        let mut rows = Vec::new();
        for (group, mut tasks) in groups {
            if !s(tasks[0], "batch_id").is_empty() {
                rows.push(TaskRow::Group {
                    id: group.clone(),
                    title: s(tasks[0], "batch_title").into(),
                    count: tasks.len(),
                    total: tasks
                        .iter()
                        .filter_map(|t| t["batch_total"].as_u64())
                        .max()
                        .unwrap_or(tasks.len() as u64) as usize,
                    ids: tasks.iter().map(|t| id(t)).collect(),
                });
                if collapsed.contains(&group) {
                    continue;
                }
                tasks.sort_by_key(|v| v["index"].as_u64().unwrap_or(u64::MAX));
            }
            rows.extend(tasks.into_iter().map(|v| TaskRow::Item(id(v))));
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Debug, PartialEq, Eq)]
    struct InputState {
        focused: bool,
        selection: Option<(usize, usize)>,
    }
    fn empty_input() -> InputState {
        InputState {
            focused: false,
            selection: None,
        }
    }
    #[test]
    fn equally_sized_virtual_window_preserves_focus_on_identity() {
        let previous_keys = ["option:a", "option:b", "option:c"]
            .map(str::to_owned)
            .to_vec();
        let next_keys = ["option:b", "option:c", "option:d"]
            .map(str::to_owned)
            .to_vec();
        let states = vec![
            empty_input(),
            InputState {
                focused: true,
                selection: Some((2, 6)),
            },
            empty_input(),
        ];
        let mut created = 0;
        let next = reconcile_visible_rows(previous_keys, states, &next_keys, |_| {
            created += 1;
            empty_input()
        });
        assert_eq!(created, 1);
        assert_eq!(
            next[0],
            InputState {
                focused: true,
                selection: Some((2, 6))
            }
        );
        assert!(!next[1].focused);
        assert_eq!(next[2], empty_input());
    }
    #[test]
    fn filtered_out_input_does_not_transfer_or_recover_hidden_focus() {
        let original = ["option:a", "option:b", "option:c"]
            .map(str::to_owned)
            .to_vec();
        let filtered = ["option:c", "option:a"].map(str::to_owned).to_vec();
        let states = vec![
            empty_input(),
            InputState {
                focused: true,
                selection: Some((0, 4)),
            },
            empty_input(),
        ];
        let next = reconcile_visible_rows(original, states, &filtered, |_| empty_input());
        assert!(next.iter().all(|state| !state.focused));
        let restored =
            reconcile_visible_rows(filtered, next, &["option:b".into()], |_| empty_input());
        assert_eq!(restored, vec![empty_input()]);
    }
    #[test]
    fn range_keeps_original_indices() {
        let entries = vec![
            json!({"id":"a","index":7}),
            json!({"id":"b","index":9,"available":false}),
        ];
        assert_eq!(
            range_selection("7-9", &entries).unwrap(),
            HashSet::from(["a".into()])
        );
        assert!(range_selection("8-2", &entries).is_err());
    }
    #[test]
    fn unknown_not_presented_as_total() {
        let entries = vec![json!({"id":"a","size":1024}), json!({"id":"b","size":null})];
        let selected = HashSet::from(["a".into(), "b".into()]);
        assert!(selected_size(&entries, &selected).contains("未知"));
    }
    #[test]
    fn export_removes_credentials() {
        let mut settings = defaults();
        settings["advanced"] = json!({"--password":"secret","--retries":"4"});
        settings["raw_args"] = "secret".into();
        let e = exportable(&settings);
        assert!(e["advanced"].get("--password").is_none());
        assert_eq!(e["advanced"]["--retries"], "4");
        assert_eq!(e["raw_args"], "");
    }
    #[test]
    fn snapshot_keeps_newer_terminal_event() {
        let mut store = TaskStore::default();
        store.upsert(json!({"id":"a","status":"completed","_revision":9}));
        store.replace(&json!([{"id":"a","status":"downloading","_revision":6}]), 6);
        assert_eq!(store.tasks["a"]["status"], "completed");
        store.replace(&json!([]), 8);
        assert!(store.tasks.contains_key("a"));
        store.replace(&json!([]), 10);
        assert!(store.tasks.is_empty());
    }
    #[test]
    fn progress_does_not_rebuild_membership() {
        let mut store = TaskStore::default();
        assert!(store.upsert(json!({"id":"a","status":"downloading","progress":1,"_revision":1})));
        assert!(!store.upsert(json!({"id":"a","status":"downloading","progress":90,"_revision":2})));
        assert!(!store.upsert(json!({"id":"a","status":"queued","_revision":1})));
        assert_eq!(store.tasks["a"]["progress"], 90);
    }
    #[test]
    fn partial_entries_cannot_be_selected() {
        assert!(
            range_selection("1", &[json!({"id":"a","index":1,"_partial":true})])
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn unchanged_failed_batch_reuses_its_idempotency_key() {
        let settings = defaults();
        let selected = HashSet::from(["b".into()]);
        let mut pending = VecDeque::from([
            json!({"request_id":"original-key","settings":settings,"entries":[{"id":"b"}]}),
        ]);
        assert!(!invalidate_pending_submissions(
            &mut pending,
            false,
            &settings,
            &selected
        ));
        assert_eq!(pending.front().unwrap()["request_id"], "original-key");
    }
    #[test]
    fn corrected_draft_rebuilds_only_unsubmitted_entries() {
        let entries = vec![
            json!({"id":"a","_batch_id":"first","index":7}),
            json!({"id":"b","_batch_id":"second","index":8,"_batch_total":100}),
        ];
        let mut selected = HashSet::from(["a".into(), "b".into()]);
        let mut settings = defaults();
        settings["directory"] = "invalid".into();
        let mut serial = 0;
        let mut pending = submission_batches(&entries, &selected, &settings, || {
            serial += 1;
            format!("old-{serial}")
        });
        let completed = pending.pop_front().unwrap();
        acknowledge_submission(&completed, &mut selected);
        let old_key = pending.front().unwrap()["request_id"].clone();
        settings["directory"] = "D:/Videos".into();
        settings["quality"] = "1080".into();
        settings["advanced"] = json!({"--embed-subs":true});
        assert!(invalidate_pending_submissions(
            &mut pending,
            false,
            &settings,
            &selected
        ));
        let rebuilt = submission_batches(&entries, &selected, &settings, || "new-draft".into());
        assert_eq!(rebuilt.len(), 1);
        let remaining = &rebuilt[0];
        assert_eq!(remaining["entries"][0]["id"], "b");
        assert_eq!(remaining["entries"][0]["index"], 8);
        assert_eq!(remaining["batch_total"], 100);
        assert_eq!(remaining["settings"], settings);
        assert_ne!(remaining["request_id"], old_key);
        assert_eq!(selected, HashSet::from(["b".into()]));
    }
    #[test]
    fn changed_selection_invalidates_pending_but_never_in_flight_batch() {
        let settings = defaults();
        let mut pending = VecDeque::from([
            json!({"request_id":"keep-active","settings":settings,"entries":[{"id":"a"},{"id":"b"}]}),
        ]);
        let selected = HashSet::from(["b".into()]);
        assert!(!invalidate_pending_submissions(
            &mut pending,
            true,
            &settings,
            &selected
        ));
        assert_eq!(pending.len(), 1);
        assert!(invalidate_pending_submissions(
            &mut pending,
            false,
            &settings,
            &selected
        ));
        assert!(pending.is_empty());
    }
}
