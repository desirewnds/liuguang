// SPDX-License-Identifier: Apache-2.0
//! Native tray/message pump and state, without a browser or persistent polling.
use crate::lifecycle::{safe_path, Paths};
use serde::{Deserialize, Serialize};
use std::{fs, sync::Arc};

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod macos;

#[cfg(windows)]
const NOTIFY_MESSAGE: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 0x46f;

/// A bounded, nonblocking sender. The native shell thread owns toast delivery.
#[derive(Clone)]
pub struct NotificationSender {
    sender: std::sync::mpsc::SyncSender<(String, String)>,
    wake_pending: Arc<std::sync::atomic::AtomicBool>,
    thread_id: u32,
    enabled: bool,
    #[cfg(target_os = "macos")]
    wake: Option<Arc<macos::Wake>>,
}
impl NotificationSender {
    /// Returns false if isolated, stopped or the queue is full. Never waits on Windows.
    pub fn notify(&self, title: &str, body: &str) -> bool {
        if !self.enabled || self.thread_id == 0 {
            return false;
        }
        if self
            .sender
            .try_send((
                title.chars().take(256).collect(),
                body.chars().take(1024).collect(),
            ))
            .is_err()
        {
            return false;
        }
        if self
            .wake_pending
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return true;
        }
        #[cfg(windows)]
        {
            let posted = unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    self.thread_id,
                    NOTIFY_MESSAGE,
                    0,
                    0,
                )
            } != 0;
            if !posted {
                self.wake_pending
                    .store(false, std::sync::atomic::Ordering::Release);
            }
            return posted;
        }
        #[cfg(target_os = "macos")]
        {
            let posted = self.wake.as_ref().is_some_and(|wake| wake.notify());
            if !posted {
                self.wake_pending
                    .store(false, std::sync::atomic::Ordering::Release);
            }
            return posted;
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        false
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ShellEvent {
    Show,
    Exit,
    NewDownload,
    Settings,
    Hidden,
}

/// Local task timestamp using Windows' time-zone/DST rules; no new date library.
pub fn format_timestamp(millis: u64) -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{FILETIME, SYSTEMTIME},
            System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx},
        };
        let Some(ticks) = millis
            .checked_mul(10_000)
            .and_then(|value| value.checked_add(116_444_736_000_000_000))
        else {
            return "—".into();
        };
        let file = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut utc = unsafe { std::mem::zeroed::<SYSTEMTIME>() };
        let mut local = unsafe { std::mem::zeroed::<SYSTEMTIME>() };
        if unsafe { FileTimeToSystemTime(&file, &mut utc) } == 0
            || unsafe { SystemTimeToTzSpecificLocalTimeEx(std::ptr::null(), &utc, &mut local) } == 0
        {
            return "—".into();
        }
        return format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute
        );
    }
    #[cfg(target_os = "macos")]
    {
        macos::format_timestamp(millis)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = millis;
        "—".into()
    }
}

/// Exact existing Flow icon, shared by the native window and system tray.
pub fn icon_rgba() -> Result<(Vec<u8>, u32, u32), String> {
    let image = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Ok((image.into_raw(), width, height))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
    /// Scale used for stored screen coordinates; dimensions are always logical.
    pub position_scale: f32,
    pub maximized: bool,
}
impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1180.0,
            height: 820.0,
            x: None,
            y: None,
            position_scale: 1.0,
            maximized: false,
        }
    }
}
impl WindowState {
    pub fn normalized(mut self) -> Self {
        if !self.width.is_finite() {
            self.width = 1180.0;
        }
        if !self.height.is_finite() {
            self.height = 820.0;
        }
        self.width = self.width.clamp(620.0, 16384.0);
        self.height = self.height.clamp(360.0, 16384.0);
        self.x = self.x.filter(|x| x.is_finite());
        self.y = self.y.filter(|y| y.is_finite());
        if !self.position_scale.is_finite() || !(0.5..=8.).contains(&self.position_scale) {
            self.position_scale = 1.;
        }
        self
    }
}
pub fn load_window_state(paths: &Paths) -> WindowState {
    let path = paths.shell.join("native-window-state.json");
    if safe_path(&path).is_err() || fs::metadata(&path).map_or(true, |m| m.len() > 64 * 1024) {
        return visible_window_state(WindowState::default());
    }
    visible_window_state(
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<WindowState>(&bytes).ok())
            .unwrap_or_default()
            .normalized(),
    )
}

