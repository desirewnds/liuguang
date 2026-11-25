// SPDX-License-Identifier: Apache-2.0
use crate::config::strval;
use serde_json::{json, Value};

pub fn size(info: &Value) -> (Option<u64>, bool) {
    if info["is_live"] == true {
        return (None, false);
    }
    if let Some(streams) = info["requested_formats"]
        .as_array()
        .or_else(|| info["requested_downloads"].as_array())
    {
        if !streams.is_empty() {
            let mut total = 0u64;
            let mut approximate = false;
            for s in streams {
                let mut stream = s.clone();
                if stream["duration"].is_null() {
                    stream["duration"] = info["duration"].clone();
                }
                let (n, a) = size(&stream);
                match n {
                    Some(n) => total = total.saturating_add(n),
                    None => return (None, false),
                }
                approximate |= a;
            }
            return (Some(total), approximate);
        }
    }
    if let Some(n) = info["filesize"].as_u64() {
        return (Some(n), false);
    }
    if let Some(n) = info["filesize_approx"].as_f64() {
        return (Some(n as u64), true);
    }
    if let (Some(rate), Some(duration)) = (info["tbr"].as_f64(), info["duration"].as_f64()) {
        if rate > 0. && duration > 0. {
            return (Some((rate * 1000. * duration / 8.) as u64), true);
        }
    }
    (None, false)
}
pub fn format_label(info: &Value) -> String {
    let video = info["requested_formats"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["height"].is_number()))
        .unwrap_or(info);
    let mut parts = Vec::new();
    if let Some(h) = video["height"].as_u64() {
        parts.push(format!("{h}p"));
    }
    if let Some(f) = video["fps"].as_f64() {
        parts.push(format!("{f} fps"));
    }
    let codec = strval(video, "vcodec");
    if !codec.is_empty() && codec != "none" {
        parts.push(codec.split('.').next().unwrap_or(codec).into());
    }
    if parts.is_empty() {
        let audio = strval(video, "acodec");
        if !audio.is_empty() && audio != "none" {
            parts.push(audio.split('.').next().unwrap_or(audio).into());
        }
    }
    if parts.is_empty() {
        "源文件".into()
    } else {
        parts.join(" · ")
    }
}
pub fn formats(info: &Value) -> Vec<Value> {
    info["formats"].as_array().map(|a|a.iter().filter(|f|f["height"].is_number() && strval(f,"protocol")!="mhtml" && !["images","none"].contains(&strval(f,"vcodec"))).map(|f|{
        let mut f=f.clone();if f["duration"].is_null(){f["duration"]=info["duration"].clone();}let(n,approx)=size(&f);
        // A format row is a stream, not necessarily a complete video+audio download.
        json!({"id":f["format_id"],"resolution":f["resolution"],"height":f["height"],"fps":f["fps"],"vcodec":f["vcodec"],"acodec":f["acodec"],"size":n,"approximate":approx,"size_scope":if f["acodec"]=="none"{"video_stream"}else{"combined"}})
    }).collect()).unwrap_or_default()
}
pub fn preview(info: &Value, original: &str) -> Value {
    let is_playlist = info["entries"].is_array();
    let entries: Vec<Value> = if let Some(items) = info["entries"].as_array() {
        items
            .iter()
            .enumerate()
            .map(|(i, e)| entry(e, original, Some(i + 1)))
            .collect()
    } else {
        vec![entry(info, original, None)]
    };
    let (n, approx) = if is_playlist {
        (None, false)
    } else {
        size(info)
    };
    json!({"id":info["id"],"url":original,"title":info["title"].as_str().unwrap_or("未命名视频"),"thumbnail":info["thumbnail"],"duration":info["duration"],"is_playlist":is_playlist,"total":info["playlist_count"].as_u64().unwrap_or(entries.len() as u64),"entries":entries,"formats":formats(info),"size":n,"approximate":approx,"format_label":format_label(info),"warnings":[]})
}
pub(crate) fn entry(e: &Value, original: &str, index: Option<usize>) -> Value {
    let raw = e["webpage_url"]
        .as_str()
        .or_else(|| e["url"].as_str())
        .unwrap_or(original);
    let url = if !raw.starts_with("http") && e["ie_key"].as_str() == Some("Youtube") {
        format!("https://www.youtube.com/watch?v={raw}")
    } else {
        raw.to_owned()
    };
    let (n, approximate) = size(e);
    let available = !e.is_null()
        && !["private", "premium_only", "subscriber_only", "needs_auth"]
            .contains(&strval(e, "availability"));
    json!({"id":e["id"].as_str().unwrap_or(raw),"url":url,"title":e["title"].as_str().unwrap_or("不可用条目"),"index":e["playlist_index"].as_u64().or(index.map(|i|i as u64)),"duration":e["duration"],"thumbnail":e["thumbnail"].as_str().or_else(||e["thumbnails"].as_array().and_then(|a|a.last()).and_then(|x|x["url"].as_str())),"size":n,"approximate":approximate,"available":available})
}

