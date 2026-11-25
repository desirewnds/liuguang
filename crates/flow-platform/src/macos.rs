// SPDX-License-Identifier: Apache-2.0
//! AppKit stays on the Iced main run loop. IPC waits in kqueue and never polls.
//! No replacement NSApplication delegate: winit continues to own window events.
use super::{fit_to_work_areas, icon_rgba, NotificationSender, ShellEvent, WindowState, WorkArea};
use crate::lifecycle::{safe_path, Paths};
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::{
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, Imp, ProtocolObject, Sel},
    Encode, MainThreadMarker,
};
use objc2_app_kit::{
    NSApplication, NSApplicationDidBecomeActiveNotification, NSApplicationTerminateReply,
    NSApplicationWillHideNotification, NSEvent, NSEventModifierFlags, NSEventType, NSScreen,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint, NSString};
use std::{
    cell::RefCell,
    fs::{DirBuilder, File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt},
    },
    path::Path,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};
use tray_icon::{
    menu::{
        accelerator::{Accelerator, Code, Modifiers},
        Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    },
    Icon, TrayIcon, TrayIconBuilder,
};

type Callback = Arc<dyn Fn(ShellEvent) + Send + Sync>;
const STOP: usize = 1;
const NOTIFY: usize = 2;

/// A descriptor can outlive the owner while cloned notification senders drain.
/// `stopped` prevents these senders from posting into a stopped event loop.
pub(super) struct Wake {
    queue: OwnedFd,
    stopped: AtomicBool,
}
impl Wake {
    fn new() -> Result<Arc<Self>, String> {
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let queue = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let wake = Arc::new(Self {
            queue,
            stopped: AtomicBool::new(false),
        });
        for id in [STOP, NOTIFY] {
            wake.change(id, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0)?;
        }
        Ok(wake)
    }
    fn change(&self, ident: usize, filter: i16, flags: u16, fflags: u32) -> Result<(), String> {
        let change = libc::kevent {
            ident,
            filter,
            flags,
            fflags,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let result = unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                &change,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        if result < 0 {
            Err(io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }
    pub(super) fn notify(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self
                .change(NOTIFY, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER)
                .is_ok()
    }
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.change(STOP, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER);
    }
}

/// The inode is deliberately retained between launches. Unlinking a flock file
/// while another process already has it open could allow two independent owners.
struct Instance {
    file: File,
    primary: bool,
}
impl Instance {
    fn acquire(root: &Path) -> Result<Self, String> {
        safe_path(root)?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)
            .map_err(|e| e.to_string())?;
        safe_path(root)?;
        let directory = std::fs::symlink_metadata(root).map_err(|e| e.to_string())?;
        if !directory.is_dir()
            || directory.uid() != unsafe { libc::geteuid() }
            || directory.mode() & 0o022 != 0
        {
            return Err("实例目录的所有者或写入权限不正确".into());
        }
        let path = root.join(".native-instance.lock");
        safe_path(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.len() > 1
            || metadata.mode() & 0o077 != 0
        {
            return Err("实例标记文件的类型、所有者或权限不正确".into());
        }
        let primary =
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                true
            } else {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                    return Err(error.to_string());
                }
                false
            };
        Ok(Self { file, primary })
    }
    fn show(&self) -> Result<(), String> {
        // A write triggers NOTE_WRITE even if the byte has not changed. No data,
        // command arguments or credentials travel across this wake channel.
        self.file.write_all_at(&[1], 0).map_err(|e| e.to_string())
    }
}

