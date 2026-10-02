use crate::{engine::Engine, model::Snapshot};
use std::sync::{atomic::Ordering, Arc};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

pub struct TrayControls {
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
                    let running = engine.snapshot.lock().await.running;
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
                    let _ = engine.set_running(false).await;
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
        } else {
            "Stopped".into()
        };
        let _ = controls
            .cancel_sleep
            .set_enabled(s.sleep_after_queue.enabled);
        let _ = controls.status.set_text(format!("Dropfarmer · {label}"));
        let _ = controls.toggle.set_text(if s.running {
            "Pause farming"
        } else {
            "Resume farming"
        });
        let _ = controls.toggle.set_enabled(
            s.running || (s.account.is_some() && !s.queue.is_empty() && !s.needs_reconnect),
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
            // Prevent closing only after hiding succeeds, so a tray failure cannot trap the app.
            if window.hide().is_ok() {
                api.prevent_close();
            }
        }
        tauri::WindowEvent::Resized(_) if window.is_minimized().unwrap_or(false) => {
            let _ = window.hide();
        }
        _ => {}
    }
}