#[derive(Clone, Copy)]
struct WorkArea {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    /// Monitor scale divided by the current window scale.
    scale: f32,
}

fn fit_to_work_areas(state: WindowState, areas: &[WorkArea]) -> WindowState {
    let mut state = state.normalized();
    let valid = areas
        .iter()
        .filter(|area| {
            area.x.is_finite()
                && area.y.is_finite()
                && area.width.is_finite()
                && area.height.is_finite()
                && area.width > 0.
                && area.height > 0.
                && area.scale.is_finite()
                && area.scale > 0.
        })
        .collect::<Vec<_>>();
    let Some(default_area) = valid.first() else {
        return state;
    };
    let area = if let (Some(x), Some(y)) = (state.x, state.y) {
        valid
            .iter()
            .copied()
            .max_by(|a, b| {
                let overlap = |area: &WorkArea| {
                    let w = ((x + state.width * area.scale).min(area.x + area.width)
                        - x.max(area.x))
                    .max(0.);
                    let h =
                        ((y + 32. * area.scale).min(area.y + area.height) - y.max(area.y)).max(0.);
                    w * h
                };
                overlap(a).total_cmp(&overlap(b))
            })
            .filter(|area| {
                x < area.x + area.width
                    && x + state.width * area.scale > area.x
                    && y < area.y + area.height
                    && y + 32. * area.scale > area.y
            })
            .unwrap_or(default_area)
    } else {
        default_area
    };
    state.width = state.width.min(area.width / area.scale);
    state.height = state.height.min((area.height / area.scale - 40.).max(100.));
    if let (Some(x), Some(y)) = (state.x, state.y) {
        state.x = Some(x.clamp(
            area.x,
            (area.x + area.width - state.width * area.scale).max(area.x),
        ));
        state.y = Some(y.clamp(
            area.y,
            (area.y + area.height - (state.height + 40.) * area.scale).max(area.y),
        ));
    }
    state
}

/// Checks monitor work areas without changing display or process DPI settings.
pub fn visible_window_state(state: WindowState) -> WindowState {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{LPARAM, RECT},
            Graphics::Gdi::*,
            UI::{HiDpi::*, WindowsAndMessaging::MONITORINFOF_PRIMARY},
        };
        unsafe extern "system" fn collect(
            monitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            data: LPARAM,
        ) -> windows_sys::core::BOOL {
            let (current_scale, areas) =
                unsafe { &mut *(data as *mut (f32, Vec<(bool, WorkArea)>)) };
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..unsafe { std::mem::zeroed() }
            };
            if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
                return 1;
            }
            let (mut dx, mut dy) = (96, 96);
            let dpi_ok =
                unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) } >= 0;
            let scale = if dpi_ok && dx > 0 {
                dx as f32 / 96.
            } else {
                1.
            };
            let work = info.rcWork;
            areas.push((
                info.dwFlags & MONITORINFOF_PRIMARY != 0,
                WorkArea {
                    x: work.left as f32 / *current_scale,
                    y: work.top as f32 / *current_scale,
                    width: (work.right - work.left) as f32 / *current_scale,
                    height: (work.bottom - work.top) as f32 / *current_scale,
                    scale: scale / *current_scale,
                },
            ));
            1
        }
        let window = main_window();
        let dpi = if window.is_null() {
            unsafe { GetDpiForSystem() }
        } else {
            unsafe { GetDpiForWindow(window) }
        }
        .max(96);
        let current_scale = dpi as f32 / 96.;
        let mut state = state.normalized();
        let ratio = state.position_scale / current_scale;
        state.x = state.x.map(|x| x * ratio);
        state.y = state.y.map(|y| y * ratio);
        state.position_scale = current_scale;
        let mut context = (current_scale, Vec::<(bool, WorkArea)>::new());
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(collect),
                &mut context as *mut _ as isize,
            );
        }
        context.1.sort_by_key(|(primary, _)| !*primary);
        fit_to_work_areas(
            state,
            &context
                .1
                .into_iter()
                .map(|(_, area)| area)
                .collect::<Vec<_>>(),
        )
    }
    #[cfg(target_os = "macos")]
    {
        macos::visible_window_state(state)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    state.normalized()
}

