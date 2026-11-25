// SPDX-License-Identifier: Apache-2.0
//! Explicit native shell operations. Call blocking dialogs from a worker thread.
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub fn dispatch(op: &str, payload: Value, engine: &Path) -> Result<Option<Value>, String> {
    if crate::lifecycle::isolation_active()
        && matches!(
            op,
            "pick_folder"
                | "pick_cookie"
                | "pick_file"
                | "open_path"
                | "open_licenses"
                | "export_file"
                | "import_file"
        )
    {
        return Err("隔离运行不打开系统对话框或资源管理器，以免写入系统最近使用记录".into());
    }
    let value = match op {
        "pick_folder" | "pick_cookie" | "pick_file" => {
            let mut dialog = rfd::FileDialog::new();
            if let Some(initial) = payload["initial"].as_str().filter(|p| !p.is_empty()) {
                dialog = dialog.set_directory(initial);
            }
            if let Some(title) = payload["title"].as_str() {
                dialog = dialog.set_title(title);
            }
            let path = if op == "pick_folder" {
                dialog.pick_folder()
            } else if op == "pick_cookie" {
                dialog.add_filter("Cookies", &["txt"]).pick_file()
            } else {
                dialog.pick_file()
            };
            path.map(|p| json!(p.to_string_lossy()))
                .unwrap_or(Value::Null)
        }
        "open_path" => {
            let path = PathBuf::from(payload["path"].as_str().ok_or("没有文件路径")?);
            open_path(&path)?;
            Value::Null
        }
        "open_licenses" => {
            open_path(&engine.join("licenses"))?;
            Value::Null
        }
        "export_file" => {
            let name = payload["name"].as_str().unwrap_or("flow-export.json");
            let content = payload["content"].as_str().ok_or("没有可导出内容")?;
            match rfd::FileDialog::new().set_file_name(name).save_file() {
                Some(path) => {
                    fs::write(&path, content).map_err(|e| e.to_string())?;
                    json!(path.to_string_lossy())
                }
                None => Value::Null,
            }
        }
        "import_file" => match rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
        {
            Some(path) => {
                if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
                    return Err("配置文件超过 8 MB".into());
                }
                let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                json!({"path":path.to_string_lossy(),"content":content})
            }
            None => Value::Null,
        },
        _ => return Ok(None),
    };
    Ok(Some(value))
}

pub fn confirm_exit() -> bool {
    if crate::lifecycle::isolation_active() {
        return false;
    }
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    let stop = "停止并退出".to_string();
    matches!(MessageDialog::new().set_title("退出流光")
        .set_description("退出将停止当前下载。临时文件会保留，下次启动后可重试继续。")
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::OkCancelCustom(stop.clone(), "继续下载".into())).show(),
        MessageDialogResult::Custom(value) if value == stop)
}

#[cfg(target_os = "macos")]
pub fn confirm_cleanup() -> bool {
    if crate::lifecycle::isolation_active() {
        return false;
    }
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    let clean = "清理并退出".to_string();
    matches!(MessageDialog::new().set_title("清理流光数据")
        .set_description("将停止下载，退出并清理流光的设置、历史、缓存、私有引擎及临时文件。视频保留。请先关闭其他流光窗口和 CLI；清理后可将应用移入废纸篓。")
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::OkCancelCustom(clean.clone(), "取消".into())).show(),
        MessageDialogResult::Custom(value) if value == clean)
}

pub fn explain_tray() {
    if crate::lifecycle::isolation_active() {
        return;
    }
    let description = if cfg!(target_os = "macos") {
        "流光会收起窗口并继续下载。可从 Dock 或菜单栏重新打开，在流光菜单中退出，或在设置中修改关闭行为。"
    } else {
        "流光会收起到系统托盘并继续下载。可从托盘退出，或在设置中修改关闭行为。"
    };
    rfd::MessageDialog::new()
        .set_title("关闭窗口后继续下载")
        .set_description(description)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

pub fn open_path(path: &Path) -> Result<(), String> {
    if crate::lifecycle::isolation_active() {
        return Err("隔离运行不打开资源管理器".into());
    }
    if !path.is_absolute() || !path.exists() {
        return Err("路径不存在，请确认文件尚未移动".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("explorer.exe");
        if path.is_file() {
            command.arg(format!("/select,{}", path.display()));
        } else {
            command.arg(path);
        }
        command
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/open");
        if path.is_file() {
            command.arg("-R");
        }
        command
            .arg("--")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let folder = if path.is_dir() {
            path
        } else {
            path.parent().ok_or("文件目录无效")?
        };
        Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub fn notify(title: &str, body: &str) -> Result<(), String> {
    if crate::lifecycle::isolation_active() {
        return Err("隔离模式不写入系统通知历史".into());
    }
    #[cfg(windows)]
    {
        use windows::{
            core::HSTRING,
            Data::Xml::Dom::XmlDocument,
            Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
            UI::Notifications::{ToastNotification, ToastNotificationManager},
        };
        let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
        let result = (|| -> windows::core::Result<()> {
            let document = XmlDocument::new()?;
            document.LoadXml(&HSTRING::from(format!("<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",xml_escape(title),xml_escape(body))))?;
            let toast = ToastNotification::CreateToastNotification(&document)?;
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(crate::lifecycle::APP_ID))?.Show(&toast)
        })().map_err(|e| e.to_string());
        if initialized {
            unsafe {
                RoUninitialize();
            }
        }
        result
    }
    #[cfg(target_os = "macos")]
    {
        crate::desktop::notify_macos(title, body)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (title, body);
        Err("当前平台尚未验证系统通知".into())
    }
}

#[cfg(windows)]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
