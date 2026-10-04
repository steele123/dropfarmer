use crate::{engine::Engine, model::Snapshot};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use tauri_plugin_notification::{NotificationExt, PermissionState};

pub struct TrayControls {
    pub show_notice: AtomicBool,
    status: MenuItem<tauri::Wry>,
    toggle: MenuItem<tauri::Wry>,
    cancel_sleep: MenuItem<tauri::Wry>,
}

pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn notify_hidden(app: &AppHandle) {
    if app.notification().permission_state().ok() != Some(PermissionState::Granted) {
        return;
    }
    let _ = app
        .notification()
        .builder()
        .title("Dropfarmer moved to the system tray")
        .body("Still running in the background. Click the Dropfarmer icon near the clock to reopen. You may need to click the ^ arrow first.")
        .show();
}

pub fn hide(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Window unavailable.")?;
    if !window.is_visible().unwrap_or(true) {
        return Ok(());
    }
    window
        .hide()
        .map_err(|_| "Could not hide the window.".to_string())?;
    notify_hidden(app);
    Ok(())
}

pub fn request_hide(app: &AppHandle) -> Result<(), String> {
    if app
        .state::<TrayControls>()
        .show_notice
        .load(Ordering::Relaxed)
    {
        // Restore native minimization so the explanation stays visible.
        show(app);
        app.emit_to("main", "tray-hide-requested", ())
            .map_err(|_| "Could not show the tray notice.".to_string())
    } else {
        hide(app)
    }
}

pub fn setup(app: &mut tauri::App) -> tauri::Result<()> {
    let status = MenuItem::with_id(
        app,
        "tray-status",
        "Dropfarmer · Stopped",
        false,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, "tray-show", "Show Dropfarmer", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "tray-toggle", "Resume farming", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit Dropfarmer", true, None::<&str>)?;
    let cancel_sleep = MenuItem::with_id(
        app,
        "tray-cancel-sleep",
        "Cancel sleep when finished",
        false,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&status, &separator, &open, &toggle, &cancel_sleep, &quit],
    )?;
    let mut tray = TrayIconBuilder::with_id("dropfarmer")
        .tooltip("Dropfarmer · Stopped")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-show" => show(app),
            "tray-toggle" => {
                let engine = app.state::<Arc<Engine>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    let state = engine.snapshot.lock().await;
                    let running = state.running || state.auto_farm.enabled;
                    drop(state);
                    if let Err(error) = engine.set_running(!running).await {
                        engine.log("warning", error).await;
                        engine.emit().await;
                    }
                });
            }
            "tray-cancel-sleep" => {
                let engine = app.state::<Arc<Engine>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = engine.set_sleep_after_queue(false).await;
                });
            }
            "tray-quit" => {
                let app = app.clone();
                let engine = app.state::<Arc<Engine>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    engine.stop_for_exit().await;
                    engine.cancel_login().await;
                    app.exit(0);
                });
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(TrayControls {
        show_notice: AtomicBool::new(false),
        status,
        toggle,
        cancel_sleep,
    });
    Ok(())
}

pub fn update(app: &AppHandle, s: &Snapshot) {
    if let Some(controls) = app.try_state::<TrayControls>() {
        let label = if let Some(seconds) = s.sleep_after_queue.seconds_remaining {
            format!("PC sleeping in {seconds}s")
        } else if s.needs_reconnect {
            "Reconnect Twitch".into()
        } else if s.running {
            "Running".into()
        } else if s.auto_farm.enabled {
            "Watching for new drops".into()
        } else {
            "Stopped".into()
        };
        let _ = controls
            .cancel_sleep
            .set_enabled(s.sleep_after_queue.enabled);
        let _ = controls.status.set_text(format!("Dropfarmer · {label}"));
        let _ = controls
            .toggle
            .set_text(if s.running || s.auto_farm.enabled {
                "Pause farming"
            } else {
                "Resume farming"
            });
        let _ = controls.toggle.set_enabled(
            s.running
                || s.auto_farm.enabled
                || (s.account.is_some() && !s.queue.is_empty() && !s.needs_reconnect),
        );
        if let Some(tray) = app.tray_by_id("dropfarmer") {
            let _ = tray.set_tooltip(Some(format!("Dropfarmer · {label}")));
        }
    }
}

pub fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" {
        return;
    }
    let Some(engine) = window.app_handle().try_state::<Arc<Engine>>() else {
        return;
    };
    if !engine.tray_enabled.load(Ordering::Relaxed) {
        return;
    }
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            // Keep the app open only after showing the notice or successfully hiding it.
            if request_hide(window.app_handle()).is_ok() {
                api.prevent_close();
            }
        }
        tauri::WindowEvent::Resized(_) if window.is_minimized().unwrap_or(false) => {
            let _ = request_hide(window.app_handle());
        }
        _ => {}
    }
}