#[cfg(windows)]
fn main_window() -> windows_sys::Win32::Foundation::HWND {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::*,
    };
    unsafe extern "system" fn find(window: HWND, data: LPARAM) -> windows_sys::core::BOOL {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut pid);
        }
        if pid != unsafe { GetCurrentProcessId() } {
            return 1;
        }
        let mut title = [0u16; 16];
        let count = unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
        if count == 2 && title[..2] == [0x6d41, 0x5149] {
            unsafe {
                *(data as *mut HWND) = window;
            }
            return 0;
        }
        1
    }
    let mut window: HWND = std::ptr::null_mut();
    unsafe {
        EnumWindows(Some(find), &mut window as *mut _ as isize);
    }
    window
}

/// Captures the main window's restore bounds, so maximized/minimized sizes are not persisted.
/// All Win32 calls are read-only and restricted to this process's window titled 流光.
pub fn capture_window_state(fallback: &WindowState) -> WindowState {
    #[cfg(target_os = "macos")]
    return macos::capture_window_state(fallback);
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::RECT,
            Graphics::Gdi::*,
            UI::{HiDpi::*, WindowsAndMessaging::*},
        };
        let window = main_window();
        if !window.is_null() {
            let mut placement = WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..unsafe { std::mem::zeroed() }
            };
            if unsafe { GetWindowPlacement(window, &mut placement) } != 0 {
                let dpi = unsafe { GetDpiForWindow(window) }.max(96);
                let scale = dpi as f32 / 96.;
                let mut bounds = placement.rcNormalPosition;
                let monitor = unsafe { MonitorFromRect(&bounds, MONITOR_DEFAULTTONEAREST) };
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..unsafe { std::mem::zeroed() }
                };
                if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
                    // WINDOWPLACEMENT restore coordinates are relative to the work area.
                    bounds.left += info.rcWork.left - info.rcMonitor.left;
                    bounds.top += info.rcWork.top - info.rcMonitor.top;
                    bounds.right += info.rcWork.left - info.rcMonitor.left;
                    bounds.bottom += info.rcWork.top - info.rcMonitor.top;
                }
                let mut frame = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                let adjusted = unsafe {
                    AdjustWindowRectExForDpi(
                        &mut frame,
                        GetWindowLongW(window, GWL_STYLE) as u32 & !(WS_MAXIMIZE | WS_MINIMIZE),
                        0,
                        GetWindowLongW(window, GWL_EXSTYLE) as u32,
                        dpi,
                    )
                } != 0;
                let (frame_width, frame_height) = if adjusted {
                    (frame.right - frame.left, frame.bottom - frame.top)
                } else {
                    (0, 0)
                };
                return visible_window_state(WindowState {
                    width: (bounds.right - bounds.left - frame_width) as f32 / scale,
                    height: (bounds.bottom - bounds.top - frame_height) as f32 / scale,
                    x: Some(bounds.left as f32 / scale),
                    y: Some(bounds.top as f32 / scale),
                    position_scale: scale,
                    maximized: placement.showCmd == SW_SHOWMAXIMIZED as u32
                        || placement.flags & WPF_RESTORETOMAXIMIZED != 0,
                });
            }
        }
    }
    visible_window_state(fallback.clone())
}
pub fn save_window_state(paths: &Paths, state: &WindowState) -> Result<(), String> {
    paths.initialize()?;
    let path = paths.shell.join("native-window-state.json");
    safe_path(&path)?;
    let staged = paths
        .shell
        .join(format!("window-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        fs::write(
            &staged,
            serde_json::to_vec(&state.clone().normalized()).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(&staged, &path).map_err(|e| e.to_string())
    })();
    if staged.exists() {
        let _ = fs::remove_file(staged);
    }
    result
}