pub(super) struct MacShell {
    primary: bool,
    notifications: NotificationSender,
    wake: Option<Arc<Wake>>,
    thread: Option<std::thread::JoinHandle<()>>,
    _instances: Vec<Instance>,
    ui_installed: bool,
}
impl MacShell {
    pub(super) fn is_primary(&self) -> bool {
        self.primary
    }
    pub(super) fn notifications(&self) -> NotificationSender {
        self.notifications.clone()
    }
    pub(super) fn new(paths: &Paths, callback: Callback) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(8);
        let pending = Arc::new(AtomicBool::new(false));
        let disabled = NotificationSender {
            sender: sender.clone(),
            wake_pending: pending.clone(),
            thread_id: 0,
            enabled: false,
            wake: None,
        };
        let mut instances = Vec::with_capacity(2);
        for root in [&paths.shell, &paths.data_base] {
            let instance = Instance::acquire(root)?;
            if !instance.primary {
                instance.show()?;
                instances.push(instance);
                return Ok(Self {
                    primary: false,
                    notifications: disabled,
                    wake: None,
                    thread: None,
                    _instances: instances,
                    ui_installed: false,
                });
            }
            instances.push(instance);
        }
        let wake = Wake::new()?;
        for instance in &instances {
            wake.change(
                instance.file.as_raw_fd() as usize,
                libc::EVFILT_VNODE,
                libc::EV_ADD | libc::EV_CLEAR,
                libc::NOTE_WRITE,
            )?;
        }
        let installed = !crate::lifecycle::isolation_active();
        if installed {
            on_main({
                let callback = callback.clone();
                move |mtm| install_ui(mtm, callback)
            })?;
        }
        let loop_wake = wake.clone();
        let loop_pending = pending.clone();
        let thread = match std::thread::Builder::new()
            .name("flow-macos-shell".into())
            .spawn(move || event_loop(loop_wake, receiver, loop_pending, callback))
        {
            Ok(thread) => thread,
            Err(error) => {
                if installed {
                    remove_ui_later();
                }
                return Err(error.to_string());
            }
        };
        Ok(Self {
            primary: true,
            notifications: NotificationSender {
                sender,
                wake_pending: pending,
                thread_id: 1,
                enabled: installed,
                wake: Some(wake.clone()),
            },
            wake: Some(wake),
            thread: Some(thread),
            _instances: instances,
            ui_installed: installed,
        })
    }
}
impl Drop for MacShell {
    fn drop(&mut self) {
        if let Some(wake) = &self.wake {
            wake.stop();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // Never synchronously wait for AppKit here: Iced may already be closing
        // its run loop while a worker drops the Service.
        if self.ui_installed {
            remove_ui_later();
        }
    }
}

fn event_loop(
    wake: Arc<Wake>,
    receiver: mpsc::Receiver<(String, String)>,
    pending: Arc<AtomicBool>,
    callback: Callback,
) {
    loop {
        if wake.stopped.load(Ordering::Acquire) {
            break;
        }
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        let count = unsafe {
            libc::kevent(
                wake.queue.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                std::ptr::null(),
            )
        };
        if count < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if event.filter == libc::EVFILT_USER && event.ident == STOP {
            break;
        }
        if event.filter == libc::EVFILT_VNODE {
            callback(ShellEvent::Show);
        }
        if event.filter == libc::EVFILT_USER && event.ident == NOTIFY {
            pending.store(false, Ordering::Release);
            for (title, body) in receiver.try_iter().take(8) {
                let _ = notify(&title, &body);
            }
        }
    }
}

/// Runs directly when already on the main thread. Otherwise only call after
/// Iced's event loop starts (Service initialization is a blocking worker).
fn on_main<R: Send>(work: impl FnOnce(MainThreadMarker) -> R + Send) -> R {
    if let Some(mtm) = MainThreadMarker::new() {
        return work(mtm);
    }
    let mut result = None;
    DispatchQueue::main().exec_sync(|| {
        result = Some(work(MainThreadMarker::new().expect("AppKit main queue")));
    });
    result.expect("main queue completed")
}

struct MainUi {
    _tray: TrayIcon,
    menu: Menu,
    activation: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    hidden: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    callback: Callback,
}
impl Drop for MainUi {
    fn drop(&mut self) {
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(&self.activation);
            NSNotificationCenter::defaultCenter().removeObserver(&self.hidden);
        }
        self.menu.remove_for_nsapp();
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    }
}
thread_local! { static UI: RefCell<Option<MainUi>> = const { RefCell::new(None) }; }

