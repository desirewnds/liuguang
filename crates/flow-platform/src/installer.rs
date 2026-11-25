// SPDX-License-Identifier: Apache-2.0
//! Native installer process/ownership checks and an owned shortcut's AppUserModelID.
use crate::lifecycle::safe_path;
use std::path::Path;

#[cfg(windows)]
struct Process {
    id: u32,
    parent: u32,
    name: String,
    path: Option<std::path::PathBuf>,
}

#[cfg(windows)]
fn processes() -> Result<Vec<Process>, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{Diagnostics::ToolHelp::*, Threading::*},
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut entry = unsafe { std::mem::zeroed::<PROCESSENTRY32W>() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut output = Vec::new();
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) };
    while more != 0 {
        let length = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
        let handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID) };
        let path = if handle.is_null() {
            None
        } else {
            let mut data = vec![0u16; 32768];
            let mut length = data.len() as u32;
            let ok =
                unsafe { QueryFullProcessImageNameW(handle, 0, data.as_mut_ptr(), &mut length) };
            unsafe {
                CloseHandle(handle);
            }
            (ok != 0).then(|| {
                std::path::PathBuf::from(String::from_utf16_lossy(&data[..length as usize]))
            })
        };
        output.push(Process {
            id: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            name,
            path,
        });
        more = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    unsafe {
        CloseHandle(snapshot);
    }
    Ok(output)
}

fn normalized(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let path = if let Some(unc) = path.strip_prefix("//?/unc/") {
        format!("//{unc}")
    } else {
        path.trim_start_matches("//?/").to_string()
    };
    path.trim_end_matches('/').to_string()
}

#[cfg(windows)]
fn check_process_snapshot(root: &Path, own: u32, processes: &[Process]) -> Result<(), String> {
    use std::collections::BTreeSet;
    let prefix = normalized(root) + "/";
    let mut active = processes
        .iter()
        .filter(|p| {
            p.id != own
                && p.name.to_lowercase() != "uninstall.exe"
                && p.path
                    .as_ref()
                    .is_some_and(|path| normalized(path).starts_with(&prefix))
        })
        .map(|p| p.id)
        .collect::<BTreeSet<_>>();
    loop {
        let previous = active.len();
        for process in processes {
            if process.id != own && active.contains(&process.parent) {
                active.insert(process.id);
            }
        }
        if previous == active.len() {
            break;
        }
    }
    if !active.is_empty() {
        return Err(format!(
            "该安装目录仍有 {} 个运行进程，请退出流光和命令行工具后重试",
            active.len()
        ));
    }
    // An inaccessible namesake cannot be proven unrelated; fail conservatively.
    if processes.iter().any(|p| {
        p.id != own
            && p.path.is_none()
            && matches!(
                p.name.to_lowercase().as_str(),
                "flow-desktop.exe" | "flow-cli.exe"
            )
    }) {
        return Err("无法确认另一个流光进程的状态，请退出后重试".into());
    }
    Ok(())
}

pub fn check_processes(root: &Path) -> Result<(), String> {
    safe_path(root)?;
    #[cfg(windows)]
    check_process_snapshot(root, std::process::id(), &processes()?)?;
    Ok(())
}

/// Older Tauri builds do not participate in the native named-event protocol.
pub fn check_legacy_instance() -> Result<(), String> {
    if [
        "FLOW_LOCAL_DATA_DIR",
        "FLOW_ROAMING_DATA_DIR",
        "FLOW_DOWNLOAD_DIR",
    ]
    .iter()
    .all(|name| std::env::var_os(name).is_some())
    {
        return Ok(());
    }
    #[cfg(windows)]
    if processes()?
        .iter()
        .any(|p| p.id != std::process::id() && p.name.eq_ignore_ascii_case("flow-desktop.exe"))
    {
        return Err("另一个流光桌面版仍在运行，请先退出旧窗口及托盘中的流光后再启动".into());
    }
    Ok(())
}

/// Reads only the link's stored path; it does not resolve network targets or execute it.
pub fn check_shortcut(link: &Path, target: &Path) -> Result<(), String> {
    shortcut(link, target, false)
}

/// Adds notification identity only to a shortcut whose target and arguments match.
/// This modifies the link, not the registry or the user's environment.
pub fn register_shortcut(link: &Path, target: &Path) -> Result<(), String> {
    if crate::lifecycle::isolation_active() {
        return Err("隔离运行不注册系统快捷方式身份".into());
    }
    shortcut(link, target, true)
}

