// SPDX-License-Identifier: Apache-2.0
use serde_json::{json, Value};
use std::path::Path;
use std::sync::OnceLock;

pub fn catalog() -> Value {
    serde_json::from_str(include_str!("../../../engine/catalog/options.json"))
        .expect("packaged option catalog")
}

pub fn defaults(data: &Path) -> Value {
    json!({"directory":data.join("downloads").to_string_lossy(),"quality":"best","format":"auto","concurrency":2,
        "proxy_mode":"system","proxy":"","cookie_file":"","cookie_browser":"","subtitles":false,"thumbnail":false,
        "theme":"system","close_to_tray":true,"auto_fallback":false,"advanced":{},"raw_args":""})
}

pub fn normalize(base: &Value, incoming: &Value) -> Result<Value, String> {
    let mut result = base.clone();
    let target = result.as_object_mut().ok_or("设置数据损坏")?;
    let source = incoming.as_object().ok_or("设置必须为对象")?;
    for (k, v) in source {
        target.insert(k.clone(), v.clone());
    }
    let n = result["concurrency"].as_u64().ok_or("并发数必须为整数")?;
    if !(1..=8).contains(&n) {
        return Err("并发数应为 1–8".into());
    }
    for key in [
        "directory",
        "quality",
        "format",
        "proxy_mode",
        "proxy",
        "cookie_file",
        "cookie_browser",
        "theme",
        "raw_args",
    ] {
        if !result[key].is_string() || result[key].as_str().unwrap().contains('\0') {
            return Err(format!("{key} 的值不正确"));
        }
    }
    if result["directory"].as_str().unwrap().trim().is_empty() {
        return Err("请选择保存目录".into());
    }
    if !["best", "2160", "1440", "1080", "720", "480"].contains(&strval(&result, "quality")) {
        return Err("不支持的画质".into());
    }
    if ![
        "auto", "mp4", "mkv", "webm", "mp3", "m4a", "flac", "wav", "opus",
    ]
    .contains(&strval(&result, "format"))
    {
        return Err("不支持的输出格式".into());
    }
    if !["system", "direct", "custom"].contains(&strval(&result, "proxy_mode")) {
        return Err("代理模式不正确".into());
    }
    if strval(&result, "proxy_mode") == "custom" && strval(&result, "proxy").is_empty() {
        return Err("请输入代理地址".into());
    }
    if !["system", "light", "dark"].contains(&strval(&result, "theme")) {
        return Err("主题不正确".into());
    }
    for k in ["auto_fallback", "subtitles", "thumbnail", "close_to_tray"] {
        if !result[k].is_boolean() {
            return Err(format!("{k} 必须为开关"));
        }
    }
    advanced_args(&result, "command")?;
    Ok(result)
}
pub fn strval<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// Unlike shell parsing, backslashes in Windows paths are never interpreted.
pub fn split_args(s: &str) -> Result<Vec<String>, String> {
    if s.len() > 65536 || s.contains('\0') {
        return Err("参数文本过长或含空字符".into());
    }
    if s.trim_start().starts_with('[') {
        let values = serde_json::from_str::<Vec<String>>(s)
            .map_err(|e| format!("参数 JSON 数组无效：{e}"))?;
        if values.iter().any(|v| v.contains('\0')) {
            return Err("参数不能含空字符".into());
        }
        return Ok(values);
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut started = false;
    for c in s.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                current.push(c);
            }
            started = true;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            started = true;
        } else if c.is_whitespace() {
            if started {
                out.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(c);
            started = true;
        }
    }
    if quote.is_some() {
        return Err("参数的引号没有闭合；复杂参数建议使用 JSON 字符串数组".into());
    }
    if started {
        out.push(current);
    }
    Ok(out)
}

fn definitions() -> &'static Vec<Value> {
    static DEFINITIONS: OnceLock<Vec<Value>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        catalog()["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["options"].as_array().unwrap().clone())
            .collect()
    })
}