fn install_ui(mtm: MainThreadMarker, callback: Callback) -> Result<(), String> {
    install_delegate_hooks(mtm)?;
    ALLOW_TERMINATION.store(false, Ordering::Release);
    let menu = Menu::new();
    let app = Submenu::new("流光", true);
    let settings = MenuItem::with_id(
        "flow-settings",
        "设置…",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::Comma)),
    );
    let quit = MenuItem::with_id(
        "flow-exit",
        "退出流光",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyQ)),
    );
    app.append_items(&[
        &settings,
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(Some("隐藏流光")),
        &PredefinedMenuItem::hide_others(Some("隐藏其他")),
        &PredefinedMenuItem::show_all(Some("全部显示")),
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .map_err(|e| e.to_string())?;
    let file = Submenu::new("文件", true);
    let add = MenuItem::with_id(
        "flow-new",
        "添加下载…",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyN)),
    );
    file.append_items(&[
        &add,
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::close_window(Some("关闭窗口")),
    ])
    .map_err(|e| e.to_string())?;
    // Iced's text inputs are not NSTextViews. Route these menu actions through
    // the focused window's existing keyboard handling, without global input.
    let edit_item = |id, text, key, shift| {
        MenuItem::with_id(
            id,
            text,
            true,
            Some(Accelerator::new(
                Some(if shift {
                    Modifiers::SUPER | Modifiers::SHIFT
                } else {
                    Modifiers::SUPER
                }),
                key,
            )),
        )
    };
    let edit = Submenu::with_items(
        "编辑",
        true,
        &[
            &edit_item("flow-undo", "撤销", Code::KeyZ, false),
            &edit_item("flow-redo", "重做", Code::KeyZ, true),
            &PredefinedMenuItem::separator(),
            &edit_item("flow-cut", "剪切", Code::KeyX, false),
            &edit_item("flow-copy", "复制", Code::KeyC, false),
            &edit_item("flow-paste", "粘贴", Code::KeyV, false),
            &edit_item("flow-select-all", "全选", Code::KeyA, false),
        ],
    )
    .map_err(|e| e.to_string())?;
    let window = Submenu::with_items(
        "窗口",
        true,
        &[
            &PredefinedMenuItem::minimize(Some("最小化")),
            &PredefinedMenuItem::maximize(Some("缩放")),
            &MenuItem::with_id("flow-show", "显示流光", true, None),
        ],
    )
    .map_err(|e| e.to_string())?;
    menu.append_items(&[&app, &file, &edit, &window])
        .map_err(|e| e.to_string())?;
    let tray_menu = Menu::with_items(&[
        &MenuItem::with_id("flow-show", "显示流光", true, None),
        &MenuItem::with_id("flow-new", "添加下载…", true, None),
        &MenuItem::with_id("flow-settings", "设置…", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("flow-exit", "退出流光", true, None),
    ])
    .map_err(|e| e.to_string())?;
    let (rgba, width, height) = icon_rgba()?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("流光")
        .with_menu(Box::new(tray_menu))
        .with_icon(Icon::from_rgba(rgba, width, height).map_err(|e| e.to_string())?)
        .build()
        .map_err(|e| e.to_string())?;
    let on_menu = callback.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(edit) = EditCommand::from_id(event.id.as_ref()) {
            perform_edit(edit);
            return;
        }
        let action = match event.id.as_ref() {
            "flow-show" => ShellEvent::Show,
            "flow-exit" => ShellEvent::Exit,
            "flow-new" => ShellEvent::NewDownload,
            "flow-settings" => ShellEvent::Settings,
            _ => return,
        };
        on_menu(action);
    }));
    let application = NSApplication::sharedApplication(mtm);
    let on_activation = callback.clone();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| on_activation(ShellEvent::Show));
    let activation = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidBecomeActiveNotification),
            Some(&application),
            None,
            &block,
        )
    };
    let on_hide = callback.clone();
    let hide_block = RcBlock::new(move |_: NonNull<NSNotification>| on_hide(ShellEvent::Hidden));
    let hidden = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationWillHideNotification),
            Some(&application),
            None,
            &hide_block,
        )
    };
    menu.init_for_nsapp();
    window.set_as_windows_menu_for_nsapp();
    UI.with(|slot| {
        *slot.borrow_mut() = Some(MainUi {
            _tray: tray,
            menu,
            activation,
            hidden,
            callback,
        })
    });
    notifications_active().store(true, Ordering::Release);
    Ok(())
}
fn remove_ui_later() {
    notifications_active().store(false, Ordering::Release);
    notification_queue()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    if MainThreadMarker::new().is_some() {
        UI.with(|slot| {
            slot.borrow_mut().take();
        });
    } else {
        DispatchQueue::main().exec_async(|| {
            UI.with(|slot| {
                slot.borrow_mut().take();
            })
        });
    }
}
pub(super) fn hide_application() {
    if crate::lifecycle::isolation_active() {
        return;
    }
    on_main(|mtm| NSApplication::sharedApplication(mtm).hide(None));
}
pub(super) fn activate_application() {
    if crate::lifecycle::isolation_active() {
        return;
    }
    on_main(|mtm| {
        let app = NSApplication::sharedApplication(mtm);
        app.unhide(None);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    });
}

