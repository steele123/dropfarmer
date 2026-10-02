use crate::browser_login::BrowserLogin;
use crate::campaign_cache::CampaignCache;
use crate::{model::*, twitch::*};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, Notify};

#[derive(Default, Serialize, Deserialize)]
struct Preferences {
    queue: Vec<String>,
    owner: Option<String>,
    #[serde(default = "enabled")]
    auto_claim: bool,
}
fn enabled() -> bool {
    true
}
pub struct Engine {
    pub snapshot: Mutex<Snapshot>,
    gate: Mutex<()>,
    session: Mutex<Option<Session>>,
    pending: Mutex<Option<PendingSignIn>>,
    login_epoch: AtomicU64,
    pub wake: Notify,
    api: Twitch,
    app: AppHandle,
    path: PathBuf,
    last_watch: Mutex<Option<Instant>>,
    initialized: Mutex<bool>,
}
enum PendingSignIn {
    Code(PendingLogin),
    Browser(BrowserLogin),
}
impl Engine {
    pub fn new(app: AppHandle, path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            snapshot: Mutex::new(Snapshot::default()),
            gate: Mutex::new(()),
            session: Mutex::new(None),
            pending: Mutex::new(None),
            login_epoch: AtomicU64::new(0),
            wake: Notify::new(),
            api: Twitch::new(),
            app,
            path,
            last_watch: Mutex::new(None),
            initialized: Mutex::new(false),
        })
    }
    fn credential() -> Result<keyring::Entry, String> {
        keyring::Entry::new("app.dropfarmer.desktop", "twitch")
            .map_err(|_| "Cannot access the operating system credential store.".into())
    }
    pub async fn emit(&self) {
        let s = self.snapshot.lock().await.clone();
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
        let p = Preferences {
            queue: s.queue.clone(),
            owner: s.account.as_ref().map(|a| a.id.clone()),
            auto_claim: s.auto_claim,
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
        s.campaigns.clear();
        s.campaign_notice = None;
        s.campaigns_cached = false;
        s.last_sync = None;
        if let Some(p) = prefs {
            s.auto_claim = p.auto_claim;
            if p.owner.as_deref() == Some(&session.account.id) {
                s.queue = p.queue;
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
        s.error = None;
        *self.session.lock().await = Some(session);
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
        Ok(self.snapshot.lock().await.clone())
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
        *self.snapshot.lock().await = Snapshot::default();
        self.emit().await;
        Ok(())
    }
    pub async fn refresh(&self) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        let session = self
            .session
            .lock()
            .await
            .clone()
            .ok_or("Connect Twitch first.")?;
        let discovery = self.api.campaigns(&session).await?;
        let count = discovery.campaigns.len();
        {
            let mut s = self.snapshot.lock().await;
            s.campaigns = discovery.campaigns;
            s.campaign_notice = discovery.notice.clone();
            s.campaigns_cached = false;
            s.last_sync = Some(Utc::now().to_rfc3339());
            s.error = None;
            if !s.running {
                s.status = "Ready".into();
            }
        }
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
                    || (!s.queue.contains(id)
                        && !s.campaigns.iter().any(|c| &c.id == id && c.can_queue()))
            }) {
                return Err(
                    "Only known, incomplete campaigns that have not ended can be added once."
                        .into(),
                );
            }
            s.queue = ids;
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
    pub async fn set_running(&self, value: bool) -> Result<(), String> {
        {
            let mut s = self.snapshot.lock().await;
            if value && (s.account.is_none() || s.queue.is_empty()) {
                return Err("Connect Twitch and add a campaign to the queue first.".into());
            }
            s.running = value;
            s.error = None;
            s.status = if value {
                "Finding an eligible stream…"
            } else {
                "Queue paused"
            }
            .into();
            if !value {
                s.channel = None;
                s.active_campaign = None;
            }
        }
        self.wake.notify_one();
        self.emit().await;
        Ok(())
    }
    pub async fn claim(&self, campaign_id: &str, drop_id: &str) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        let session = self
            .session
            .lock()
            .await
            .clone()
            .ok_or("Connect Twitch first.")?;
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
            .find(|d| d.id == drop_id && !d.claimed && d.minutes >= d.required && d.required > 0)
            .ok_or("This reward is not ready to claim yet.")?;
        self.api.claim(&session, d).await?;
        let fresh = self.api.update_campaign(&session, &fresh).await?.ok_or(
            "Claim submitted, but Twitch has not returned the updated campaign. Refresh shortly.",
        )?;
        self.replace_campaign(fresh).await;
        self.log("success", "Reward claim confirmed by Twitch.")
            .await;
        self.emit().await;
        Ok(())
    }
    async fn replace_campaign(&self, c: Campaign) {
        let mut s = self.snapshot.lock().await;
        if let Some(old) = s.campaigns.iter_mut().find(|old| old.id == c.id) {
            *old = c;
        }
        s.last_sync = Some(Utc::now().to_rfc3339());
        drop(s);
        self.save_campaign_cache().await;
    }
    async fn save_campaign_cache(&self) {
        let cache = CampaignCache::from_snapshot(&*self.snapshot.lock().await);
        if let Some(cache) = cache {
            if let Err(error) = cache.write(&self.path.with_file_name("campaigns.json")) {
                self.log("warning", error).await;
            }
        }
    }
    async fn tick(&self) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        let session = self
            .session
            .lock()
            .await
            .clone()
            .ok_or("Reconnect Twitch to continue.")?;
        let state = self.snapshot.lock().await.clone();
        if !state.running {
            return Ok(());
        }
        let mut unavailable = Vec::new();
        // Refresh queued campaigns using details plus authoritative inventory progress.
        for id in &state.queue {
            let Some(old) = state.campaigns.iter().find(|c| &c.id == id) else {
                continue;
            };
            let Some(mut c) = self.api.update_campaign(&session, old).await? else {
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
                        && d.required > 0
                        && d.minutes >= d.required
                        && d.claim_id.is_some()
                    {
                        self.api.claim(&session, d).await?;
                        self.log("success", format!("Claimed {} · {}", d.name, c.game))
                            .await;
                    }
                }
                if c.drops.iter().any(|d| {
                    !d.claimed && d.required > 0 && d.minutes >= d.required && d.claim_id.is_some()
                }) {
                    let Some(fresh) = self.api.update_campaign(&session, &c).await? else {
                        unavailable.push(c.name.clone());
                        continue;
                    };
                    c = fresh;
                }
            }
            self.replace_campaign(c.clone()).await;
            if c.complete() {
                self.snapshot.lock().await.queue.retain(|i| i != id);
                self.save().await?;
                self.log(
                    "success",
                    format!("{} complete. Moving to the next campaign.", c.game),
                )
                .await;
                continue;
            }
            if c.next_drop().is_none() {
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
                s.active_campaign = Some(c.id);
                s.channel = Some(ch);
                s.status = "Farming · checking Twitch for progress".into();
                s.error = None;
                return Ok(());
            }
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
            "Queue complete"
        } else if !unavailable.is_empty() {
            "Waiting for Twitch campaign data · retrying in 60s"
        } else {
            "Waiting for an eligible drop or live channel · retrying in 60s"
        }
        .into();
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
                let mut s = self.snapshot.lock().await;
                s.error = Some(e);
                s.status = "Connection interrupted · retrying in 60s".into();
                s.channel = None;
                s.active_campaign = None;
            }
            self.emit().await;
            tokio::select! {_=self.wake.notified()=>{},_=tokio::time::sleep(Duration::from_secs(60))=>{}}
        }
    }
}