fn for_preview(def: &Value) -> bool {
    def["preview"] == true
        || [
            "--format",
            "--format-sort",
            "--format-sort-force",
            "--no-format-sort-force",
            "--prefer-free-formats",
            "--no-prefer-free-formats",
            "--video-multistreams",
            "--no-video-multistreams",
            "--audio-multistreams",
            "--no-audio-multistreams",
            "--check-formats",
            "--check-all-formats",
            "--no-check-formats",
            "--extract-audio",
            "--audio-format",
            "--audio-quality",
            "--preset-alias",
        ]
        .contains(&strval(def, "name"))
}

pub fn advanced_args(settings: &Value, context: &str) -> Result<Vec<String>, String> {
    let defs = definitions();
    let mut output = Vec::new();
    let map = settings["advanced"]
        .as_object()
        .ok_or("高级设置必须为对象")?;
    for (name, value) in map {
        if value == &Value::Bool(false) || value.is_null() {
            continue;
        }
        let def = defs
            .iter()
            .find(|x| x["name"] == *name)
            .ok_or_else(|| format!("未知高级选项 {name}，可在命令工具中运行"))?;
        let nargs = def["nargs"].as_u64().unwrap_or(0) as usize;
        if context != "command" && def["command_only"].as_bool() == Some(true) {
            return Err(format!("{name} 会改变队列执行方式，请在工具中运行"));
        }
        if context == "preview" && !for_preview(def) {
            continue;
        }
        if nargs == 0 {
            if !value.is_boolean() {
                return Err(format!("{name} 必须为开关"));
            }
            output.push(name.clone());
            continue;
        }
        let rows: Vec<Vec<String>> = if let Some(s) = value.as_str() {
            vec![vec![s.to_owned()]]
        } else if let Some(a) = value.as_array() {
            if !a.iter().all(|v| {
                v.is_string()
                    || v.as_array()
                        .map(|r| r.iter().all(Value::is_string))
                        .unwrap_or(false)
            }) {
                return Err(format!("{name} 的参数必须为文本"));
            }
            if a.iter().all(Value::is_array) {
                a.iter()
                    .map(|r| {
                        r.as_array()
                            .unwrap()
                            .iter()
                            .map(|x| x.as_str().unwrap_or("").to_owned())
                            .collect()
                    })
                    .collect()
            } else if nargs == 1 {
                a.iter()
                    .map(|x| vec![x.as_str().unwrap_or("").to_owned()])
                    .collect()
            } else {
                vec![a
                    .iter()
                    .map(|x| x.as_str().unwrap_or("").to_owned())
                    .collect()]
            }
        } else {
            return Err(format!("{name} 需要文本或参数数组"));
        };
        for row in rows {
            if row.len() != nargs {
                return Err(format!("{name} 每次需要 {nargs} 个值"));
            }
            for val in &row {
                if context != "command" && name == "--output" && (val == "-" || val.ends_with(":-"))
                {
                    return Err("标准输出媒体流请在命令工具中运行".into());
                }
                if val.len() > 16384 || val.contains('\0') {
                    return Err(format!("{name} 的参数值无效"));
                }
                if def["input_type"] == "number"
                    && !(name == "--file-access-retries" && val == "infinite")
                {
                    let n = val.parse::<f64>().map_err(|_| format!("{name} 需要数字"))?;
                    let positive = [
                        "--socket-timeout",
                        "--concurrent-fragments",
                        "--trim-filenames",
                    ]
                    .contains(&name.as_str());
                    if !n.is_finite() || n < 0. || (positive && n == 0.) {
                        return Err(format!("{name} 数值范围不正确"));
                    }
                    if [
                        "--concurrent-fragments",
                        "--trim-filenames",
                        "--file-access-retries",
                    ]
                    .contains(&name.as_str())
                        && n.fract() != 0.
                    {
                        return Err(format!("{name} 需要整数"));
                    }
                }
                if let Some(choices) = def["choices"].as_array() {
                    if !choices.is_empty() && !choices.iter().any(|x| x.as_str() == Some(val)) {
                        return Err(format!("{name} 的值不在可选范围内"));
                    }
                }
            }
            output.push(name.clone());
            output.extend(row);
        }
    }
    let raw = split_args(strval(settings, "raw_args"))?;
    if context == "command" {
        output.extend(raw);
        return Ok(output);
    }
    let mut i = 0;
    while i < raw.len() {
        let token = &raw[i];
        let (name, attached) = token
            .split_once('=')
            .map(|(a, b)| (a, Some(b)))
            .unwrap_or((token, None));
        let def = defs
            .iter()
            .find(|d| {
                d["aliases"]
                    .as_array()
                    .map(|a| a.iter().any(|x| x.as_str() == Some(name)))
                    .unwrap_or(false)
            })
            .ok_or_else(|| format!("无法在托管下载中识别 {name}，请使用完整参数名或命令工具"))?;
        let canonical = def["name"].as_str().unwrap();
        if map.contains_key(canonical) && map[canonical] != false {
            return Err(format!(
                "{canonical} 同时在图形设置和原始参数中填写，请保留一种"
            ));
        }
        if def["command_only"].as_bool() == Some(true) {
            return Err(format!("{canonical} 请在命令工具中运行"));
        }
        let count = def["nargs"].as_u64().unwrap_or(0) as usize;
        if attached.is_some() && count == 0 {
            return Err(format!("{name} 不接受值"));
        }
        let required = count.saturating_sub(usize::from(attached.is_some()));
        if i + required >= raw.len() {
            return Err(format!("{name} 缺少参数"));
        }
        if canonical == "--output"
            && (attached == Some("-")
                || (attached.is_none()
                    && raw
                        .get(i + 1)
                        .map(|s| s == "-" || s.ends_with(":-"))
                        .unwrap_or(false)))
        {
            return Err("标准输出媒体流请在命令工具中运行".into());
        }
        if context != "preview" || for_preview(def) {
            output.extend(raw[i..=i + required].iter().cloned());
        }
        i += required + 1;
    }
    let options = parse_option_tokens(&output)?;
    let enabled: std::collections::HashSet<_> =
        options.iter().map(|(name, _)| name.as_str()).collect();
    for name in &enabled {
        if let Some(positive) = name.strip_prefix("--no-") {
            if enabled.contains(format!("--{positive}").as_str()) {
                return Err(format!("{name} 与 --{positive} 不能同时启用"));
            }
        }
    }
    for pair in [
        ["--force-ipv4", "--force-ipv6"],
        ["--abort-on-error", "--ignore-errors"],
    ] {
        if enabled.contains(pair[0]) && enabled.contains(pair[1]) {
            return Err(format!("{} 与 {} 不能同时启用", pair[0], pair[1]));
        }
    }
    Ok(output)
}