fn shortcut(link: &Path, target: &Path, register: bool) -> Result<(), String> {
    safe_path(link)?;
    safe_path(target)?;
    if !link.is_file() {
        return Err("快捷方式不存在".into());
    }
    #[cfg(windows)]
    {
        use windows::{
            core::{Interface, PCWSTR},
            Win32::{System::Com::*, UI::Shell::*},
        };
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|e| e.to_string())?;
        let result = (|| -> Result<(), String> {
            let shell: IShellLinkW =
                unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
                    .map_err(|e| e.to_string())?;
            let file: IPersistFile = shell.cast().map_err(|e| e.to_string())?;
            let filename = link
                .to_string_lossy()
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>();
            unsafe {
                file.Load(
                    PCWSTR(filename.as_ptr()),
                    if register { STGM_READWRITE } else { STGM_READ },
                )
            }
            .map_err(|e| e.to_string())?;
            let mut buffer = vec![0u16; 32768];
            unsafe { shell.GetPath(&mut buffer, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32) }
                .map_err(|e| e.to_string())?;
            let length = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
            let actual = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
            if normalized(&actual) != normalized(target) {
                return Err("快捷方式属于其他目标，已保留".into());
            }
            buffer.fill(0);
            unsafe { shell.GetArguments(&mut buffer) }.map_err(|e| e.to_string())?;
            if buffer[0] != 0 {
                return Err("快捷方式含自定义参数，已保留".into());
            }
            if register {
                use windows::{
                    core::{GUID, PWSTR},
                    Win32::{
                        Foundation::PROPERTYKEY,
                        System::{
                            Com::StructuredStorage::{
                                PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                            },
                            Variant::VT_LPWSTR,
                        },
                        UI::Shell::PropertiesSystem::IPropertyStore,
                    },
                };
                let store: IPropertyStore = shell.cast().map_err(|e| e.to_string())?;
                let key = PROPERTYKEY {
                    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
                    pid: 5,
                };
                let value = crate::lifecycle::APP_ID
                    .encode_utf16()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                let data =
                    unsafe { CoTaskMemAlloc(value.len() * std::mem::size_of::<u16>()) } as *mut u16;
                if data.is_null() {
                    return Err("无法分配快捷方式属性内存".into());
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(value.as_ptr(), data, value.len());
                }
                // PROPVARIANT's Drop releases the COM-allocated string on every path.
                let property = PROPVARIANT {
                    Anonymous: PROPVARIANT_0 {
                        Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                            vt: VT_LPWSTR,
                            wReserved1: 0,
                            wReserved2: 0,
                            wReserved3: 0,
                            Anonymous: PROPVARIANT_0_0_0 {
                                pwszVal: PWSTR(data),
                            },
                        }),
                    },
                };
                unsafe { store.SetValue(&key, &property) }.map_err(|e| e.to_string())?;
                unsafe { store.Commit() }.map_err(|e| e.to_string())?;
                unsafe { file.Save(PCWSTR(filename.as_ptr()), true) }.map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        unsafe {
            CoUninitialize();
        }
        result
    }
    #[cfg(not(windows))]
    {
        let _ = (target, register);
        Err("快捷方式检查仅支持 Windows".into())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn process(id: u32, parent: u32, name: &str, path: Option<&str>) -> Process {
        Process {
            id,
            parent,
            name: name.into(),
            path: path.map(Into::into),
        }
    }

    #[test]
    fn process_check_tracks_descendants_and_respects_directory_boundaries() {
        let root = Path::new(r"C:\Flow");
        let own = process(10, 0, "flow-cli.exe", Some(r"C:\Flow\flow-cli.exe"));
        let uninstaller = process(11, 0, "uninstall.exe", Some(r"C:\Flow\uninstall.exe"));
        let neighbor = process(12, 0, "node.exe", Some(r"C:\Flow-other\node.exe"));
        assert!(check_process_snapshot(root, 10, &[own, uninstaller, neighbor]).is_ok());
        let engine = process(20, 0, "node.exe", Some(r"c:\flow\engine\node.exe"));
        let child = process(21, 20, "ffmpeg.exe", Some(r"C:\Other\ffmpeg.exe"));
        let grandchild = process(22, 21, "helper.exe", None);
        let error = check_process_snapshot(root, 10, &[grandchild, child, engine]).unwrap_err();
        assert!(error.contains("3 个"));
        assert!(check_process_snapshot(root, 10, &[process(30, 0, "flow-cli.exe", None)]).is_err());
    }

    #[test]
    fn extended_unc_paths_share_the_same_comparison_identity() {
        assert_eq!(
            normalized(Path::new(r"\\?\UNC\Server\Share\Flow")),
            normalized(Path::new(r"\\server\share\flow"))
        );
        assert_eq!(
            normalized(Path::new(r"\\?\C:\Flow")),
            normalized(Path::new(r"c:\flow"))
        );
    }
}
