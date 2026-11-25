// SPDX-License-Identifier: Apache-2.0
use flow_core::Core;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

async fn terminal(core: &Arc<Core>, id: &str) -> Value {
    for _ in 0..240 {
        let ts = core.dispatch("list_tasks", json!({})).await.unwrap();
        if let Some(t) = ts.as_array().unwrap().iter().find(|t| t["id"] == id) {
            if matches!(
                t["status"].as_str(),
                Some("completed" | "failed" | "cancelled" | "interrupted" | "skipped")
            ) {
                return t.clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("task timed out: {id}")
}

async fn download(core: &Arc<Core>, url: &str, settings: &Value, key: &str) -> Value {
    let preview = core.dispatch("preview", json!({"url":url,"mode":"video","settings":settings,"request_id":format!("preview-{key}")})).await.unwrap();
    assert!(
        !preview["entries"].as_array().unwrap().is_empty(),
        "no preview entry"
    );
    let payload = json!({"entries":preview["entries"],"settings":settings,"request_id":key});
    let tasks = core.dispatch("enqueue", payload.clone()).await.unwrap();
    let duplicate = core.dispatch("enqueue", payload).await.unwrap();
    assert_eq!(
        tasks[0]["id"], duplicate[0]["id"],
        "submission is not idempotent"
    );
    terminal(core, tasks[0]["id"].as_str().unwrap()).await
}

fn probe(engine: &std::path::Path, task: &Value) -> Value {
    assert_eq!(task["status"], "completed", "download failed: {task}");
    let output = task["output"].as_str().expect("output path missing");
    assert!(PathBuf::from(output).is_file(), "output missing {output}");
    let result = std::process::Command::new(engine.join("ffprobe.exe"))
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
            output,
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    serde_json::from_slice(&result.stdout).unwrap()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let base = &args[2];
    let engine = args
        .get(3)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("engine"));
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let work = args
        .get(4)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(format!("test-artifacts/integration/run-{run}")));
    assert!(
        work.is_absolute() && work.starts_with(root.join("test-artifacts")),
        "Test output must stay under the project test-artifacts directory"
    );
    let data = work.join("data");
    let events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = events.clone();
    let core = Core::new(
        data.clone(),
        engine.clone(),
        Arc::new(move |e| {
            captured.lock().unwrap().push(e);
        }),
    )
    .unwrap();
    let boot = core.dispatch("bootstrap", json!({})).await.unwrap();
    let count: usize = boot["catalog"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["options"].as_array().unwrap().len())
        .sum();
    assert_eq!(count, 323, "catalog missing options");
    let mut settings = boot["settings"].clone();
    settings["directory"] = json!(work.join("下载 output"));
    settings["proxy_mode"] = json!("direct");
    settings["quality"] = json!("best");
    settings["format"] = json!("auto");
    core.dispatch("save_settings", json!({"settings":settings}))
        .await
        .unwrap();

    let task = download(&core, &format!("{base}/sample.mp4"), &settings, "single").await;
    let streams = probe(&engine, &task);
    assert!(streams["streams"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["width"] == 320 && s["height"] == 180));
    println!("PASS: independent state, 323 options, Unicode/space paths, real download, idempotent submission, dimensions");

    let merged = download(&core, &format!("{base}/stream.mpd"), &settings, "merged").await;
    let streams = probe(&engine, &merged);
    assert!(streams["streams"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["codec_type"] == "video"));
    assert!(streams["streams"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["codec_type"] == "audio"));
    println!("PASS: DASH separate audio/video selection and FFmpeg merge");

    let mut exact = settings.clone();
    exact["quality"] = json!("1080");
    exact["auto_fallback"] = json!(false);
    let result = core.dispatch("preview", json!({"url":format!("{base}/stream.mpd"),"mode":"video","settings":exact,"request_id":"unavailable"})).await;
    assert!(
        result.is_err(),
        "unavailable exact resolution silently downgraded"
    );
    exact["auto_fallback"] = json!(true);
    let result = core.dispatch("preview", json!({"url":format!("{base}/stream.mpd"),"mode":"video","settings":exact,"request_id":"fallback"})).await;
    assert!(
        result.is_ok(),
        "explicit resolution fallback failed: {result:?}"
    );
    println!("PASS: strict quality failure and explicit fallback");

    let mut audio = settings.clone();
    audio["format"] = json!("m4a");
    audio["directory"] = json!(work.join("audio"));
    let audio = download(&core, &format!("{base}/stream.mpd"), &audio, "audio").await;
    let streams = probe(&engine, &audio);
    assert!(streams["streams"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["codec_type"] != "video"));
    assert!(streams["streams"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["codec_type"] == "audio"));
    println!("PASS: audio-only extraction");

    let mut subtitles = settings.clone();
    subtitles["subtitles"] = json!(true);
    subtitles["directory"] = json!(work.join("字幕 sidecars"));
    let subtitle_task = download(
        &core,
        &format!("{base}/subtitles.html"),
        &subtitles,
        "subtitle-sidecars",
    )
    .await;
    probe(&engine, &subtitle_task);
    let subtitle_output = PathBuf::from(subtitle_task["output"].as_str().unwrap());
    for (language, marker) in [("en", "FLOW_SUBTITLE_ENGLISH"), ("zh-CN", "流光字幕校验")] {
        let path = subtitle_output.with_extension(format!("{language}.vtt"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("subtitle sidecar missing {}: {error}", path.display()));
        assert!(
            text.starts_with("WEBVTT"),
            "invalid VTT: {}",
            path.display()
        );
        assert!(
            text.contains(marker),
            "subtitle contents differ: {}",
            path.display()
        );
        assert!(text.contains("00:00:00.000 --> 00:00:01.500"));
    }
    assert!(
        !subtitle_output.with_extension("fr.vtt").exists(),
        "default Chinese/English subtitle selection downloaded French"
    );
    println!("PASS: HTML5 VTT Chinese/English sidecars, cue contents and language selection");

    let mut embedded = subtitles.clone();
    embedded["format"] = json!("mp4");
    embedded["directory"] = json!(work.join("embedded subtitles"));
    embedded["advanced"] = json!({"--embed-subs":true});
    let embedded_task = download(
        &core,
        &format!("{base}/subtitles.html"),
        &embedded,
        "subtitle-embedded",
    )
    .await;
    let embedded_info = probe(&engine, &embedded_task);
    let subtitle_streams: Vec<_> = embedded_info["streams"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|stream| stream["codec_type"] == "subtitle")
        .collect();
    assert_eq!(
        subtitle_streams.len(),
        2,
        "embedded subtitle stream count differs: {embedded_info}"
    );
    let mut embedded_text = String::new();
    for stream in subtitle_streams {
        assert_eq!(
            stream["codec_name"], "mov_text",
            "MP4 subtitle codec differs"
        );
        let result = std::process::Command::new(engine.join("ffmpeg.exe"))
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                embedded_task["output"].as_str().unwrap(),
                "-map",
                &format!("0:{}", stream["index"].as_u64().unwrap()),
                "-f",
                "srt",
                "-",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "subtitle extraction failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        embedded_text.push_str(&String::from_utf8(result.stdout).unwrap());
    }
    assert!(embedded_text.contains("FLOW_SUBTITLE_ENGLISH"));
    assert!(embedded_text.contains("流光字幕校验"));
    assert!(!embedded_text.contains("FLOW_FRENCH_NOT_SELECTED"));
    println!(
        "PASS: FFmpeg MP4 subtitle embedding, two mov_text streams and extracted cue contents"
    );

    let output = task["output"].as_str().unwrap();
    let original = std::fs::read(output).unwrap();
    let repeat = download(
        &core,
        &format!("{base}/sample.mp4"),
        &settings,
        "keep-existing",
    )
    .await;
    assert!(
        matches!(repeat["status"].as_str(), Some("completed" | "skipped")),
        "existing file rejected: {repeat}"
    );
    assert_eq!(
        original,
        std::fs::read(output).unwrap(),
        "existing file overwritten"
    );
    let profiles = core
        .dispatch(
            "profiles_save",
            json!({"name":"integration-profile","settings":settings}),
        )
        .await
        .unwrap();
    assert!(profiles
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "integration-profile"));

    let cmd = core.dispatch("run_command", json!({"args":format!("--dump-single-json --skip-download {base}/sample.mp4"),"settings":settings})).await.unwrap();
    let cmd = terminal(&core, cmd["id"].as_str().unwrap()).await;
    assert_eq!(cmd["status"], "completed", "command failed: {cmd}");
    let logs = core
        .dispatch("logs", json!({"id":cmd["id"]}))
        .await
        .unwrap();
    let info: Value =
        serde_json::from_str(logs["output"].as_str().unwrap()).expect("stdout JSON corrupted");
    assert!(info["url"].as_str().unwrap().ends_with("sample.mp4"));
    println!("PASS: preserve existing files, presets, complete unmodified native JSON stdout");

    let playlist = core.dispatch("preview", json!({"url":format!("{base}/playlist.html"),"mode":"playlist","settings":settings,"request_id":"playlist"})).await.unwrap();
    assert!(
        playlist["is_playlist"] == true,
        "collection not recognized: {playlist}"
    );
    assert_eq!(playlist["entries"].as_array().unwrap().len(), 2);
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "preview_progress" && e["data"]["request_id"] == "playlist"));
    println!("PASS: real collection parsing and incremental progress events");

    core.shutdown();
    drop(core);
    let reopened = Core::new(data, engine, Arc::new(|_| {})).unwrap();
    let recovered = reopened.dispatch("bootstrap", json!({})).await.unwrap();
    // Persisted directories are normalized; Windows accepts both slash styles.
    // Check the actual destination rather than its display spelling.
    assert_eq!(
        PathBuf::from(recovered["settings"]["directory"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        PathBuf::from(settings["directory"].as_str().unwrap())
            .canonicalize()
            .unwrap()
    );
    assert!(recovered["tasks"].as_array().unwrap().len() >= 5);
    reopened.shutdown();
    println!("PASS: saved settings, task history and profiles survive restart");
}