/// Validated advanced options with aliases normalized and values kept separate.
fn parsed_options(settings: &Value, context: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let args = advanced_args(settings, context)?;
    parse_option_tokens(&args)
}

fn parse_option_tokens(args: &[String]) -> Result<Vec<(String, Vec<String>)>, String> {
    let defs = definitions();
    let mut output = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let (name, attached) = args[index]
            .split_once('=')
            .map(|(name, value)| (name, Some(value)))
            .unwrap_or((&args[index], None));
        let definition = defs
            .iter()
            .find(|definition| {
                definition["name"] == name
                    || definition["aliases"]
                        .as_array()
                        .is_some_and(|aliases| aliases.iter().any(|alias| alias == name))
            })
            .ok_or("无法识别高级参数")?;
        let count = definition["nargs"].as_u64().unwrap_or(0) as usize;
        let following = count.saturating_sub(usize::from(attached.is_some()));
        if index + following >= args.len() {
            return Err("高级参数缺少值".into());
        }
        let mut values = Vec::new();
        if let Some(value) = attached {
            values.push(value.to_owned());
        }
        values.extend(args[index + 1..index + 1 + following].iter().cloned());
        output.push((strval(definition, "name").to_owned(), values));
        index += 1 + following;
    }
    Ok(output)
}