#[cfg(windows)]
fn instance_name(scope: &str, root: &std::path::Path, suffix: &str) -> Vec<u16> {
    use sha2::{Digest, Sha256};
    // Resolve existing ancestors too: a new data directory may still have a
    // short-name/extended-path alias in an already existing parent directory.
    let canonical = root
        .ancestors()
        .find_map(|ancestor| {
            fs::canonicalize(ancestor).ok().and_then(|parent| {
                root.strip_prefix(ancestor)
                    .ok()
                    .map(|rest| parent.join(rest))
            })
        })
        .unwrap_or_else(|| root.to_path_buf());
    let key = format!(
        "{:x}",
        Sha256::digest(crate::lifecycle::normalized(&canonical).as_bytes())
    );
    format!(
        "Local\\{}.{scope}.{}.{suffix}\0",
        crate::lifecycle::APP_ID,
        &key[..24]
    )
    .encode_utf16()
    .collect()
}

/// A named marker and show event. Handles own their lifetime across threads;
/// no thread-owned mutex lock or on-disk lockfile is needed.
#[cfg(windows)]
struct InstanceGuard {
    primary: bool,
    mutex: usize,
    event: usize,
}
#[cfg(windows)]
impl InstanceGuard {
    fn acquire(scope: &str, root: &std::path::Path) -> Result<Self, String> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS},
            System::Threading::{CreateEventW, CreateMutexW},
        };
        safe_path(root)?;
        let event = unsafe {
            CreateEventW(
                std::ptr::null(),
                0,
                0,
                instance_name(scope, root, "show").as_ptr(),
            )
        };
        if event.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mutex = unsafe {
            CreateMutexW(
                std::ptr::null(),
                0,
                instance_name(scope, root, "instance").as_ptr(),
            )
        };
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if mutex.is_null() {
            let error = std::io::Error::last_os_error().to_string();
            unsafe {
                CloseHandle(event);
            }
            return Err(error);
        }
        Ok(Self {
            primary: !existed,
            mutex: mutex as usize,
            event: event as usize,
        })
    }
    fn show(&self) -> Result<(), String> {
        if unsafe { windows_sys::Win32::System::Threading::SetEvent(self.event as _) } == 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }
}
#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.event as _);
            windows_sys::Win32::Foundation::CloseHandle(self.mutex as _);
        }
    }
}

/// A secondary instance signals the existing owner without opening its Core DB.
pub struct NativeShell {
    primary: bool,
    notifications: NotificationSender,
    #[cfg(windows)]
    _instances: Vec<InstanceGuard>,
    #[cfg(windows)]
    thread_id: u32,
    thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(target_os = "macos")]
    mac: Option<macos::MacShell>,
}
impl NativeShell {
    pub fn is_primary(&self) -> bool {
        self.primary
    }
    pub fn notifications(&self) -> NotificationSender {
        self.notifications.clone()
    }
    pub fn tray_available(&self) -> bool {
        self.primary && self.notifications.enabled
    }
    pub fn new(
        paths: &Paths,
        callback: Arc<dyn Fn(ShellEvent) + Send + Sync>,
    ) -> Result<Self, String> {
        #[cfg(windows)]
        {
            let mut instances = Vec::with_capacity(2);
            for (scope, path) in [("shell", &paths.shell), ("core", &paths.data_base)] {
                let instance = InstanceGuard::acquire(scope, path)?;
                let primary = instance.primary;
                if !primary {
                    instance.show()?;
                }
                instances.push(instance);
                // Do not acquire the next scope after a collision: two simultaneous
                // startups must not each claim one scope and reject each other.
                if !primary {
                    return Ok(Self {
                        primary: false,
                        notifications: NotificationSender {
                            sender: std::sync::mpsc::sync_channel(1).0,
                            wake_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                            thread_id: 0,
                            enabled: false,
                        },
                        _instances: instances,
                        thread_id: 0,
                        thread: None,
                    });
                }
            }
            crate::installer::check_legacy_instance()?;
            let app_id = crate::lifecycle::APP_ID
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>();
            if !crate::lifecycle::isolation_active() {
                unsafe {
                    windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
                        app_id.as_ptr(),
                    );
                }
            }
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let (notify_tx, notify_rx) = std::sync::mpsc::sync_channel(8);
            let wake_pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let wake_loop = wake_pending.clone();
            let events = instances.iter().map(|instance| instance.event).collect();
            let thread = std::thread::Builder::new()
                .name("flow-native-shell".into())
                .spawn(move || {
                    native_loop(events, callback, tx, notify_rx, wake_loop);
                })
                .map_err(|error| error.to_string())?;
            match rx.recv().unwrap_or_else(|e| Err(e.to_string())) {
                Ok(thread_id) => Ok(Self {
                    primary: true,
                    notifications: NotificationSender {
                        sender: notify_tx,
                        wake_pending,
                        thread_id,
                        enabled: !crate::lifecycle::isolation_active(),
                    },
                    _instances: instances,
                    thread_id,
                    thread: Some(thread),
                }),
                Err(error) => {
                    let _ = thread.join();
                    Err(error)
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            let mac = macos::MacShell::new(paths, callback)?;
            Ok(Self {
                primary: mac.is_primary(),
                notifications: mac.notifications(),
                thread: None,
                mac: Some(mac),
            })
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (paths, callback);
            Err("当前原生外壳仅支持 Windows".into())
        }
    }
}
impl Drop for NativeShell {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        drop(self.mac.take());
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
            if self.thread_id != 0 {
                PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            // InstanceGuard fields drop after the message pump has stopped.
        }
    }
}

