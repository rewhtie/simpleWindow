use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use tauri::{
    menu::{CheckMenuItemBuilder, MenuBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tauri_plugin_autostart::ManagerExt;
use windows::{
    core::{w, BOOL},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::SystemInformation::GetTickCount64,
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO},
            WindowsAndMessaging::{
                CallNextHookEx, DispatchMessageW, EnumWindows, FindWindowExW, FindWindowW,
                GetMessageW, SetWindowsHookExW, ShowWindow, TranslateMessage, MSG, SW_HIDE,
                SW_SHOW, WH_MOUSE_LL, WM_MOUSEHWHEEL, WM_MOUSEWHEEL,
            },
        },
    },
};


static WHEEL_EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && (wparam.0 as u32 == WM_MOUSEWHEEL || wparam.0 as u32 == WM_MOUSEHWHEEL) {
        WHEEL_EVENT_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    }
    CallNextHookEx(None, code, wparam, lparam)
}

struct DesktopState {
    desktop_visible: AtomicBool,
    shutting_down: AtomicBool,
}


unsafe fn set_window_visible(window: HWND, visible: bool) {
    if !window.is_invalid() {
        let _ = ShowWindow(window, if visible { SW_SHOW } else { SW_HIDE });
    }
}

unsafe extern "system" fn set_secondary_taskbars(window: HWND, lparam: LPARAM) -> BOOL {
    let mut name = [0u16; 64];
    let length = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(window, &mut name);
    if length > 0 && String::from_utf16_lossy(&name[..length as usize]) == "Shell_SecondaryTrayWnd" {
        set_window_visible(window, lparam.0 != 0);
    }
    BOOL(1)
}

unsafe fn set_taskbars_visible(visible: bool) {
    if let Ok(taskbar) = FindWindowW(w!("Shell_TrayWnd"), None) {
        set_window_visible(taskbar, visible);
    }
    let _ = EnumWindows(
        Some(set_secondary_taskbars),
        LPARAM(if visible { 1 } else { 0 }),
    );
}

unsafe extern "system" fn find_desktop_view(window: HWND, lparam: LPARAM) -> BOOL {
    if let Ok(view) = FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), None) {
        if !view.is_invalid() {
            *(lparam.0 as *mut HWND) = view;
            return BOOL(0);
        }
    }
    BOOL(1)
}

unsafe fn desktop_view() -> HWND {
    if let Ok(program_manager) = FindWindowW(w!("Progman"), None) {
        if let Ok(view) = FindWindowExW(
            Some(program_manager),
            None,
            w!("SHELLDLL_DefView"),
            None,
        ) {
            if !view.is_invalid() {
                return view;
            }
        }
    }

    let mut result = HWND::default();
    let _ = EnumWindows(
        Some(find_desktop_view),
        LPARAM((&mut result as *mut HWND) as isize),
    );
    result
}

fn set_desktop_visible(state: &DesktopState, visible: bool) {
    unsafe {
        set_taskbars_visible(visible);
        set_window_visible(desktop_view(), visible);
    }
    state.desktop_visible.store(visible, Ordering::SeqCst);
}

fn toggle_desktop(state: &DesktopState) {
    let visible = state.desktop_visible.load(Ordering::SeqCst);
    set_desktop_visible(state, !visible);
}

fn restore_before_exit(state: &DesktopState) {
    state.shutting_down.store(true, Ordering::SeqCst);
    set_desktop_visible(state, true);
}

fn idle_milliseconds() -> u64 {
    unsafe {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut info).as_bool() {
            GetTickCount64().saturating_sub(info.dwTime as u64)
        } else {
            0
        }
    }
}

fn start_input_watcher(state: Arc<DesktopState>) {
    thread::spawn(|| unsafe {
        let Ok(_hook) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) else {
            return;
        };
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    });

    thread::spawn(move || {
        let mut key_down = [false; 256];
        let mut wheel_sequence = WHEEL_EVENT_SEQUENCE.load(Ordering::SeqCst);
        let mut hidden_input_armed = false;

        loop {
            thread::sleep(Duration::from_millis(50));
            if state.shutting_down.load(Ordering::SeqCst) {
                break;
            }

            let visible = state.desktop_visible.load(Ordering::SeqCst);
            if visible {
                hidden_input_armed = false;
                if idle_milliseconds() >= 10_000 {
                    set_desktop_visible(&state, false);
                }
                continue;
            }

            let current_wheel_sequence = WHEEL_EVENT_SEQUENCE.load(Ordering::SeqCst);
            let wheel_changed = current_wheel_sequence != wheel_sequence;
            wheel_sequence = current_wheel_sequence;

            let mut pressed = false;
            for virtual_key in 1..=255 {
                let is_down = unsafe { GetAsyncKeyState(virtual_key) } < 0;
                if hidden_input_armed && is_down && !key_down[virtual_key as usize] {
                    pressed = true;
                }
                key_down[virtual_key as usize] = is_down;
            }

            if !hidden_input_armed {
                hidden_input_armed = true;
                continue;
            }
            if pressed || wheel_changed {
                set_desktop_visible(&state, true);
            }
        }
    });
}

pub fn run() {
    let state = Arc::new(DesktopState {
        desktop_visible: AtomicBool::new(false),
        shutting_down: AtomicBool::new(false),
    });
    let setup_state = state.clone();
    let run_state = state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(state)
        .setup(move |app| {
            set_desktop_visible(&setup_state, false);
            start_input_watcher(setup_state.clone());

            let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
            let autostart = CheckMenuItemBuilder::with_id("autostart", "开机自启动")
                .checked(autostart_enabled)
                .build(app)?;
            let menu = MenuBuilder::new(app)
                .text("hide", "立即隐藏")
                .text("show", "临时显示")
                .separator()
                .item(&autostart)
                .text("restart", "重启应用")
                .separator()
                .text("quit", "退出应用")
                .build()?;

            let menu_state = setup_state.clone();
            let autostart_item = autostart.clone();
            let tray_state = setup_state.clone();
            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;

            TrayIconBuilder::with_id("main")
                .icon(icon)
                .tooltip("Window 收纳")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(move |_tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Down,
                        ..
                    } = event
                    {
                        toggle_desktop(&tray_state);
                    }
                })
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "hide" => set_desktop_visible(&menu_state, false),
                    "show" => set_desktop_visible(&menu_state, true),
                    "autostart" => {
                        let manager = app.autolaunch();
                        let enabled = manager.is_enabled().unwrap_or(false);
                        let result = if enabled { manager.disable() } else { manager.enable() };
                        if result.is_ok() {
                            let _ = autostart_item.set_checked(!enabled);
                        }
                    }
                    "restart" => {
                        restore_before_exit(&menu_state);
                        app.restart();
                    }
                    "quit" => {
                        restore_before_exit(&menu_state);
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Window 收纳")
        .run(move |_app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                restore_before_exit(&run_state);
            }
        });
}
