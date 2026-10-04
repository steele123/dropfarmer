use crate::browser_login::BrowserLogin;
use crate::campaign_cache::CampaignCache;
use crate::{model::*, twitch::*};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tokio::sync::{Mutex, Notify};

#[derive(Default, Serialize, Deserialize)]
struct Preferences {
    queue: Vec<String>,
    owner: Option<String>,
    #[serde(default = "enabled")]
    auto_claim: bool,
    #[serde(default = "enabled")]
    tray_enabled: bool,
    #[serde(default)]
    notifications: NotificationSettings,
    #[serde(default)]
    auto_farm: crate::auto_farm::AutoFarm,
}
fn enabled() -> bool {
    true
}
pub struct Engine {
    pub snapshot: Mutex<Snapshot>,
    analytics: Mutex<Option<crate::analytics::Ledger>>,
    gate: Mutex<()>,
    session: Mutex<Option<Session>>,
    renewal_retry: Mutex<Option<Instant>>,
    pending: Mutex<Option<PendingSignIn>>,
    login_epoch: AtomicU64,
    pub wake: Notify,
    auto_farm_wake: Notify,
    next_auto_farm_check: Mutex<Option<Instant>>,
    api: Twitch,
    app: AppHandle,
    path: PathBuf,
    last_watch: Mutex<Option<Instant>>,
    initialized: Mutex<bool>,
    pub tray_enabled: AtomicBool,
    reconnect_notified: AtomicBool,
    claimed_notified: Mutex<std::collections::HashSet<String>>,
}
enum PendingSignIn {
    Code(PendingLogin),
    Browser(BrowserLogin),
}
impl Engine {
    pub fn new(app: AppHandle, path: PathBuf) -> Arc<Self> {
        let prefs = Self::read_preferences(&path);
        let mut snapshot = Snapshot::default();
        if let Some(p) = prefs {
            snapshot.auto_claim = p.auto_claim;
            snapshot.tray_enabled = p.tray_enabled;
            snapshot.notifications = p.notifications;
        }
        let tray_enabled = AtomicBool::new(snapshot.tray_enabled);
        Arc::new(Self {
            snapshot: Mutex::new(snapshot),
            analytics: Mutex::new(None),
            tray_enabled,
            reconnect_notified: AtomicBool::new(false),
            claimed_notified: Mutex::new(std::collections::HashSet::new()),
            gate: Mutex::new(()),
            session: Mutex::new(None),
            renewal_retry: Mutex::new(None),
            pending: Mutex::new(None),
            login_epoch: AtomicU64::new(0),
            wake: Notify::new(),
            auto_farm_wake: Notify::new(),
            next_auto_farm_check: Mutex::new(None),
            api: Twitch::new(),
            app,
            path,
            last_watch: Mutex::new(None),
            initialized: Mutex::new(false),
        })
    }
    fn read_preferences(path: &std::path::Path) -> Option<Preferences> {
        std::fs::read(path)
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
    }
    pub async fn state(&self) -> Snapshot {
        // Keep account and farming state stable while updating its analytics ledger.
        let snapshot = self.snapshot.lock().await;
        let mut state = snapshot.clone();
        state.queue_statuses = crate::queue_status::statuses(&state);
        state.sleep_after_queue = state.sleep_plan.status(Instant::now());
        let recent_watch = self
            .last_watch
            .lock()
            .await
            .is_some_and(|last| last.elapsed() <= Duration::from_secs(75));
        let mut analytics = self.analytics.lock().await;
        if let Some(ledger) = analytics.as_mut().filter(|ledger| {
            state
                .account
                .as_ref()
                .is_some_and(|account| ledger.belongs_to(&account.id))
        }) {
            let now = Utc::now().to_rfc3339();
            let active = if state.running
                && !state.needs_reconnect
                && state.error.is_none()
                && state.channel.is_some()
                && recent_watch
            {
                state
                    .campaigns
                    .iter()
                    .find(|c| Some(&c.id) == state.active_campaign.as_ref())
            } else {
                None
            };
            ledger.sample(active, Instant::now(), &now);
            let changed = !state.campaigns_cached && ledger.observe(&state.campaigns, &now);
            let _ = ledger.save(changed || active.is_none());
            state.analytics = ledger.data.clone();
        }
        state
    }
    pub async fn analytics_worker(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if self.snapshot.lock().await.running {
                self.emit().await;
            }
        }
    }
    pub fn flush_analytics(&self) {
        if let Some(ledger) = self.analytics.blocking_lock().as_mut() {
            ledger.sample(None, Instant::now(), &Utc::now().to_rfc3339());
            let _ = ledger.save(true);
        }
    }
    fn credential() -> Result<keyring::Entry, String> {
        keyring::Entry::new("app.dropfarmer.desktop", "twitch")
            .map_err(|_| "Cannot access the operating system credential store.".into())
    }
    pub async fn emit(&self) {
        let s = self.state().await;
        crate::desktop::update(&self.app, &s);
        let _ = self.app.emit("farmer-state", s);
    }
    pub async fn log(&self, level: &str, message: impl Into<String>) {
        let mut s = self.snapshot.lock().await;
        s.logs.insert(
            0,
            Log {
                time: Utc::now().to_rfc3339(),
                level: level.into(),
                message: message.into(),
            },
        );
        s.logs.truncate(100);
    }
    async fn save(&self) -> Result<(), String> {
        let s = self.snapshot.lock().await;
        self.save_snapshot(&s)
    }
    fn save_snapshot(&self, s: &Snapshot) -> Result<(), String> {
        let saved = Self::read_preferences(&self.path);
        let p = Preferences {
            auto_farm: if s.account.is_some() {
                s.auto_farm.clone()
            } else {
                saved
                    .as_ref()
                    .map(|p| p.auto_farm.clone())
                    .unwrap_or_default()
            },
            queue: if s.account.is_some() {
                s.queue.clone()
            } else {
                saved.as_ref().map(|p| p.queue.clone()).unwrap_or_default()
            },
            owner: s
                .account
                .as_ref()
                .map(|a| a.id.clone())
                .or_else(|| saved.and_then(|p| p.owner)),
            auto_claim: s.auto_claim,
            tray_enabled: s.tray_enabled,
            notifications: s.notifications.clone(),
        };
        std::fs::create_dir_all(self.path.parent().ok_or("Invalid preferences path")?)
            .map_err(|_| "Cannot create preferences folder.")?;
        let bytes = serde_json::to_vec_pretty(&p).map_err(|_| "Cannot encode preferences.")?;
        // Same-directory temporary file avoids partially written JSON on interruption.
        let temp = self.path.with_extension("tmp");
        std::fs::write(&temp, bytes).map_err(|_| "Cannot save queue preferences.")?;
        std::fs::rename(temp, &self.path).map_err(|_| "Cannot replace queue preferences.")?;
        Ok(())
    }
    async fn attach(&self, session: Session) {
        *self.analytics.lock().await = Some(crate::analytics::Ledger::load(
            &self.path.with_file_name("analytics"),
            &session.account.id,
        ));
        *self.last_watch.lock().await = None;
        self.reconnect_notified.store(false, Ordering::Relaxed);
        self.claimed_notified.lock().await.clear();
        let method = if session.browser.is_some() {
            "browser"
        } else {
            "code"
        };
        let cached = CampaignCache::read(
            &self.path.with_file_name("campaigns.json"),
            &session.account.id,
            method,
        );
        let prefs: Option<Preferences> = std::fs::read(&self.path)
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok());
        let mut s = self.snapshot.lock().await;
        s.queue.clear();
        s.auto_farm = crate::auto_farm::AutoFarm::default();
        s.auto_farm_last_check = None;
        s.auto_farm_next_check = None;
        s.queue_statuses.clear();
        s.needs_reconnect = false;
        s.running = false;
        s.sleep_plan.cancel();
        s.active_campaign = None;
        s.channel = None;
        s.campaigns.clear();
        s.campaign_notice = None;
        s.campaigns_cached = false;
        s.last_sync = None;
        if let Some(p) = prefs {
            s.auto_claim = p.auto_claim;
            if p.owner.as_deref() == Some(&session.account.id) {
                s.queue = p.queue;
                s.auto_farm = p.auto_farm;
            }
        }
        s.account = Some(session.account.clone());
        s.login_method = Some(method.into());
        s.status = "Connected · refresh to load campaigns".into();
        if let Some(cache) = cached {
            s.campaigns = cache.campaigns;
            s.campaign_notice = cache.notice;
            s.last_sync = Some(cache.synced_at);
            s.campaigns_cached = true;
            s.status = "Cached campaigns loaded".into();
        }
        crate::queue_status::remove_non_watchable(&mut s);
        s.error = None;
        *self.session.lock().await = Some(session);
        *self.renewal_retry.lock().await = None;
        *self.next_auto_farm_check.lock().await = None;
        self.auto_farm_wake.notify_one();
    }
    pub async fn initialize(&self) -> Result<Snapshot, String> {
        let _gate = self.gate.lock().await;
        let mut initialized = self.initialized.lock().await;
        if !*initialized {
            *initialized = true;
            match Self::credential()?.get_password() {
                Ok(raw) => {
                    if let Ok(saved) = serde_json::from_str::<Session>(&raw) {
                        match self.api.validate_context(saved.token, saved.browser).await {
                            Ok(session) => {
                                self.attach(session).await;
                                self.log("info", "Twitch session restored.").await;
                            }
                            Err(e) => {
                                self.log("warning", &e).await;
                                self.report_error(&e).await;
                                self.snapshot.lock().await.error = Some(e);
                            }
                        }
                    }
                }
                Err(keyring::Error::NoEntry) => {}
                Err(_) => {
                    self.log(
                        "warning",
                        "Saved login could not be read from the credential store.",
                    )
                    .await;
                }
            }
        }
        self.emit().await;
        Ok(self.state().await)
    }
    pub async fn begin_login(&self) -> Result<LoginCode, String> {
        self.cancel_login().await;
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        self.set_running(false).await?;
        let _gate = self.gate.lock().await;
        let (pending, code) = self.api.begin_login().await?;
        let mut slot = self.pending.lock().await;
        if epoch != self.login_epoch.load(Ordering::SeqCst) {
            return Err("Sign-in cancelled.".into());
        }
        *slot = Some(PendingSignIn::Code(pending));
        Ok(code)
    }
    pub async fn begin_browser_login(&self) -> Result<(), String> {
        self.cancel_login().await;
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        self.set_running(false).await?;
        let _gate = self.gate.lock().await;
        let root = self
            .app
            .path()
            .app_cache_dir()
            .map_err(|_| "Cannot locate the browser sign-in folder.")?
            .join("browser-login");
        let mut slot = self.pending.lock().await;
        if epoch != self.login_epoch.load(Ordering::SeqCst) {
            return Err("Sign-in cancelled.".into());
        }
        *slot = Some(PendingSignIn::Browser(BrowserLogin::start(&root)?));
        Ok(())
    }
    pub async fn cancel_login(&self) {
        self.login_epoch.fetch_add(1, Ordering::SeqCst);
        *self.pending.lock().await = None;
    }
    pub async fn poll_login(&self) -> Result<bool, String> {
        let _gate = self.gate.lock().await;
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        let mut pending = self.pending.lock().await;
        let p = pending.as_mut().ok_or("Start a Twitch sign-in first.")?;
        let result = match p {
            PendingSignIn::Code(code) => self.api.poll_login(code).await,
            PendingSignIn::Browser(browser) => match browser.poll().await {
                Ok(Some(credentials)) => self.api.accept_browser(credentials).await.map(Some),
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            },
        };
        if epoch != self.login_epoch.load(Ordering::SeqCst) {
            *pending = None;
            return Err("Sign-in cancelled.".into());
        }
        match result {
            Ok(Some(session)) => {
                // Consume the attempt before saving: a completed browser task must not be polled twice.
                *pending = None;
                let data = serde_json::to_string(&session).map_err(|_| "Could not save login.")?;
                Self::credential()?
                    .set_password(&data)
                    .map_err(|_| "Cannot securely save the login in your credential store.")?;
                self.attach(session).await;
                *pending = None;
                self.log("success", "Twitch connected.").await;
                self.emit().await;
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(e) => {
                *pending = None;
                Err(e)
            }
        }
    }
    pub async fn logout(&self) -> Result<(), String> {
        self.cancel_login().await;
        self.set_running(false).await?;
        let _gate = self.gate.lock().await;
        match Self::credential()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(_) => return Err("Could not remove saved login. Please retry.".into()),
        }
        *self.session.lock().await = None;
        *self.pending.lock().await = None;
        let mut s = self.snapshot.lock().await;
        *s = Snapshot {
            auto_claim: s.auto_claim,
            tray_enabled: s.tray_enabled,
            notifications: s.notifications.clone(),
            ..Snapshot::default()
        };
        drop(s);
        self.emit().await;
        Ok(())
    }
    // All callers hold gate, so refresh, claims and farming share one renewal.
    async fn session_for_requests(&self) -> Result<Session, String> {
        let session = self
            .session
            .lock()
            .await
            .clone()
            .ok_or("Connect Twitch first.")?;
        if !session
            .browser
            .as_ref()
            .is_some_and(|context| context.needs_renewal())
        {
            return Ok(session);
        }
        if self
            .renewal_retry
            .lock()
            .await
            .is_some_and(|retry| retry > Instant::now())
        {
            return Err(
                "Browser renewal failed. Retrying in a few minutes; you can also reconnect Twitch."
                    .into(),
            );
        }
        let root = self
            .app
            .path()
            .app_cache_dir()
            .map_err(|_| "Cannot locate the browser sign-in folder.")?
            .join("browser-login");
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        self.log(
            "info",
            "Renewing the Twitch browser session in the background.",
        )
        .await;
        // Do not count time waiting for renewal as farming time.
        *self.last_watch.lock().await = None;
        self.emit().await;
        let cancelled = async {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if epoch != self.login_epoch.load(Ordering::SeqCst) {
                    break;
                }
            }
        };
        let result = tokio::select! {
            biased;
            _ = cancelled => return Err("Browser renewal cancelled.".into()),
            result = self.api.renew_browser(&session, &root) => result,
        };
        if epoch != self.login_epoch.load(Ordering::SeqCst) {
            return Err("Browser renewal cancelled.".into());
        }
        let result = result.and_then(|renewed| {
            let data = serde_json::to_string(&renewed).map_err(|_| "Could not save login.")?;
            Self::credential()?
                .set_password(&data)
                .map_err(|_| "Cannot securely save the login in your credential store.")?;
            Ok(renewed)
        });
        match result {
            Ok(renewed) => {
                *self.session.lock().await = Some(renewed.clone());
                *self.renewal_retry.lock().await = None;
                self.log("success", "Twitch browser session renewed.").await;
                Ok(renewed)
            }
            Err(error) => {
                *self.renewal_retry.lock().await = Some(Instant::now() + Duration::from_secs(300));
                self.report_error(&error).await;
                self.emit().await;
                Err(error)
            }
        }
    }
    pub async fn refresh(&self) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        self.refresh_locked().await
    }
    async fn refresh_locked(&self) -> Result<(), String> {
        let session = self.session_for_requests().await?;
        let mut discovery = match self.api.campaigns(&session).await {
            Ok(discovery) => discovery,
            Err(error) => {
                self.report_error(&error).await;
                self.emit().await;
                return Err(error);
            }
        };
        let count = discovery.campaigns.len();
        if let Some(ledger) = self
            .analytics
            .lock()
            .await
            .as_mut()
            .filter(|l| l.belongs_to(&session.account.id))
        {
            ledger.restore_claims(&mut discovery.campaigns);
            let changed = ledger.observe_inventory(
                &discovery.campaigns,
                &discovery.received,
                &Utc::now().to_rfc3339(),
            );
            let _ = ledger.save(changed);
        }
        let before = self.snapshot.lock().await.campaigns.clone();
        let claimed = crate::notifications::newly_claimed(&before, &discovery.campaigns);
        let queue_changed;
        {
            let mut s = self.snapshot.lock().await;
            s.campaigns = discovery.campaigns;
            s.campaign_notice = discovery.notice.clone();
            s.campaigns_cached = false;
            s.last_sync = Some(Utc::now().to_rfc3339());
            s.error = None;
            s.needs_reconnect = false;
            if !s.running {
                s.status = "Ready".into();
            }
            queue_changed = crate::queue_status::remove_non_watchable(&mut s);
        }
        if queue_changed {
            self.save().await?;
            self.log(
                "info",
                "Removed campaigns without watch-time rewards from the queue.",
            )
            .await;
            self.wake.notify_one();
        }
        let discovered = {
            let mut s = self.snapshot.lock().await;
            let mut next = s.clone();
            let discovered = crate::auto_farm::enqueue(&mut next, Utc::now());
            if next.auto_farm.enabled {
                // Commit queue and handled IDs together before starting the worker.
                self.save_snapshot(&next)?;
                next.auto_farm_last_check = Some(Utc::now().to_rfc3339());
                next.auto_farm_next_check = Some(
                    (Utc::now()
                        + chrono::Duration::seconds(crate::auto_farm::CHECK_SECONDS as i64))
                    .to_rfc3339(),
                );
                *self.next_auto_farm_check.lock().await =
                    Some(Instant::now() + Duration::from_secs(crate::auto_farm::CHECK_SECONDS));
            }
            *s = next;
            discovered
        };
        if !discovered.is_empty() {
            self.log(
                "success",
                format!("Auto farm found new rewards: {}.", discovered.join(", ")),
            )
            .await;
            self.wake.notify_one();
        }
        self.reconnect_notified.store(false, Ordering::Relaxed);
        self.notify_claims(claimed).await;
        self.save_campaign_cache().await;
        self.log("info", format!("Synced {count} campaigns from Twitch."))
            .await;
        if let Some(notice) = discovery.notice {
            self.log("warning", notice).await;
        }
        self.emit().await;
        Ok(())
    }
    pub async fn set_queue(&self, ids: Vec<String>) -> Result<(), String> {
        if ids.len() > 200 {
            return Err("Queue is limited to 200 campaigns.".into());
        }
        {
            let mut s = self.snapshot.lock().await;
            let mut unique = std::collections::HashSet::new();
            if ids.iter().any(|id| {
                !unique.insert(id)
                    || s.campaigns.iter().any(|c| {
                        &c.id == id
                            && !c.drops.is_empty()
                            && !c.complete()
                            && !c.has_watch_rewards()
                    })
                    || (!s.queue.contains(id)
                        && !s.campaigns.iter().any(|c| &c.id == id && c.can_queue()))
            }) {
                return Err(
                    "Only campaigns with unclaimed watch-time rewards that have not ended can be added once."
                        .into(),
                );
            }
            let before = s.queue.clone();
            s.sleep_plan.queue_changed(&before, &ids);
            s.queue = ids;
            let queue = s.queue.clone();
            s.queue_statuses.retain(|id, _| queue.contains(id));
            if s.active_campaign
                .as_ref()
                .is_some_and(|id| !queue.contains(id))
            {
                s.active_campaign = None;
                s.channel = None;
            }
            if s.queue.is_empty() {
                s.running = false;
                s.status = "Queue empty".into();
            }
        }
        self.save().await?;
        self.wake.notify_one();
        self.emit().await;
        Ok(())
    }
    pub async fn set_auto_claim(&self, value: bool) -> Result<(), String> {
        self.snapshot.lock().await.auto_claim = value;
        self.save().await?;
        self.emit().await;
        Ok(())
    }
    pub async fn set_auto_farm(&self, enabled: bool, game_ids: Vec<String>) -> Result<(), String> {
        if game_ids.len() > 100 {
            return Err("Follow up to 100 games.".into());
        }
        let mut s = self.snapshot.lock().await;
        if s.account.is_none() {
            return Err("Connect Twitch first.".into());
        }
        if enabled && s.needs_reconnect {
            return Err("Reconnect Twitch first.".into());
        }
        let mut games = Vec::new();
        for id in game_ids {
            if games
                .iter()
                .any(|g: &crate::auto_farm::FollowedGame| g.id == id)
            {
                continue;
            }
            let name = s
                .auto_farm
                .games
                .iter()
                .find(|g| g.id == id)
                .map(|g| g.name.clone())
                .or_else(|| {
                    s.campaigns
                        .iter()
                        .find(|c| c.game_id == id && !id.is_empty())
                        .map(|c| c.game.clone())
                })
                .ok_or("Choose a game from the loaded campaigns.")?;
            games.push(crate::auto_farm::FollowedGame { id, name });
        }
        if enabled && games.is_empty() {
            return Err("Follow a game first.".into());
        }
        let mut next = s.clone();
        next.auto_farm.enabled = enabled;
        next.auto_farm.games = games;
        next.auto_farm_next_check = None;
        if enabled {
            next.sleep_plan.cancel();
        }
        self.save_snapshot(&next)?;
        *s = next;
        *self.next_auto_farm_check.lock().await = None;
        drop(s);
        self.auto_farm_wake.notify_one();
        self.emit().await;
        Ok(())
    }
    pub async fn auto_farm_worker(self: Arc<Self>) {
        loop {
            tokio::select! {
                _ = self.auto_farm_wake.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(30)) => {},
            }
            // Use the same request gate as manual refresh, claims and farming.
            let _gate = self.gate.lock().await;
            {
                let mut s = self.snapshot.lock().await;
                if !s.auto_farm.enabled
                    || s.auto_farm.games.is_empty()
                    || s.account.is_none()
                    || s.needs_reconnect
                {
                    continue;
                }
                let mut due = self.next_auto_farm_check.lock().await;
                if due.is_some_and(|at| at > Instant::now()) {
                    continue;
                }
                *due = Some(Instant::now() + Duration::from_secs(crate::auto_farm::CHECK_SECONDS));
                s.auto_farm_next_check = Some(
                    (Utc::now()
                        + chrono::Duration::seconds(crate::auto_farm::CHECK_SECONDS as i64))
                    .to_rfc3339(),
                );
            }
            if let Err(error) = self.refresh_locked().await {
                self.log("warning", format!("Auto farm check failed: {error}"))
                    .await;
                self.report_error(&error).await;
                self.emit().await;
            }
        }
    }
    pub async fn set_running(&self, value: bool) -> Result<(), String> {
        let saved;
        {
            let mut s = self.snapshot.lock().await;
            if value && s.needs_reconnect {
                return Err("Reconnect Twitch to continue.".into());
            }
            if value && (s.account.is_none() || s.queue.is_empty()) {
                return Err("Connect Twitch and add a campaign to the queue first.".into());
            }
            s.running = value;
            s.queue_statuses.clear();
            s.error = None;
            s.status = if value {
                "Finding an eligible stream…"
            } else {
                "Queue paused"
            }
            .into();
            if !value {
                // An explicit pause must not be reversed by background discovery.
                s.auto_farm.enabled = false;
                s.auto_farm_next_check = None;
                s.sleep_plan.cancel();
                s.channel = None;
                s.active_campaign = None;
            }
            saved = self.save_snapshot(&s);
        }
        self.wake.notify_one();
        self.emit().await;
        saved
    }
    pub async fn stop_for_exit(&self) {
        let mut s = self.snapshot.lock().await;
        s.running = false;
        s.auto_farm.enabled = false;
        s.sleep_plan.cancel();
        // Quitting preserves the monitoring preference for the next launch.
        drop(s);
        self.wake.notify_one();
    }
    pub async fn claim(&self, campaign_id: &str, drop_id: &str) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        let session = self.session_for_requests().await?;
        let c = self
            .snapshot
            .lock()
            .await
            .campaigns
            .iter()
            .find(|c| c.id == campaign_id)
            .cloned()
            .ok_or("Campaign not found.")?;
        let fresh =
            self.api.update_campaign(&session, &c).await?.ok_or(
                "This campaign is temporarily unavailable on Twitch. Try refreshing shortly.",
            )?;
        let d = fresh
            .drops
            .iter()
            .find(|d| d.id == drop_id && !d.claimed && d.minutes >= d.required && d.watch_reward())
            .ok_or("This reward is not ready to claim yet.")?;
        self.api.claim(&session, d).await?;
        let fresh = self.api.update_campaign(&session, &fresh).await?.ok_or(
            "Claim submitted, but Twitch has not returned the updated campaign. Refresh shortly.",
        )?;
        let fresh = self.replace_campaign(fresh).await;
        let confirmed = fresh.drops.iter().any(|d| d.id == drop_id && d.claimed);
        if !confirmed {
            return Err(
                "Claim submitted. Twitch has not confirmed it yet; refresh shortly.".into(),
            );
        }
        self.log("success", "Reward claim confirmed by Twitch.")
            .await;
        self.emit().await;
        Ok(())
    }
    async fn replace_campaign(&self, mut c: Campaign) -> Campaign {
        let mut s = self.snapshot.lock().await;
        // A single fresh campaign can confirm claims even when the full catalog is still cached.
        if let Some(ledger) = self.analytics.lock().await.as_mut().filter(|ledger| {
            s.account
                .as_ref()
                .is_some_and(|account| ledger.belongs_to(&account.id))
        }) {
            ledger.restore_claims(std::slice::from_mut(&mut c));
            let changed = ledger.observe(std::slice::from_ref(&c), &Utc::now().to_rfc3339());
            let _ = ledger.save(changed);
        }
        let claimed = crate::notifications::newly_claimed(&s.campaigns, std::slice::from_ref(&c));
        if let Some(old) = s.campaigns.iter_mut().find(|old| old.id == c.id) {
            *old = c.clone();
        }
        s.last_sync = Some(Utc::now().to_rfc3339());
        drop(s);
        self.notify_claims(claimed).await;
        self.save_campaign_cache().await;
        c
    }
    async fn save_campaign_cache(&self) {
        let cache = CampaignCache::from_snapshot(&*self.snapshot.lock().await);
        if let Some(cache) = cache {
            if let Err(error) = cache.write(&self.path.with_file_name("campaigns.json")) {
                self.log("warning", error).await;
            }
        }
    }
    pub async fn set_desktop_settings(
        &self,
        tray_enabled: bool,
        notifications: NotificationSettings,
    ) -> Result<(), String> {
        let current = self.snapshot.lock().await.notifications.clone();
        let enabling_notifications = (notifications.rewards && !current.rewards)
            || (notifications.queue && !current.queue)
            || (notifications.reconnect && !current.reconnect);
        if enabling_notifications {
            let permission = self
                .app
                .notification()
                .request_permission()
                .map_err(|_| "Could not request notification permission.")?;
            if permission != PermissionState::Granted {
                return Err("Allow Dropfarmer notifications in your system settings first.".into());
            }
        }
        let previous = {
            let mut s = self.snapshot.lock().await;
            let previous = (s.tray_enabled, s.notifications.clone());
            s.tray_enabled = tray_enabled;
            s.notifications = notifications;
            previous
        };
        if let Err(error) = self.save().await {
            let mut s = self.snapshot.lock().await;
            s.tray_enabled = previous.0;
            s.notifications = previous.1;
            return Err(error);
        }
        self.tray_enabled.store(tray_enabled, Ordering::Relaxed);
        self.emit().await;
        Ok(())
    }
    pub async fn set_sleep_after_queue(&self, value: bool) -> Result<(), String> {
        let mut s = self.snapshot.lock().await;
        if value {
            if s.auto_farm.enabled {
                return Err("Turn off Auto farm before enabling sleep when finished.".into());
            }
            if !cfg!(windows) {
                return Err("Sleep when finished is currently available on Windows.".into());
            }
            if s.account.is_none() || s.needs_reconnect {
                return Err("Connect Twitch first.".into());
            }
            let queue = s.queue.clone();
            s.sleep_plan.arm(&queue)?;
        } else {
            s.sleep_plan.cancel();
        }
        drop(s);
        self.emit().await;
        Ok(())
    }
    pub async fn sleep_worker(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let status = self.snapshot.lock().await.sleep_plan.status(Instant::now());
            let Some(seconds) = status.seconds_remaining else {
                continue;
            };
            if seconds > 0 {
                self.emit().await;
                continue;
            }
            // Save before the final check; cancellation remains available during disk I/O.
            let _gate = self.gate.lock().await;
            let saved = async {
                self.save().await?;
                let cache = CampaignCache::from_snapshot(&*self.snapshot.lock().await);
                if let Some(cache) = cache {
                    cache.write(&self.path.with_file_name("campaigns.json"))?;
                }
                Ok::<_, String>(())
            }
            .await;
            if let Err(error) = saved {
                self.snapshot.lock().await.sleep_plan.cancel();
                let error = format!("Sleep cancelled: {error}");
                self.report_error(&error).await;
                self.log("warning", error).await;
                self.emit().await;
                continue;
            }
            let engine = self.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let mut s = engine.snapshot.blocking_lock();
                if !crate::sleep::take_request(&mut s, Instant::now()) {
                    return Ok(());
                }
                // Consume the run before calling Windows, so resume cannot sleep again.
                drop(s);
                crate::sleep::suspend()
            })
            .await
            .unwrap_or_else(|_| Err("Could not complete the sleep request.".into()));
            if let Err(error) = result {
                self.snapshot.lock().await.sleep_plan.cancel();
                self.report_error(&error).await;
                self.log("warning", error).await;
            }
            self.emit().await;
        }
    }
    pub async fn test_notification(&self) -> Result<(), String> {
        let settings = self.snapshot.lock().await.notifications.clone();
        if !(settings.rewards || settings.queue || settings.reconnect) {
            return Err("Enable a notification option first.".into());
        }
        self.app
            .notification()
            .builder()
            .title("Dropfarmer")
            .body("Desktop notifications are enabled.")
            .show()
            .map_err(|_| {
                "Could not send a notification. Check your system notification settings.".into()
            })
    }
    async fn notify(&self, title: &str, body: &str) {
        if self.app.notification().permission_state().ok() != Some(PermissionState::Granted) {
            return;
        }
        if self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .is_err()
        {
            self.log("warning", "Could not send a desktop notification.")
                .await;
        }
    }
    async fn notify_claims(&self, claimed: Vec<(String, String)>) {
        if !self.snapshot.lock().await.notifications.rewards {
            return;
        }
        let mut seen = self.claimed_notified.lock().await;
        let names: Vec<_> = claimed
            .into_iter()
            .filter_map(|(id, name)| seen.insert(id).then_some(name))
            .collect();
        drop(seen);
        if names.is_empty() {
            return;
        }
        let mut body = names.iter().take(3).cloned().collect::<Vec<_>>().join("\n");
        if names.len() > 3 {
            body.push_str(&format!("\nAnd {} more", names.len() - 3));
        }
        self.notify(
            if names.len() == 1 {
                "Drop claimed"
            } else {
                "Drops claimed"
            },
            &body,
        )
        .await;
    }
    pub async fn report_error(&self, error: &str) {
        let mut s = self.snapshot.lock().await;
        s.error = Some(error.into());
        if s.sleep_plan
            .status(Instant::now())
            .seconds_remaining
            .is_some()
        {
            s.sleep_plan.cancel();
        }
        if !crate::notifications::requires_reconnect(error) {
            return;
        }
        s.needs_reconnect = true;
        s.sleep_plan.cancel();
        s.running = false;
        s.active_campaign = None;
        s.channel = None;
        s.status = "Reconnect Twitch to continue".into();
        let notify =
            s.notifications.reconnect && !self.reconnect_notified.swap(true, Ordering::Relaxed);
        drop(s);
        self.wake.notify_one();
        if notify {
            self.notify(
                "Twitch needs reconnecting",
                "Open Dropfarmer and sign in again to continue farming.",
            )
            .await;
        }
    }
    async fn queue_status(&self, id: &str, status: QueueStatus) {
        self.snapshot
            .lock()
            .await
            .queue_statuses
            .insert(id.into(), status);
    }
    async fn tick(&self) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        let state = self.snapshot.lock().await.clone();
        if !state.running {
            return Ok(());
        }
        let session = self.session_for_requests().await?;
        self.snapshot.lock().await.queue_statuses.clear();
        let mut completed_any = false;
        let mut removed_non_watchable = false;
        let mut unavailable = Vec::new();
        // Refresh queued campaigns using details plus authoritative inventory progress.
        for id in &state.queue {
            let Some(old) = state.campaigns.iter().find(|c| &c.id == id) else {
                self.queue_status(
                    id,
                    QueueStatus::checked(
                        "unavailable",
                        "Campaign not loaded — refresh campaigns",
                        true,
                    ),
                )
                .await;
                unavailable.push(id.clone());
                continue;
            };
            self.queue_status(
                id,
                QueueStatus::checked("checking", "Checking Twitch", false),
            )
            .await;
            self.emit().await;
            let Some(mut c) = self.api.update_campaign(&session, old).await? else {
                self.queue_status(
                    id,
                    QueueStatus::checked(
                        "unavailable",
                        "Twitch did not return this campaign",
                        true,
                    ),
                )
                .await;
                unavailable.push(old.name.clone());
                self.log(
                    "warning",
                    format!(
                        "{} is temporarily unavailable on Twitch. Trying the next queued campaign.",
                        old.name
                    ),
                )
                .await;
                continue;
            };
            if state.auto_claim {
                for d in &c.drops {
                    if !d.claimed
                        && d.watch_reward()
                        && d.minutes >= d.required
                        && d.claim_id.is_some()
                    {
                        self.api.claim(&session, d).await?;
                        self.log(
                            "info",
                            format!("Submitted claim for {} · {}", d.name, c.game),
                        )
                        .await;
                    }
                }
                if c.drops.iter().any(|d| {
                    !d.claimed
                        && d.watch_reward()
                        && d.minutes >= d.required
                        && d.claim_id.is_some()
                }) {
                    let Some(fresh) = self.api.update_campaign(&session, &c).await? else {
                        self.queue_status(
                            id,
                            QueueStatus::checked(
                                "unavailable",
                                "Waiting for Twitch to confirm the claim",
                                true,
                            ),
                        )
                        .await;
                        unavailable.push(c.name.clone());
                        continue;
                    };
                    c = fresh;
                }
            }
            c = self.replace_campaign(c).await;
            if !c.complete() && !c.drops.is_empty() && !c.has_watch_rewards() {
                removed_non_watchable = true;
                crate::queue_status::remove_non_watchable(&mut *self.snapshot.lock().await);
                self.save().await?;
                self.log(
                    "info",
                    format!(
                        "Removed {} from the queue: no watch-time rewards remain.",
                        c.name
                    ),
                )
                .await;
                continue;
            }
            if c.complete() {
                completed_any = true;
                {
                    let mut s = self.snapshot.lock().await;
                    s.queue.retain(|i| i != id);
                    s.sleep_plan.confirmed(id);
                }
                self.save().await?;
                self.log(
                    "success",
                    format!("{} complete. Moving to the next campaign.", c.game),
                )
                .await;
                continue;
            }
            if c.next_drop().is_none() {
                if let Some(status) = crate::queue_status::blocker(&c, Utc::now()) {
                    self.queue_status(id, status).await;
                }
                continue;
            }
            let previous = if state.active_campaign.as_deref() == Some(&c.id) {
                state.channel.as_ref()
            } else {
                None
            };
            if let Some(ch) = self.api.find_channel(&session, &c, previous).await? {
                let wait = self
                    .last_watch
                    .lock()
                    .await
                    .map(|last| Duration::from_secs(60).saturating_sub(last.elapsed()))
                    .unwrap_or_default();
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
                // Reserve the interval before sending; cancellation must not cause a burst.
                *self.last_watch.lock().await = Some(Instant::now());
                self.api.watch(&session, &c, &ch).await?;
                if previous.is_none_or(|p| p.login != ch.login) {
                    self.log("info", format!("Farming {} on {}.", c.game, ch.name))
                        .await;
                }
                let mut s = self.snapshot.lock().await;
                s.queue_statuses.insert(
                    c.id.clone(),
                    QueueStatus::checked("farming", format!("Farming on {}", ch.name), false),
                );
                s.active_campaign = Some(c.id);
                s.channel = Some(ch);
                s.status = "Farming · checking Twitch for progress".into();
                s.error = None;
                return Ok(());
            }
            self.queue_status(
                id,
                QueueStatus::checked("offline", "No eligible channel is live", true),
            )
            .await;
        }
        let mut s = self.snapshot.lock().await;
        s.active_campaign = None;
        s.channel = None;
        s.error = (!unavailable.is_empty()).then(|| {
            format!(
                "Twitch temporarily omitted {}. Your queue is saved; retrying in 60s.",
                unavailable.join(", ")
            )
        });
        s.status = if s.queue.is_empty() {
            s.running = false;
            if removed_non_watchable {
                "Queue empty"
            } else {
                "Queue complete"
            }
        } else if !unavailable.is_empty() {
            "Waiting for Twitch campaign data · retrying in 60s"
        } else {
            "Waiting for an eligible drop or live channel · retrying in 60s"
        }
        .into();
        let finished = completed_any && !removed_non_watchable && s.queue.is_empty();
        let notify = finished && s.notifications.queue;
        let countdown = finished && s.error.is_none() && s.sleep_plan.finish(Instant::now());
        drop(s);
        if countdown {
            crate::desktop::show(&self.app);
            self.log(
                "info",
                "Queue complete. PC will sleep in 60 seconds unless cancelled.",
            )
            .await;
            self.emit().await;
        }
        if notify {
            self.notify("Queue finished", "All queued rewards have been claimed.")
                .await;
        }
        Ok(())
    }
    pub async fn worker(self: Arc<Self>) {
        loop {
            if !self.snapshot.lock().await.running {
                self.wake.notified().await;
                continue;
            }
            let result =
                tokio::select! { biased; _=self.wake.notified()=>{continue;},r=self.tick()=>r };
            if let Err(e) = result {
                self.log("warning", &e).await;
                self.report_error(&e).await;
                let mut s = self.snapshot.lock().await;
                s.error = Some(e);
                if !s.needs_reconnect {
                    s.status = "Connection interrupted · retrying in 60s".into();
                    let queue = s.queue.clone();
                    for id in queue {
                        s.queue_statuses.insert(
                            id,
                            QueueStatus::checked("retrying", "Connection interrupted", true),
                        );
                    }
                }
                s.channel = None;
                s.active_campaign = None;
            }
            {
                let mut s = self.snapshot.lock().await;
                let retry_at = (Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
                for status in s.queue_statuses.values_mut() {
                    if status.retry_at.is_some() {
                        status.retry_at = Some(retry_at.clone());
                    }
                }
            }
            self.emit().await;
            tokio::select! {_=self.wake.notified()=>{},_=tokio::time::sleep(Duration::from_secs(60))=>{}}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_preferences_enable_tray_without_enabling_notifications() {
        let prefs: Preferences =
            serde_json::from_str(r#"{"queue":["campaign"],"owner":"account","auto_claim":false}"#)
                .unwrap();
        assert!(prefs.tray_enabled);
        assert!(
            !prefs.notifications.rewards
                && !prefs.notifications.queue
                && !prefs.notifications.reconnect
        );
        assert!(!prefs.auto_claim);
        assert!(!prefs.auto_farm.enabled);
        assert!(prefs.auto_farm.games.is_empty());
        assert_eq!(prefs.queue, ["campaign"]);
    }
}