static ALLOW_TERMINATION: AtomicBool = AtomicBool::new(false);
pub(super) fn prepare_exit() {
    ALLOW_TERMINATION.store(true, Ordering::Release);
}

fn shell_callback() -> Option<Callback> {
    UI.with(|slot| slot.borrow().as_ref().map(|ui| ui.callback.clone()))
}
extern "C-unwind" fn should_terminate(
    _: &AnyObject,
    _: Sel,
    _: &NSApplication,
) -> NSApplicationTerminateReply {
    if !ALLOW_TERMINATION.load(Ordering::Acquire) {
        if let Some(callback) = shell_callback() {
            callback(ShellEvent::Exit);
            return NSApplicationTerminateReply::TerminateCancel;
        }
    }
    NSApplicationTerminateReply::TerminateNow
}
extern "C-unwind" fn should_reopen(_: &AnyObject, _: Sel, _: &NSApplication, _: Bool) -> Bool {
    if let Some(callback) = shell_callback() {
        callback(ShellEvent::Show);
        return Bool::NO;
    }
    Bool::YES
}
fn install_delegate_hooks(mtm: MainThreadMarker) -> Result<(), String> {
    let application = NSApplication::sharedApplication(mtm);
    let delegate = application
        .delegate()
        .ok_or("尚未建立 macOS 窗口事件循环")?;
    let class = delegate.class();
    // winit 0.30 owns this object and checks its class before handling events.
    // Add only absent optional selectors, retaining the object, class hierarchy,
    // ivars and all existing methods. Never replace or swizzle a supplied method.
    if class.name().to_bytes() != b"WinitApplicationDelegate" {
        return Err("macOS 窗口委托版本不匹配，拒绝覆盖其他事件处理器".into());
    }
    let terminate: extern "C-unwind" fn(
        &AnyObject,
        Sel,
        &NSApplication,
    ) -> NSApplicationTerminateReply = should_terminate;
    let reopen: extern "C-unwind" fn(&AnyObject, Sel, &NSApplication, Bool) -> Bool = should_reopen;
    let methods = [
        (
            objc2::sel!(applicationShouldTerminate:),
            unsafe { std::mem::transmute::<_, Imp>(terminate) },
            format!("{}@:@", NSApplicationTerminateReply::ENCODING),
        ),
        (
            objc2::sel!(applicationShouldHandleReopen:hasVisibleWindows:),
            unsafe { std::mem::transmute::<_, Imp>(reopen) },
            format!("{}@:@{}", Bool::ENCODING, Bool::ENCODING),
        ),
    ];
    for (selector, implementation, encoding) in methods {
        if let Some(existing) = class.instance_method(selector) {
            if existing.implementation() as usize == implementation as usize {
                continue;
            }
            return Err(format!(
                "macOS 窗口委托已实现 {selector}，需要复核退出兼容性"
            ));
        }
        let encoding = std::ffi::CString::new(encoding).map_err(|e| e.to_string())?;
        // SAFETY: signatures match NSApplicationDelegate exactly, including
        // target-specific Objective-C BOOL/NSUInteger encodings. Main-thread
        // installation adds no instance fields and never overwrites a method.
        if !unsafe {
            objc2::ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                selector,
                implementation,
                encoding.as_ptr(),
            )
        }
        .as_bool()
        {
            return Err(format!("无法注册 macOS 窗口委托方法 {selector}"));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum EditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}
impl EditCommand {
    fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "flow-undo" => Self::Undo,
            "flow-redo" => Self::Redo,
            "flow-cut" => Self::Cut,
            "flow-copy" => Self::Copy,
            "flow-paste" => Self::Paste,
            "flow-select-all" => Self::SelectAll,
            _ => return None,
        })
    }
}
fn perform_edit(command: EditCommand) {
    on_main(move |mtm| {
        let app = NSApplication::sharedApplication(mtm);
        let Some(window) = app.keyWindow() else {
            return;
        };
        let Some(responder) = window.firstResponder() else {
            return;
        };
        let (key, code) = match command {
            EditCommand::Undo | EditCommand::Redo => ("z", 6),
            EditCommand::Cut => ("x", 7),
            EditCommand::Copy => ("c", 8),
            EditCommand::Paste => ("v", 9),
            EditCommand::SelectAll => ("a", 0),
        };
        let mut flags = NSEventModifierFlags::Command;
        if matches!(command, EditCommand::Redo) {
            flags |= NSEventModifierFlags::Shift;
        }
        let keys = NSString::from_str(key);
        for event_type in [NSEventType::KeyDown, NSEventType::KeyUp] {
            if let Some(event) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                event_type, NSPoint::new(0., 0.), flags, 0., window.windowNumber(), None, &keys, &keys, false, code) {
                if event_type == NSEventType::KeyDown { responder.keyDown(&event); }
                else { responder.keyUp(&event); }
            }
        }
    });
}

