mod browser_login;
mod campaign_cache;
mod desktop;
mod engine;
mod model;
mod notifications;
mod queue_status;
mod sleep;
mod twitch;
use engine::Engine;
use std::sync::Arc;
use tauri::{Manager, State};
type Farmer<'a> = State<'a, Arc<Engine>>;
#[tauri::command]
async fn initialize(e: Farmer<'_>) -> Result<model::Snapshot, String> {
    e.initialize().await
}
#[tauri::command]
async fn get_state(e: Farmer<'_>) -> Result<model::Snapshot, String> {
    Ok(e.state().await)
}
#[tauri::command]
async fn begin_login(e: Farmer<'_>) -> Result<twitch::LoginCode, String> {
    e.begin_login().await
}
#[tauri::command]
async fn begin_browser_login(e: Farmer<'_>) -> Result<(), String> {
    e.begin_browser_login().await
}
#[tauri::command]
async fn cancel_login(e: Farmer<'_>) -> Result<(), String> {
    e.cancel_login().await;
    Ok(())
}
#[tauri::command]
async fn poll_login(e: Farmer<'_>) -> Result<bool, String> {
    e.poll_login().await
}
#[tauri::command]
async fn logout(e: Farmer<'_>) -> Result<(), String> {
    e.logout().await
}
#[tauri::command]
async fn refresh(e: Farmer<'_>) -> Result<(), String> {
    e.refresh().await
}
#[tauri::command]
async fn set_queue(e: Farmer<'_>, ids: Vec<String>) -> Result<(), String> {
    e.set_queue(ids).await
}
#[tauri::command]
async fn set_running(e: Farmer<'_>, value: bool) -> Result<(), String> {
    e.set_running(value).await
}
#[tauri::command]
async fn set_auto_claim(e: Farmer<'_>, value: bool) -> Result<(), String> {
    e.set_auto_claim(value).await
}
#[tauri::command]
async fn claim_drop(e: Farmer<'_>, campaign_id: String, drop_id: String) -> Result<(), String> {
    let result = e.claim(&campaign_id, &drop_id).await;
    if let Err(error) = &result {
        e.report_error(error).await;
        e.emit().await;
    }
    result
}
#[tauri::command]
async fn set_desktop_settings(
    e: Farmer<'_>,
    tray_enabled: bool,
    notifications: model::NotificationSettings,
) -> Result<(), String> {
    e.set_desktop_settings(tray_enabled, notifications).await
}
#[tauri::command]
async fn set_sleep_after_queue(e: Farmer<'_>, value: bool) -> Result<(), String> {
    e.set_sleep_after_queue(value).await
}
#[tauri::command]
async fn test_notification(e: Farmer<'_>) -> Result<(), String> {
    e.test_notification().await
}
#[tauri::command]
fn minimize_window(app: tauri::AppHandle, e: Farmer<'_>) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Window unavailable.")?;
    if e.tray_enabled.load(std::sync::atomic::Ordering::Relaxed) {
        window
            .hide()
            .map_err(|_| "Could not minimize the window.".to_string())?;
        desktop::notify_hidden(&app);
        Ok(())
    } else {
        window
            .minimize()
            .map_err(|_| "Could not minimize the window.".to_string())
    }
}
#[tauri::command]
fn open_twitch(url: String) -> Result<(), String> {
    if !twitch::valid_twitch_link(&url) {
        return Err("Only Twitch links can be opened.".into());
    }
    open::that(url).map_err(|_| "Could not open your browser.".into())
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            desktop::show(app);
        }))
        .on_window_event(desktop::window_event)
        .setup(|app| {
            let engine = Engine::new(
                app.handle().clone(),
                app.path().app_data_dir()?.join("preferences.json"),
            );
            app.manage(engine.clone());
            desktop::setup(app)?;
            tauri::async_runtime::spawn(engine.clone().sleep_worker());
            tauri::async_runtime::spawn(engine.worker());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            initialize,
            get_state,
            begin_login,
            begin_browser_login,
            cancel_login,
            poll_login,
            logout,
            refresh,
            set_queue,
            set_running,
            set_auto_claim,
            claim_drop,
            set_desktop_settings,
            test_notification,
            set_sleep_after_queue,
            minimize_window,
            open_twitch
        ])
        .run(tauri::generate_context!())
        .expect("Could not start Dropfarmer");
}