pub fn metadata_url(original: &str, mode: &str) -> String {
    if mode == "video" {
        return original.into();
    }
    if let Ok(mut u) = url::Url::parse(original) {
        if u.host_str()
            .map(|s| s.ends_with("bilibili.com"))
            .unwrap_or(false)
            && u.path().contains("/video/")
        {
            let q: Vec<(String, String)> = u
                .query_pairs()
                .filter(|(k, _)| k != "p")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            u.set_query(None);
            if !q.is_empty() {
                u.query_pairs_mut().extend_pairs(q);
            }
            return u.into();
        }
    }
    original.into()
}
pub fn valid_url(s: &str) -> Result<(), String> {
    let u = url::Url::parse(s).map_err(|_| "请输入有效视频网址")?;
    if !["https", "http"].contains(&u.scheme()) || u.host_str().is_none() {
        return Err("仅支持 HTTP / HTTPS 视频网址".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_stream_not_partial() {
        assert_eq!(
            size(&json!({"requested_formats":[{"filesize":100},{}]})).0,
            None
        );
    }
    #[test]
    fn sum_selected_streams() {
        assert_eq!(
            size(&json!({"requested_formats":[{"filesize":100},{"filesize_approx":50}]})),
            (Some(150), true)
        );
    }
    #[test]
    fn bili_current_preserved() {
        let u = "https://www.bilibili.com/video/BV1JV411t7ow?t=5&p=7";
        assert!(metadata_url(u, "auto").contains("t=5"));
        assert!(!metadata_url(u, "auto").contains("p=7"));
        assert_eq!(metadata_url(u, "video"), u);
    }
    #[test]
    fn single_preview_retains_exact_estimated_and_unknown_sizes() {
        for (info, expected, approximate) in [
            (json!({"filesize":100}), json!(100), false),
            (json!({"filesize_approx":100}), json!(100), true),
            (json!({"tbr":8,"duration":1}), json!(1000), true),
            (json!({}), Value::Null, false),
            (json!({"is_live":true,"filesize":100}), Value::Null, false),
        ] {
            let out = preview(&info, "https://example.test/video");
            assert_eq!(out["size"], expected);
            assert_eq!(out["approximate"], approximate);
            assert_eq!(out["entries"][0]["size"], expected);
            assert_eq!(out["entries"][0]["approximate"], approximate);
        }
    }
    #[test]
    fn playlist_and_incremental_entries_retain_size_accuracy() {
        let entries = json!([
            {"id":"exact","playlist_index":7,"requested_formats":[{"filesize":100},{"filesize":50}]},
            {"id":"estimate","playlist_index":8,"requested_formats":[{"filesize":100},{"filesize_approx":50}]},
            {"id":"unknown","playlist_index":9,"requested_formats":[{"filesize":100},{}]}
        ]);
        let out = preview(&json!({"entries":entries}), "https://example.test/list");
        assert!(out["size"].is_null());
        for (i, (size, approximate)) in [
            (json!(150), false),
            (json!(150), true),
            (Value::Null, false),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(out["entries"][i]["size"], size);
            assert_eq!(out["entries"][i]["approximate"], approximate);
            let incremental = entry(&entries[i], "https://example.test/list", Some(i + 1));
            assert_eq!(incremental["size"], size);
            assert_eq!(incremental["approximate"], approximate);
            assert_eq!(incremental["index"], 7 + i);
        }
    }
    #[test]
    fn video_formats_exclude_storyboards_and_audio_but_keep_unknown_video_codecs() {
        let info = json!({"formats":[
            {"format_id":"sb3","protocol":"mhtml","height":27,"vcodec":"images","acodec":"none"},
            {"format_id":"sb0","protocol":"mhtml","height":180},
            {"format_id":"image-grid","protocol":"https","height":90,"vcodec":"images"},
            {"format_id":"audio-with-dimensions","protocol":"https","height":180,"vcodec":"none","acodec":"opus"},
            {"format_id":"251","protocol":"https","vcodec":"none","acodec":"opus"},
            {"format_id":"137","protocol":"https","height":1080,"vcodec":"avc1.640028","acodec":"none"},
            {"format_id":"18","protocol":"https","height":360,"vcodec":"avc1.42001E","acodec":"mp4a.40.2"},
            {"format_id":"unknown-codec","protocol":"https","height":720},
            {"format_id":"new-codec","protocol":"https","height":2160,"vcodec":"futurecodec.1"}
        ]});
        let ids: Vec<_> = formats(&info)
            .into_iter()
            .map(|f| f["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, ["137", "18", "unknown-codec", "new-codec"]);
    }
}