/// Names only, so a GUI can explain overrides without exposing sensitive values.
pub fn effective_override_names(settings: &Value) -> Result<Vec<String>, String> {
    let names: std::collections::BTreeSet<_> = parsed_options(settings, "download")?
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    Ok(names.into_iter().collect())
}

/// Share the engine's precedence; never inspect OS settings or proxy programs.
pub fn effective_proxy(settings: &Value) -> Result<(String, String), String> {
    let mut policy = match strval(settings, "proxy_mode") {
        "" | "system" => ("system".to_owned(), String::new()),
        "direct" => ("direct".to_owned(), String::new()),
        "custom" if !strval(settings, "proxy").is_empty() => {
            ("custom".to_owned(), strval(settings, "proxy").to_owned())
        }
        _ => return Err("代理设置无效".into()),
    };
    for (name, values) in parsed_options(settings, "preview")? {
        if name == "--proxy" {
            let value = values.first().ok_or("代理参数缺少地址")?;
            policy = (
                if value.is_empty() { "direct" } else { "custom" }.into(),
                value.clone(),
            );
        }
    }
    Ok(policy)
}

pub fn selection(settings: &Value) -> Result<Vec<String>, String> {
    let mut args = selection_defaults(settings)?;
    args.extend(advanced_args(settings, "preview")?);
    Ok(args)
}

pub fn selection_defaults(settings: &Value) -> Result<Vec<String>, String> {
    let extra = parsed_options(settings, "preview")?;
    let has = |name: &str| extra.iter().any(|(option, _)| option == name);
    let audio = ["mp3", "m4a", "flac", "wav", "opus"].contains(&strval(settings, "format"))
        || has("--extract-audio");
    let mut args = Vec::new();
    if !has("--format") && !has("--preset-alias") {
        let q = strval(settings, "quality");
        let filter = if q == "best" {
            String::new()
        } else if settings["auto_fallback"] == true {
            format!("[height<=?{q}]")
        } else {
            format!("[height={q}]")
        };
        args.extend([
            "--format".into(),
            if audio {
                "bestaudio/best".into()
            } else {
                format!("bv*{filter}+ba/b{filter}")
            },
        ]);
    }
    if !has("--format-sort") {
        args.extend(["--format-sort".into(), "res,fps".into()]);
    }
    Ok(args)
}