pub(super) fn format_timestamp(millis: u64) -> String {
    let Ok(seconds) = libc::time_t::try_from(millis / 1000) else {
        return "—".into();
    };
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&seconds, &mut parts) }.is_null() {
        return "—".into();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        parts.tm_year + 1900,
        parts.tm_mon + 1,
        parts.tm_mday,
        parts.tm_hour,
        parts.tm_min
    )
}
fn work_areas(mtm: MainThreadMarker) -> Vec<WorkArea> {
    let screens = NSScreen::screens(mtm);
    let origin_top = screens
        .firstObject()
        .map(|s| {
            let f = s.frame();
            f.origin.y + f.size.height
        })
        .unwrap_or(0.);
    screens
        .iter()
        .map(|screen| {
            let frame = screen.visibleFrame();
            WorkArea {
                x: frame.origin.x as f32,
                y: (origin_top - frame.origin.y - frame.size.height) as f32,
                width: frame.size.width as f32,
                height: frame.size.height as f32,
                scale: 1.,
            }
        })
        .collect()
}
pub(super) fn visible_window_state(mut state: WindowState) -> WindowState {
    // AppKit window/screen coordinates are points, independent of Retina scale.
    state.position_scale = 1.;
    if crate::lifecycle::isolation_active() {
        return state.normalized();
    }
    on_main(move |mtm| fit_to_work_areas(state, &work_areas(mtm)))
}
pub(super) fn capture_window_state(fallback: &WindowState) -> WindowState {
    let fallback = fallback.clone();
    if crate::lifecycle::isolation_active() {
        return fallback.normalized();
    }
    on_main(move |mtm| {
        let app = NSApplication::sharedApplication(mtm);
        let title = NSString::from_str("流光");
        let windows = app.windows();
        let Some(window) = windows
            .iter()
            .find(|window| window.title().isEqualToString(&title))
        else {
            return fit_to_work_areas(fallback, &work_areas(mtm));
        };
        let zoomed = window.isZoomed();
        if zoomed || window.isMiniaturized() {
            let mut state = fallback;
            state.maximized = zoomed || state.maximized;
            return fit_to_work_areas(state, &work_areas(mtm));
        }
        let frame = window.frame();
        let content = window.contentRectForFrameRect(frame);
        let screens = NSScreen::screens(mtm);
        let origin_top = screens
            .firstObject()
            .map(|s| {
                let f = s.frame();
                f.origin.y + f.size.height
            })
            .unwrap_or(0.);
        fit_to_work_areas(
            WindowState {
                width: content.size.width as f32,
                height: content.size.height as f32,
                x: Some(frame.origin.x as f32),
                y: Some((origin_top - frame.origin.y - frame.size.height) as f32),
                position_scale: 1.,
                maximized: false,
            },
            &work_areas(mtm),
        )
    })
}

