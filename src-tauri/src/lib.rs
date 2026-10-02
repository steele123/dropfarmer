mod browser_login;
mod campaign_cache;
mod engine;
mod model;
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
    Ok(e.snapshot.lock().await.clone())
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
    e.claim(&campaign_id, &drop_id).await
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
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            let engine = Engine::new(
                app.handle().clone(),
                app.path().app_data_dir()?.join("preferences.json"),
            );
            app.manage(engine.clone());
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
            open_twitch
        ])
        .run(tauri::generate_context!())
        .expect("Could not start Dropfarmer");
}