pub fn redact(text: &str, settings: &Value) -> String {
    let mut clean = text.to_owned();
    for k in ["proxy", "cookie_file"] {
        let s = strval(settings, k);
        if !s.is_empty() {
            clean = clean.replace(s, "[已隐藏]");
        }
    }
    let defs = definitions();
    fn mask(value: &Value, text: &mut String) {
        match value {
            Value::String(s) if !s.is_empty() => *text = text.replace(s, "[已隐藏]"),
            Value::Array(a) => {
                for v in a {
                    mask(v, text)
                }
            }
            _ => {}
        }
    }
    for d in defs {
        let name = d["name"].as_str().unwrap();
        if d["sensitive"] == true
            || [
                "--add-headers",
                "--add-header",
                "--netrc-cmd",
                "--username",
                "--ap-username",
                "--cookies",
                "--cookies-from-browser",
                "--proxy",
            ]
            .contains(&name)
        {
            mask(&settings["advanced"][name], &mut clean);
            if name == "--add-headers" {
                if let Some(values) = settings["advanced"][name].as_array() {
                    for v in values {
                        if let Some((_, value)) = v.as_str().and_then(|s| s.split_once(':')) {
                            if !value.trim().is_empty() {
                                clean = clean.replace(value.trim(), "[已隐藏]");
                            }
                        }
                    }
                }
            }
        }
    }
    if let Ok(raw) = split_args(strval(settings, "raw_args")) {
        let mut i = 0;
        while i < raw.len() {
            let (name, attached) = raw[i]
                .split_once('=')
                .map(|(a, b)| (a, Some(b)))
                .unwrap_or((&raw[i], None));
            if let Some(d) = defs.iter().find(|d| {
                d["aliases"]
                    .as_array()
                    .map(|a| a.iter().any(|x| x.as_str() == Some(name)))
                    .unwrap_or(false)
            }) {
                let n = d["nargs"].as_u64().unwrap_or(0) as usize;
                if d["sensitive"] == true
                    || name.contains("header")
                    || name.contains("username")
                    || name.contains("cookie")
                    || name == "--proxy"
                {
                    if let Some(s) = attached {
                        if !s.is_empty() {
                            clean = clean.replace(s, "[已隐藏]");
                        }
                    } else {
                        for s in raw.iter().skip(i + 1).take(n) {
                            if !s.is_empty() {
                                clean = clean.replace(s, "[已隐藏]");
                            }
                        }
                    }
                }
                i += 1 + n.saturating_sub(usize::from(attached.is_some()));
            } else {
                i += 1;
            }
        }
    }
    // URLs occur inside quoted debug JSON as well as plain-text messages.
    let mut result = String::new();
    let mut rest = clean.as_str();
    while let Some(start) = rest
        .find("http://")
        .into_iter()
        .chain(rest.find("https://"))
        .min()
    {
        result.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| c.is_whitespace() || ['\'', '"', '<', '>', '(', ')'].contains(&c))
            .unwrap_or(tail.len());
        let raw = &tail[..end];
        if let Ok(mut u) = url::Url::parse(raw) {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.set_query(None);
            u.set_fragment(None);
            result.push_str(u.as_str());
        } else {
            result.push_str("[已隐藏的网址]");
        }
        rest = &tail[end..];
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_path_quotes() {
        assert_eq!(
            split_args("--output \"D:\\视频\\%(title)s.%(ext)s\"").unwrap(),
            vec!["--output", "D:\\视频\\%(title)s.%(ext)s"]
        );
    }
    #[test]
    fn proxy_resolution_shares_structured_and_raw_precedence() {
        let mut settings = defaults(Path::new("."));
        assert_eq!(
            effective_proxy(&settings).unwrap(),
            ("system".into(), "".into())
        );
        settings["advanced"] = json!({"--proxy":"socks5://127.0.0.1:1080"});
        assert_eq!(
            effective_proxy(&settings).unwrap(),
            ("custom".into(), "socks5://127.0.0.1:1080".into())
        );
        settings["raw_args"] = json!("--proxy=http://127.0.0.1:8080");
        assert!(
            effective_proxy(&settings).is_err(),
            "ambiguous overrides must be rejected"
        );
        settings["advanced"] = json!({});
        assert_eq!(
            effective_proxy(&settings).unwrap().1,
            "http://127.0.0.1:8080"
        );
        settings["raw_args"] = json!(r#"["--proxy", ""]"#);
        assert_eq!(
            effective_proxy(&settings).unwrap(),
            ("direct".into(), "".into())
        );
    }
    #[test]
    fn proxy_name_inside_another_option_value_is_not_a_proxy_override() {
        let mut settings = defaults(Path::new("."));
        settings["raw_args"] =
            json!(r#"["--proxy", "socks5://127.0.0.1:1080", "--user-agent", "--proxy"]"#);
        assert_eq!(
            effective_proxy(&settings).unwrap(),
            ("custom".into(), "socks5://127.0.0.1:1080".into())
        );
    }
    #[test]
    fn override_summary_normalizes_aliases_but_not_option_values() {
        let mut settings = defaults(Path::new("."));
        settings["quality"] = json!("1080");
        settings["raw_args"] = json!(r#"["--user-agent", "-f"]"#);
        assert_eq!(
            effective_override_names(&settings).unwrap(),
            vec!["--user-agent"]
        );
        assert!(selection_defaults(&settings)
            .unwrap()
            .contains(&"bv*[height=1080]+ba/b[height=1080]".into()));
        for raw in ["-f best", "--format=best"] {
            settings["raw_args"] = json!(raw);
            assert_eq!(
                effective_override_names(&settings).unwrap(),
                vec!["--format"]
            );
            assert!(!selection_defaults(&settings)
                .unwrap()
                .contains(&"--format".into()));
        }
    }
    #[test]
    fn option_like_values_do_not_create_false_mutual_exclusion() {
        let mut settings = defaults(Path::new("."));
        settings["raw_args"] = json!(r#"["--user-agent", "--write-subs", "--no-write-subs"]"#);
        assert_eq!(
            effective_override_names(&settings).unwrap(),
            vec!["--no-write-subs", "--user-agent"]
        );
        settings["raw_args"] = json!("--write-subs --no-write-subs");
        assert!(advanced_args(&settings, "download").is_err());
    }
    #[test]
    fn exact_quality() {
        let mut s = defaults(Path::new("."));
        s["quality"] = json!("1080");
        assert!(selection(&s)
            .unwrap()
            .contains(&"bv*[height=1080]+ba/b[height=1080]".into()));
        s["auto_fallback"] = json!(true);
        assert!(selection(&s)
            .unwrap()
            .iter()
            .any(|x| x.contains("height<=?1080")));
    }
    #[test]
    fn commands_cannot_break_queue() {
        let mut s = defaults(Path::new("."));
        s["raw_args"] = json!("--dump-json");
        assert!(advanced_args(&s, "download").is_err());
        assert!(advanced_args(&s, "command").is_ok());
    }
    #[test]
    fn raw_conflict() {
        let mut s = defaults(Path::new("."));
        s["advanced"] = json!({"--format":"best"});
        s["raw_args"] = json!("-f worst");
        assert!(advanced_args(&s, "download").is_err());
    }
    #[test]
    fn preview_and_download_share_advanced_selection() {
        let mut s = defaults(Path::new("."));
        s["advanced"] = json!({"--format":"137+140","--format-sort":"codec:h264"});
        let preview = selection(&s).unwrap();
        assert_eq!(
            preview,
            vec!["--format", "137+140", "--format-sort", "codec:h264"]
        );
        let mut download = selection_defaults(&s).unwrap();
        download.extend(advanced_args(&s, "download").unwrap());
        assert_eq!(preview, download);
    }
    #[test]
    fn malformed_options_and_binary_stdout_rejected() {
        let mut s = defaults(Path::new("."));
        s["advanced"] = json!({"--extract-audio":"false"});
        assert!(advanced_args(&s, "download").is_err());
        s["advanced"] = json!({"--output":"-"});
        assert!(advanced_args(&s, "download").is_err());
        assert!(advanced_args(&s, "command").is_ok());
        assert!(split_args(r#"["a\u0000b"]"#).is_err());
    }
    #[test]
    fn diagnostic_redaction_handles_json_urls_and_cli_passwords() {
        let mut s = defaults(Path::new("."));
        s["raw_args"] = json!("--password verysecret");
        s["proxy"] = json!("http://proxyuser:proxysecret@localhost:8000");
        let clean = redact(
            r#"{"url":"https://alice:pass@example.test/video?token=signed#frag"} verysecret http://proxyuser:proxysecret@localhost:8000"#,
            &s,
        );
        for secret in ["alice", "pass@", "signed", "verysecret", "proxysecret"] {
            assert!(!clean.contains(secret), "{clean}");
        }
        assert!(clean.contains("https://example.test/video"));
    }
}