fn notifications_active() -> &'static AtomicBool {
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    &ACTIVE
}
fn notification_queue() -> &'static std::sync::Mutex<std::collections::VecDeque<(String, String)>> {
    static QUEUE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::VecDeque<(String, String)>>,
    > = std::sync::OnceLock::new();
    QUEUE.get_or_init(|| std::sync::Mutex::new(std::collections::VecDeque::new()))
}
fn notification_permission() -> &'static std::sync::atomic::AtomicU8 {
    // 0 unknown, 1 one request outstanding, 2 granted, 3 denied/error.
    static STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    &STATE
}
fn deliver_notifications() {
    use objc2_user_notifications::{
        UNMutableNotificationContent, UNNotificationRequest, UNNotificationSound,
        UNUserNotificationCenter,
    };
    if !notifications_active().load(Ordering::Acquire) {
        return;
    }
    let pending = notification_queue()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .collect::<Vec<_>>();
    objc2::rc::autoreleasepool(|_| {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        for (title, body) in pending {
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&title));
            content.setBody(&NSString::from_str(&body));
            content.setSound(Some(&UNNotificationSound::defaultSound()));
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&format!("flow-{}", uuid::Uuid::new_v4())),
                &content,
                None,
            );
            center.addNotificationRequest_withCompletionHandler(&request, None);
        }
    });
}
pub(super) fn notify(title: &str, body: &str) -> Result<(), String> {
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError};
    use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};
    if !notifications_active().load(Ordering::Acquire) || crate::lifecycle::isolation_active() {
        return Err("系统通知未启用".into());
    }
    // Apple requires a real application bundle identity. Avoid an Objective-C
    // exception when developers invoke the raw binary outside 流光.app.
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return Err("系统通知需要从流光.app 启动".into());
    }
    if notification_permission().load(Ordering::Acquire) == 3 {
        return Err("系统通知未获授权".into());
    }
    {
        let mut queue = notification_queue()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if queue.len() >= 8 {
            queue.pop_front();
        }
        queue.push_back((
            title.chars().take(256).collect(),
            body.chars().take(1024).collect(),
        ));
    }
    match notification_permission().compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => DispatchQueue::main().exec_async(|| {
            let completion = RcBlock::new(|granted: Bool, error: *mut NSError| {
                let granted = granted.as_bool() && error.is_null();
                notification_permission().store(if granted { 2 } else { 3 }, Ordering::Release);
                if granted {
                    deliver_notifications();
                } else {
                    notification_queue()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clear();
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .requestAuthorizationWithOptions_completionHandler(
                    UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                    &completion,
                );
        }),
        Err(2) => deliver_notifications(),
        _ => {}
    }
    Ok(())
}