/// AppKit owns activation independently of a window's visibility. Call when
/// hiding/restoring the Iced window; Windows continues to use its existing path.
pub fn hide_application() {
    #[cfg(target_os = "macos")]
    macos::hide_application();
}

pub fn activate_application() {
    #[cfg(target_os = "macos")]
    macos::activate_application();
}

/// Only call after the application has approved exit and stopped its tasks.
/// This lets AppKit's final termination proceed without another quit prompt.
pub fn prepare_exit() {
    #[cfg(target_os = "macos")]
    macos::prepare_exit();
}

#[cfg(target_os = "macos")]
pub(crate) fn notify_macos(title: &str, body: &str) -> Result<(), String> {
    macos::notify(title, body)
}

#[cfg(windows)]
fn native_loop(
    events: Vec<usize>,
    callback: Arc<dyn Fn(ShellEvent) + Send + Sync>,
    ready: std::sync::mpsc::SyncSender<Result<u32, String>>,
    notifications: std::sync::mpsc::Receiver<(String, String)>,
    wake_pending: Arc<std::sync::atomic::AtomicBool>,
) {
    use tray_icon::{
        menu::{Menu, MenuEvent, MenuItem},
        Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
    };
    use windows_sys::Win32::{
        Foundation::HANDLE, System::Threading::GetCurrentThreadId, UI::WindowsAndMessaging::*,
    };
    let setup = (|| {
        if crate::lifecycle::isolation_active() {
            return Ok(None);
        }
        let menu = Menu::new();
        let show = MenuItem::with_id("show", "显示流光", true, None);
        let exit = MenuItem::with_id("exit", "退出流光", true, None);
        menu.append_items(&[&show, &exit])
            .map_err(|e| e.to_string())?;
        let on_menu = callback.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.as_ref() {
            "show" => on_menu(ShellEvent::Show),
            "exit" => on_menu(ShellEvent::Exit),
            _ => {}
        }));
        let on_tray = callback.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                on_tray(ShellEvent::Show);
            }
        }));
        let (rgba, width, height) = icon_rgba()?;
        TrayIconBuilder::new()
            .with_tooltip("流光")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_icon(Icon::from_rgba(rgba, width, height).map_err(|e| e.to_string())?)
            .build()
            .map(Some)
            .map_err(|e| e.to_string())
    })();
    let tray = match setup {
        Ok(tray) => tray,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut msg = unsafe { std::mem::zeroed::<MSG>() };
    // Ensure queue exists before the drop handler can PostThreadMessage(WM_QUIT).
    unsafe {
        PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
    }
    if ready.send(Ok(unsafe { GetCurrentThreadId() })).is_err() {
        return;
    }
    let events = events
        .into_iter()
        .map(|event| event as HANDLE)
        .collect::<Vec<_>>();
    'messages: loop {
        let result = unsafe {
            MsgWaitForMultipleObjects(
                events.len() as u32,
                events.as_ptr(),
                0,
                u32::MAX,
                QS_ALLINPUT,
            )
        };
        if result < events.len() as u32 {
            callback(ShellEvent::Show);
        } else if result == u32::MAX {
            break;
        }
        while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_QUIT {
                break 'messages;
            }
            if msg.message == NOTIFY_MESSAGE {
                // Clear before draining: a producer arriving during delivery
                // posts one new wake; it cannot strand an item or flood Win32.
                wake_pending.store(false, std::sync::atomic::Ordering::Release);
                for (title, body) in notifications.try_iter().take(8) {
                    let _ = crate::shell::notify(&title, &body);
                }
                continue;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    drop(tray);
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    fn instance_fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("test-artifacts/instance-scopes")
            .join(uuid::Uuid::new_v4().to_string())
    }
    #[cfg(windows)]
    #[test]
    fn instance_names_normalize_aliases_and_keep_shell_and_core_separate() {
        let root = instance_fixture_path();
        let upper = std::path::PathBuf::from(root.to_string_lossy().to_uppercase());
        assert_eq!(
            instance_name("core", &root, "instance"),
            instance_name("core", &upper, "instance")
        );
        assert_ne!(
            instance_name("shell", &root, "instance"),
            instance_name("core", &root, "instance")
        );
        assert_ne!(
            instance_name("core", &root, "instance"),
            instance_name("core", &root.join("other"), "instance")
        );
    }
    #[cfg(windows)]
    #[test]
    fn different_shells_share_one_core_guard_and_wake_its_owner() {
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject,
        };
        let root = instance_fixture_path();
        let first_shell = InstanceGuard::acquire("shell", &root.join("first-shell")).unwrap();
        let first_core = InstanceGuard::acquire("core", &root.join("data")).unwrap();
        assert!(first_shell.primary && first_core.primary);
        let second_shell = InstanceGuard::acquire("shell", &root.join("second-shell")).unwrap();
        let second_core = InstanceGuard::acquire("core", &root.join("data")).unwrap();
        assert!(second_shell.primary);
        assert!(!second_core.primary);
        second_core.show().unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(first_core.event as _, 0) },
            WAIT_OBJECT_0
        );
        drop((second_core, second_shell, first_core, first_shell));
        let next = InstanceGuard::acquire("core", &root.join("data")).unwrap();
        assert!(
            next.primary,
            "All handles must be released before the next owner starts"
        );
    }
    #[test]
    fn notification_sender_never_queues_when_disabled_or_full() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let disabled = NotificationSender {
            sender: sender.clone(),
            wake_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            thread_id: 1,
            enabled: false,
            #[cfg(target_os = "macos")]
            wake: None,
        };
        assert!(!disabled.notify("test", "test"));
        assert!(receiver.try_recv().is_err());
        sender
            .try_send(("queued".into(), "already".into()))
            .unwrap();
        let full = NotificationSender {
            sender,
            wake_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            thread_id: 1,
            enabled: true,
            #[cfg(target_os = "macos")]
            wake: None,
        };
        assert!(!full.notify("test", "test"));
        assert_eq!(
            receiver.try_recv().unwrap(),
            ("queued".into(), "already".into())
        );
    }

    #[test]
    fn offscreen_and_oversized_windows_return_to_a_visible_work_area() {
        let areas = [
            WorkArea {
                x: 0.,
                y: 0.,
                width: 1366.,
                height: 728.,
                scale: 1.,
            },
            WorkArea {
                x: -1920.,
                y: 0.,
                width: 1920.,
                height: 1040.,
                scale: 1.,
            },
        ];
        let mut state = WindowState {
            width: 2400.,
            height: 1600.,
            x: Some(5000.),
            y: Some(-3000.),
            position_scale: 1.,
            maximized: true,
        };
        state = fit_to_work_areas(state, &areas);
        assert_eq!((state.x, state.y), (Some(0.), Some(0.)));
        assert_eq!((state.width, state.height), (1366., 688.));
        assert!(state.maximized);
        let state = fit_to_work_areas(
            WindowState {
                width: 900.,
                height: 650.,
                x: Some(-1800.),
                y: Some(100.),
                position_scale: 1.,
                maximized: false,
            },
            &areas,
        );
        assert_eq!((state.x, state.y), (Some(-1800.), Some(100.)));
    }
    #[test]
    fn invalid_window_geometry_is_bounded() {
        let state = WindowState {
            width: f32::NAN,
            height: -10.,
            x: Some(f32::INFINITY),
            y: Some(50.),
            position_scale: f32::NAN,
            maximized: false,
        }
        .normalized();
        assert_eq!(state.width, 1180.);
        assert_eq!(state.height, 360.);
        assert_eq!(state.x, None);
        assert_eq!(state.y, Some(50.));
        assert_eq!(state.position_scale, 1.);
    }
    #[test]
    fn window_state_roundtrip_uses_owned_shell_only() {
        let workspace =
            fs::canonicalize(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
                .unwrap();
        let base = workspace.join("test-artifacts/window-state");
        safe_path(&base).unwrap();
        fs::create_dir_all(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        assert!(base.is_absolute() && base.starts_with(&workspace));
        let directory = base.join(uuid::Uuid::new_v4().to_string());
        assert_eq!(directory.parent(), Some(base.as_path()));
        safe_path(&directory).unwrap();
        fs::create_dir(&directory).unwrap();
        let outside_shell = directory.join("outside-shell.txt");
        fs::write(&outside_shell, b"preserve outside owned shell").unwrap();
        let roaming = directory.join("roaming");
        let local = directory.join("local");
        let paths =
            Paths::for_locations(roaming.clone(), local, roaming, directory.join("media")).unwrap();
        let value = WindowState {
            width: 900.,
            height: 650.,
            x: Some(20.),
            y: Some(40.),
            position_scale: 1.,
            maximized: true,
        };
        save_window_state(&paths, &value).unwrap();
        // Persistence is deterministic; monitor adaptation has separate pure tests.
        let restored: WindowState = serde_json::from_slice(
            &fs::read(paths.shell.join("native-window-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(restored.width, 900.);
        assert_eq!(restored.x, Some(20.));
        assert!(restored.maximized);
        assert!(!paths.shell.join("webview").exists());
        crate::lifecycle::cleanup(&paths, false).unwrap();
        assert!(!paths.shell.exists());
        assert!(!paths.local.exists());
        assert_eq!(
            fs::read(&outside_shell).unwrap(),
            b"preserve outside owned shell"
        );
        // Delete only this fixture's known sentinel and empty parent. Unexpected
        // files make the test fail rather than being silently removed recursively.
        safe_path(&outside_shell).unwrap();
        fs::remove_file(outside_shell).unwrap();
        safe_path(&directory).unwrap();
        fs::remove_dir(&directory).unwrap();
        // Other tests/runs may have their own fixtures under this shared parent.
        let _ = fs::remove_dir(base);
    }

    #[test]
    fn mixed_dpi_work_area_bounds_use_target_monitor_scale() {
        let areas = [
            WorkArea {
                x: 0.,
                y: 0.,
                width: 1920.,
                height: 1040.,
                scale: 1.,
            },
            WorkArea {
                x: 1920.,
                y: 0.,
                width: 2560.,
                height: 1400.,
                scale: 1.5,
            },
        ];
        let state = fit_to_work_areas(
            WindowState {
                width: 1000.,
                height: 700.,
                x: Some(4300.),
                y: Some(100.),
                position_scale: 1.,
                maximized: false,
            },
            &areas,
        );
        assert_eq!(state.width, 1000.);
        assert_eq!(state.x, Some(2980.));
        assert_eq!(state.y, Some(100.));
        let legacy: WindowState =
            serde_json::from_str(r#"{"width":900,"height":650,"x":20,"y":40,"maximized":true}"#)
                .unwrap();
        assert_eq!(legacy.position_scale, 1.);
    }
}
