// SPDX-License-Identifier: Apache-2.0
//! Explicit, isolated real-engine acceptance. Invoked by run-core-smoke.py.
use flow_core::Core;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetExitCodeProcess(handle: isize, code: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }
    unsafe {
        let handle = OpenProcess(0x1000, 0, pid);
        if handle == 0 {
            return false;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == 259
    }
}
#[cfg(not(windows))]
fn process_alive(pid: u32) -> bool {
    PathBuf::from(format!("/proc/{pid}")).exists()
}
async fn task(core: &Arc<Core>, id: &str) -> Value {
    core.dispatch("list_tasks", json!({}))
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap()
        .clone()
}
async fn wait(core: &Arc<Core>, id: &str, wanted: &[&str], seconds: u64) -> Value {
    let start = Instant::now();
    loop {
        let t = task(core, id).await;
        if wanted.contains(&t["status"].as_str().unwrap_or("")) {
            return t;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timeout: {t}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn own_pid(core: &Arc<Core>, id: &str) -> u32 {
    for _ in 0..100 {
        let d = core.dispatch("diagnostics", json!({})).await.unwrap();
        if let Some(p) = d["processes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["task_id"] == id)
        {
            return p["pid"].as_u64().unwrap() as u32;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("process not registered: {id}")
}
async fn stopped(pids: &[u32]) {
    let start = Instant::now();
    while pids.iter().any(|p| process_alive(*p)) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "owned processes survived: {pids:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn retry(core: &Arc<Core>, id: &str) {
    for _ in 0..100 {
        match core
            .dispatch("task_action", json!({"ids":[id],"action":"retry"}))
            .await
        {
            Ok(_) => return,
            Err(e) if e.contains("尚未停止") => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(e) => panic!("retry failed: {e}"),
        }
    }
    panic!("retry blocked")
}
fn sha(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let mode = &args[2];
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    if mode == "update" {
        let data = root.join(format!("test-artifacts/engine-update-test/run-{run}"));
        let original = root.join("engine/yt-dlp.exe");
        let before = sha(&original);
        let core = Core::new(data.clone(), root.join("engine"), Arc::new(|_| {})).unwrap();
        core.dispatch("save_settings", json!({"settings":{"proxy_mode":"direct"}}))
            .await
            .unwrap();
        let old = core.dispatch("engine_info", json!({})).await.unwrap();
        println!("BEFORE: {old}");
        let update = core.dispatch("engine_update", json!({})).await;
        assert_eq!(before, sha(&original), "packaged engine changed");
        match update {
            Ok(result) => {
                println!("UPDATE: {result}");
                assert!(
                    PathBuf::from(result["engine"]["path"].as_str().unwrap()).starts_with(&data)
                );
                assert!(result["engine"]["can_rollback"] == true);
                let rollback = core.dispatch("engine_rollback", json!({})).await.unwrap();
                println!("ROLLBACK: {rollback}");
                assert_eq!(rollback["engine"]["version"], old["version"]);
                assert_eq!(
                    sha(Path::new(rollback["engine"]["path"].as_str().unwrap())),
                    before
                );
                assert_eq!(sha(&original), before);
                std::fs::write(data.join("report.json"),json!({"status":"passed","before":old,"update":result,"rollback":rollback,"packaged_sha256":before}).to_string()).unwrap();
                println!("PASS: official update, isolated writable copy, protocol validation, rollback, packaged SHA256 unchanged");
            }
            Err(e) => {
                println!("UPDATE BLOCKED: {e}");
                std::fs::write(data.join("report.json"),json!({"status":"network_or_engine_failure","error":e,"before":old,"packaged_sha256_unchanged":true}).to_string()).unwrap();
                core.shutdown();
                std::process::exit(2);
            }
        }
        core.shutdown();
        return;
    }
    let base = &args[3];
    let python = &args[4];
    let child = &args[5];
    let work = root.join(format!("test-artifacts/core-smoke/run-{run}"));
    let data = work.join("data");
    let core = Core::new(data.clone(), root.join("engine"), Arc::new(|_| {})).unwrap();
    let boot = core.dispatch("bootstrap", json!({})).await.unwrap();
    let mut settings = boot["settings"].clone();
    settings["proxy_mode"] = json!("direct");
    settings["directory"] = json!(work.join("downloads"));
    settings["advanced"] = json!({"--limit-rate":"8K"});
    core.dispatch("save_settings", json!({"settings":settings}))
        .await
        .unwrap();
    let entries =
        json!([{"id":"sample","url":format!("{base}/sample.mp4"),"title":"cancel sample"}]);
    let first = core
        .dispatch("enqueue", json!({"entries":entries,"settings":settings}))
        .await
        .unwrap();
    let id = first[0]["id"].as_str().unwrap();
    let pid = own_pid(&core, id).await;
    wait(&core, id, &["downloading"], 20).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    core.dispatch("task_action", json!({"ids":[id],"action":"cancel"}))
        .await
        .unwrap();
    stopped(&[pid]).await;
    assert_eq!(task(&core, id).await["status"], "cancelled");
    retry(&core, id).await;
    let completed = wait(&core, id, &["completed", "failed"], 120).await;
    assert_eq!(completed["status"], "completed", "{completed}");
    assert!(completed["output_media"]["verified"] == true);
    println!("PASS: slow media download cancellation stopped owned PID; retry completed and FFprobe verified");
    let mut secondsettings = settings.clone();
    secondsettings["directory"] = json!(work.join("recovered"));
    let second = core
        .dispatch(
            "enqueue",
            json!({"entries":entries,"settings":secondsettings}),
        )
        .await
        .unwrap();
    let secondid = second[0]["id"].as_str().unwrap().to_owned();
    let pid = own_pid(&core, &secondid).await;
    wait(&core, &secondid, &["downloading"], 20).await;
    core.shutdown();
    stopped(&[pid]).await;
    drop(core);
    let reopened = Core::new(data, root.join("engine"), Arc::new(|_| {})).unwrap();
    assert_eq!(task(&reopened, &secondid).await["status"], "interrupted");
    retry(&reopened, &secondid).await;
    let recovered = wait(&reopened, &secondid, &["completed", "failed"], 120).await;
    assert_eq!(recovered["status"], "completed", "{recovered}");
    println!(
        "PASS: shutdown killed active PID, reopened state interrupted, manual retry completed"
    );
    let pids_file = work.join("owned-child-pids.json");
    let hook = format!(
        "before_dl:\"{python}\" \"{child}\" \"{}\"",
        pids_file.display()
    );
    let cmd = serde_json::to_string(&vec![
        "--exec".to_owned(),
        hook,
        "--no-simulate".into(),
        "--no-playlist".into(),
        "--paths".into(),
        work.join("tree").to_string_lossy().into_owned(),
        format!("{base}/sample.mp4"),
    ])
    .unwrap();
    let command = reopened
        .dispatch(
            "run_command",
            json!({"args":cmd,"settings":{"advanced":{},"proxy_mode":"direct"}}),
        )
        .await
        .unwrap();
    let id = command["id"].as_str().unwrap();
    let parent = own_pid(&reopened, id).await;
    let start = Instant::now();
    while !pids_file.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "hook did not run: {}",
            task(&reopened, id).await
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut pids: Vec<u32> = serde_json::from_slice(&std::fs::read(&pids_file).unwrap()).unwrap();
    assert!(pids.iter().all(|p| process_alive(*p)));
    pids.push(parent);
    reopened
        .dispatch("task_action", json!({"ids":[id],"action":"cancel"}))
        .await
        .unwrap();
    stopped(&pids).await;
    println!("PASS: cancellation killed yt-dlp plus independently recorded child and grandchild PIDs {pids:?}");
    let report = json!({"status":"passed","cancel_retry":completed,"shutdown_recovery":recovered,"cancelled_tree_pids":pids});
    std::fs::write(work.join("report.json"), report.to_string()).unwrap();
    reopened.shutdown();
    println!("REPORT: {}", work.join("report.json").display());
}
