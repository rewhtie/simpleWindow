use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State};
use windows::{
    core::{w, BOOL},
    Win32::{
        Foundation::{HWND, LPARAM},
        System::SystemInformation::GetTickCount64,
        UI::{
            Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
            WindowsAndMessaging::{
                EnumWindows, FindWindowExW, FindWindowW, ShowWindow, SW_HIDE, SW_SHOW,
            },
        },
    },
};

struct DesktopState {
    desktop_visible: AtomicBool,
    menu_open: AtomicBool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiState {
    desktop_visible: bool,
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
            let target = lparam.0 as *mut HWND;
            *target = view;
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

fn emit_state(app: &AppHandle, state: &DesktopState) {
    let _ = app.emit(
        "desktop-state-changed",
        UiState {
            desktop_visible: state.desktop_visible.load(Ordering::SeqCst),
        },
    );
}

#[tauri::command]
fn hide_desktop(app: AppHandle, state: State<'_, Arc<DesktopState>>) {
    set_desktop_visible(&state, false);
    emit_state(&app, &state);
}

#[tauri::command]
fn show_desktop_temporarily(app: AppHandle, state: State<'_, Arc<DesktopState>>) {
    set_desktop_visible(&state, true);
    emit_state(&app, &state);
}

#[tauri::command]
fn set_menu_open(open: bool, state: State<'_, Arc<DesktopState>>) {
    state.menu_open.store(open, Ordering::SeqCst);
}

#[tauri::command]
fn get_desktop_state(state: State<'_, Arc<DesktopState>>) -> UiState {
    UiState {
        desktop_visible: state.desktop_visible.load(Ordering::SeqCst),
    }
}

#[tauri::command]
fn restart_app(app: AppHandle, state: State<'_, Arc<DesktopState>>) {
    set_desktop_visible(&state, true);
    app.restart();
}

#[tauri::command]
fn quit_app(app: AppHandle, state: State<'_, Arc<DesktopState>>) {
    set_desktop_visible(&state, true);
    app.exit(0);
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

fn start_idle_watcher(app: AppHandle, state: Arc<DesktopState>) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(250));
        if state.desktop_visible.load(Ordering::SeqCst)
            && !state.menu_open.load(Ordering::SeqCst)
            && idle_milliseconds() >= 10_000
        {
            set_desktop_visible(&state, false);
            emit_state(&app, &state);
        }
    });
}

fn place_orb(app: &AppHandle) {
    let Some(window) = app.get_webview_window("orb") else {
        return;
    };
    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let scale = monitor.scale_factor();
    let orb_size = (76.0 * scale).round() as i32;
    let margin = (18.0 * scale).round() as i32;
    let x = monitor_position.x + monitor_size.width as i32 - orb_size - margin;
    let y = monitor_position.y + (monitor_size.height as i32 - orb_size) / 2;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

pub fn run() {
    let state = Arc::new(DesktopState {
        desktop_visible: AtomicBool::new(false),
        menu_open: AtomicBool::new(false),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            hide_desktop,
            show_desktop_temporarily,
            set_menu_open,
            get_desktop_state,
            restart_app,
            quit_app
        ])
        .setup(move |app| {
            set_desktop_visible(&state, false);
            place_orb(app.handle());
            start_idle_watcher(app.handle().clone(), state.clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let state = window.state::<Arc<DesktopState>>();
                set_desktop_visible(&state, true);
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run Window 收纳");
}
