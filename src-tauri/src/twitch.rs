use crate::browser_login::{BrowserContext, BrowserCredentials, WEB_CLIENT_ID};
use crate::model::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use futures_util::{stream, StreamExt, TryStreamExt};
use regex::Regex;
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Duration};

// Public client identity and persisted-query signatures from TwitchDropsMiner.
// These private Twitch endpoints are isolated here so changes are easy to repair.
pub const CLIENT_ID: &str = "ue6666qo983tsx6so1t0vnawi233wa";
#[derive(Clone)]
pub struct Twitch {
    http: Client,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub account: Account,
    #[serde(default)]
    pub browser: Option<BrowserContext>,
}
pub struct CampaignDiscovery {
    pub campaigns: Vec<Campaign>,
    pub notice: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginCode {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}
pub struct PendingLogin {
    pub device_code: String,
    pub deadline: std::time::Instant,
    pub next_poll: std::time::Instant,
    pub interval: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum GraphqlFailure {
    Integrity,
    QueryChanged,
    Unauthorized,
    Forbidden,
    Unknown,
    RateLimited,
    Unavailable,
}
fn graphql_failure(response: &Value) -> Option<GraphqlFailure> {
    let errors = array(&response["errors"]);
    errors
        .iter()
        .map(|error| {
            let message = error["message"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let code = error["extensions"]["code"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let has = |needle: &str| message.contains(needle) || code.contains(needle);
            if has("integrity") {
                GraphqlFailure::Integrity
            } else if has("persistedquerynotfound") || has("persisted_query_not_found") {
                GraphqlFailure::QueryChanged
            } else if has("unauth") || has("invalid oauth") || has("invalid token") {
                GraphqlFailure::Unauthorized
            } else if has("forbidden") || has("access denied") {
                GraphqlFailure::Forbidden
            } else if has("too many requests")
                || matches!(code.as_str(), "too_many_requests" | "rate_limited")
            {
                GraphqlFailure::RateLimited
            } else if matches!(
                message.as_str(),
                "service error"
                    | "server error"
                    | "service timeout"
                    | "request cancelled"
                    | "service unavailable"
                    | "context deadline exceeded"
                    | "internal server error"
            ) || matches!(
                code.as_str(),
                "internal_server_error"
                    | "service_unavailable"
                    | "timeout"
                    | "request_timeout"
                    | "gateway_timeout"
            ) {
                GraphqlFailure::Unavailable
            } else {
                GraphqlFailure::Unknown
            }
        })
        .min() // An authorization, unknown, or query error must not be hidden by a transient error.
}

// Never expose raw upstream messages: they can contain request or account data.
fn graphql_error(response: &Value, operation: &str) -> Option<String> {
    let failure = graphql_failure(response)?;
    let step = match operation {
        "ViewerDropsDashboard" => "campaign list",
        "Inventory" => "inventory",
        "DropCampaignDetails" => "campaign details",
        "DropsPage_ClaimDropRewards" => "reward claim",
        _ => "drop request",
    };
    let reason = match failure {
        GraphqlFailure::Integrity => {
            "Twitch rejected the browser session proof. Try browser sign-in again."
        }
        GraphqlFailure::QueryChanged => {
            "Twitch no longer recognizes this query. The app's query needs updating."
        }
        GraphqlFailure::Unauthorized => "Twitch rejected the saved authorization. Sign in again.",
        GraphqlFailure::Forbidden => "Twitch denied access for this session.",
        GraphqlFailure::RateLimited => "Twitch is limiting requests. Wait a moment and try again.",
        GraphqlFailure::Unavailable => {
            "Twitch's service is temporarily unavailable. Try again shortly."
        }
        GraphqlFailure::Unknown => "Twitch rejected this request (unclassified GraphQL error).",
    };
    Some(format!("Could not load {step}: {reason}"))
}

async fn gql_with_retry<F, Fut>(operation: &str, mut request: F) -> Result<Value, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let read_only = matches!(
        operation,
        "Inventory"
            | "ViewerDropsDashboard"
            | "DropCampaignDetails"
            | "VideoPlayerStreamInfoOverlayChannel"
            | "DropsHighlightService_AvailableDrops"
            | "DirectoryGameRedirect"
            | "DirectoryPage_Game"
    );
    for attempt in 0..3 {
        let response = request().await?;
        let transient = matches!(
            graphql_failure(&response),
            Some(GraphqlFailure::Unavailable | GraphqlFailure::RateLimited)
        );
        if read_only && transient && attempt < 2 {
            tokio::time::sleep(Duration::from_secs([2, 5][attempt])).await;
            continue;
        }
        if let Some(error) = graphql_error(&response, operation) {
            return Err(error);
        }
        if response["data"].is_null() {
            return Err("Twitch's drop response format has changed.".into());
        }
        return Ok(response["data"].clone());
    }
    unreachable!("The last attempt always returns")
}

async fn decode(response: Result<Response, reqwest::Error>) -> Result<Value, String> {
    let response =
        response.map_err(|_| "Cannot reach Twitch. Check your connection.".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Twitch returned HTTP {status}. Please retry or reconnect Twitch."
        ));
    }
    response
        .json()
        .await
        .map_err(|_| "Twitch returned an unexpected response.".into())
}
pub fn persisted(name: &str, variables: Value) -> Value {
    let hash = match name {
        "Inventory" => "8337eb8541b314040b0edde0c09c5c7a2783ba1960aa9edfbf3bac16d0fec404",
        "ViewerDropsDashboard" => {
            "c16bb890cc8ce7647a96ee69cd313d423a378a3dedadf630a1017cde18975feb"
        }
        "DropCampaignDetails" => "039277bf98f3130929262cc7c6efd9c141ca3749cb6dca442fc8ead9a53f77c1",
        "DropsPage_ClaimDropRewards" => {
            "a455deea71bdc9015b78eb49f4acfbce8baa7ccbedd28e549bb025bd0f751930"
        }
        "VideoPlayerStreamInfoOverlayChannel" => {
            "198492e0857f6aedead9665c81c5a06d67b25b58034649687124083ff288597d"
        }
        "DropsHighlightService_AvailableDrops" => {
            "782dad0f032942260171d2d80a654f88bdd0c5a9dddc392e9bc92218a0f42d20"
        }
        "DirectoryGameRedirect" => {
            "1f0300090caceec51f33c5e20647aceff9017f740f223c3c532ba6fa59f6b6cc"
        }
        "DirectoryPage_Game" => "86bcceb4e8b1a51256ff8eed8bd8aae4acacf80d737efe904f84f3aeadf8cafd",
        _ => unreachable!("Unknown internal operation"),
    };
    json!({"operationName":name,"variables":variables,"extensions":{"persistedQuery":{"version":1,"sha256Hash":hash}}})
}
impl Twitch {
    pub fn new() -> Self {
        Self { http: Client::builder().timeout(Duration::from_secs(25)).user_agent("Mozilla/5.0 (Linux; Android 7.1; Smart Box C1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
            .redirect(reqwest::redirect::Policy::none()).build().expect("HTTP client") }
    }
    pub async fn begin_login(&self) -> Result<(PendingLogin, LoginCode), String> {
        let v = decode(
            self.http
                .post("https://id.twitch.tv/oauth2/device")
                .form(&[("client_id", CLIENT_ID), ("scopes", "")])
                .send()
                .await,
        )
        .await?;
        let device_code = string(&v, "device_code");
        let uri = string(&v, "verification_uri");
        if device_code.is_empty() || !valid_twitch_link(&uri) {
            return Err("Twitch did not return a valid sign-in code.".into());
        }
        let interval = v["interval"].as_u64().unwrap_or(5).max(5);
        let expires_in = v["expires_in"].as_u64().unwrap_or(1800).min(3600);
        let now = std::time::Instant::now();
        Ok((
            PendingLogin {
                device_code,
                deadline: now + Duration::from_secs(expires_in),
                next_poll: now + Duration::from_secs(interval),
                interval,
            },
            LoginCode {
                user_code: string(&v, "user_code"),
                verification_uri: uri,
                expires_in,
                interval,
            },
        ))
    }
    pub async fn poll_login(&self, p: &mut PendingLogin) -> Result<Option<Session>, String> {
        let now = std::time::Instant::now();
        if now >= p.deadline {
            return Err("Sign-in code expired. Request a new code.".into());
        }
        if now < p.next_poll {
            return Ok(None);
        }
        p.next_poll = now + Duration::from_secs(p.interval);
        let response = self
            .http
            .post("https://id.twitch.tv/oauth2/token")
            .form(&[
                ("client_id", CLIENT_ID),
                ("device_code", &p.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(|_| "Could not check sign-in. Try again.".to_string())?;
        let status = response.status();
        let v: Value = response
            .json()
            .await
            .map_err(|_| "Invalid sign-in response.".to_string())?;
        if status.is_success() {
            let token = string(&v, "access_token");
            return self.validate(token).await.map(Some);
        }
        let msg = string(&v, "message");
        let err = string(&v, "error");
        if msg.contains("authorization_pending") || err == "authorization_pending" {
            return Ok(None);
        }
        if msg.contains("slow_down") || err == "slow_down" {
            p.interval += 5;
            p.next_poll = now + Duration::from_secs(p.interval);
            return Ok(None);
        }
        Err("Twitch sign-in was declined or expired. Request a new code.".into())
    }
    pub async fn validate(&self, token: String) -> Result<Session, String> {
        self.validate_context(token, None).await
    }
    pub async fn validate_context(
        &self,
        token: String,
        browser: Option<BrowserContext>,
    ) -> Result<Session, String> {
        if let Some(context) = &browser {
            context.validate_structure()?;
        }
        let v = decode(
            self.http
                .get("https://id.twitch.tv/oauth2/validate")
                .header("Authorization", format!("OAuth {token}"))
                .send()
                .await,
        )
        .await?;
        let expected_client = if browser.is_some() {
            WEB_CLIENT_ID
        } else {
            CLIENT_ID
        };
        if string(&v, "user_id").is_empty() || string(&v, "client_id") != expected_client {
            return Err("Saved Twitch session is invalid. Reconnect Twitch.".into());
        }
        Ok(Session {
            token,
            browser,
            account: Account {
                id: string(&v, "user_id"),
                login: string(&v, "login"),
            },
        })
    }
    pub async fn accept_browser(&self, credentials: BrowserCredentials) -> Result<Session, String> {
        let session = self
            .validate_context(credentials.token, Some(credentials.context))
            .await?;
        let dashboard = self
            .gql(
                &session,
                persisted(
                    "ViewerDropsDashboard",
                    json!({"fetchRewardCampaigns":false}),
                ),
            )
            .await?;
        if campaign_dashboard(&dashboard)?.is_none() {
            return Err("Twitch did not grant campaign access. Try browser sign-in again.".into());
        }
        let inventory = self
            .gql(
                &session,
                persisted("Inventory", json!({"fetchRewardCampaigns":false})),
            )
            .await?;
        if !inventory["currentUser"]["inventory"].is_object() {
            return Err(
                "Twitch inventory could not be verified. Try browser sign-in again.".into(),
            );
        }
        Ok(session)
    }
    pub async fn renew_browser(
        &self,
        saved: &Session,
        root: &std::path::Path,
    ) -> Result<Session, String> {
        let validated = self
            .validate_context(saved.token.clone(), saved.browser.clone())
            .await?;
        if validated.account.id != saved.account.id {
            return Err("Saved Twitch session is invalid. Reconnect Twitch.".into());
        }
        let credentials =
            crate::browser_login::BrowserLogin::renew(root, saved.token.clone()).await?;
        let renewed = self.accept_browser(credentials).await?;
        if renewed.account.id != saved.account.id {
            return Err(
                "The browser connected a different Twitch account. Reconnect Twitch.".into(),
            );
        }
        Ok(renewed)
    }
    async fn gql(&self, s: &Session, op: Value) -> Result<Value, String> {
        gql_with_retry(op["operationName"].as_str().unwrap_or_default(), || {
            self.gql_response(s, op.clone())
        })
        .await
    }
    async fn gql_response(&self, s: &Session, mut op: Value) -> Result<Value, String> {
        let mut request = self
            .http
            .post("https://gql.twitch.tv/gql")
            .header("Authorization", format!("OAuth {}", s.token))
            .header("Origin", "https://www.twitch.tv");
        if let Some(context) = &s.browser {
            context.validate()?;
            // The web catalog uses newer persisted operations than the TV client.
            let hash = match op["operationName"].as_str() {
                Some("Inventory") => {
                    Some("d86775d0ef16a63a33ad52e80eaff963b2d5b72fada7c991504a57496e1d8e4b")
                }
                Some("ViewerDropsDashboard") => {
                    Some("5a4da2ab3d5b47c9f9ce864e727b2cb346af1e3ea8b897fe8f704a97ff017619")
                }
                _ => None,
            };
            if let Some(hash) = hash {
                op["extensions"]["persistedQuery"]["sha256Hash"] = json!(hash);
            }
            request = request
                .header("Client-ID", WEB_CLIENT_ID)
                .header("User-Agent", &context.user_agent)
                .header("Referer", "https://www.twitch.tv/");
            for (name, value) in &context.headers {
                request = request.header(name, value);
            }
        } else {
            request = request.header("Client-ID", CLIENT_ID);
        }
        decode(request.json(&op).send().await).await
    }
    pub async fn campaigns(&self, s: &Session) -> Result<CampaignDiscovery, String> {
        let inv = self
            .gql(
                s,
                persisted("Inventory", json!({"fetchRewardCampaigns":false})),
            )
            .await?;
        if inv["currentUser"]["inventory"].is_null() {
            return Err("Twitch inventory is unavailable. Reconnect your account.".into());
        }
        let dashboard = self
            .gql(
                s,
                persisted(
                    "ViewerDropsDashboard",
                    json!({"fetchRewardCampaigns":false}),
                ),
            )
            .await?;
        let available = campaign_dashboard(&dashboard)?;
        let mut notice = available.is_none().then(||
            "This Twitch login only returns campaigns already in progress. Start a missing drop on Twitch, then refresh.".to_string());
        let mut campaigns = BTreeMap::new();
        for v in array(&inv["currentUser"]["inventory"]["dropCampaignsInProgress"]) {
            campaigns.insert(string(v, "id"), v.clone());
        }
        let summaries: Vec<Value> = available
            .unwrap_or_default()
            .iter()
            .filter(|v| matches!(v["status"].as_str(), Some("ACTIVE" | "UPCOMING")))
            .cloned()
            .collect();
        let mut details = stream::iter(summaries)
            .map(|v| {
                let api = self.clone();
                let session = s.clone();
                async move {
                    // Bound concurrency and retain pacing instead of issuing a catalog-sized burst.
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    let id = string(&v, "id");
                    let detail = api
                        .gql(
                            &session,
                            persisted(
                                "DropCampaignDetails",
                                json!({"channelLogin":session.account.id,"dropID":id}),
                            ),
                        )
                        .await?;
                    Ok::<_, String>((v, id, detail))
                }
            })
            .buffer_unordered(4);
        while let Some((v, id, detail)) = details.try_next().await? {
            let details = &detail["user"]["dropCampaign"];
            if details.is_null() {
                // Inventory can still contain a usable campaign when details are unavailable.
                if notice.is_none() {
                    notice = Some("Twitch omitted details for some campaigns. The list may be incomplete; try refreshing later.".into());
                }
                continue;
            }
            let full = merge(details, &v);
            campaigns.insert(
                id.clone(),
                campaigns.get(&id).map(|i| merge(i, &full)).unwrap_or(full),
            );
        }
        let mut result: Vec<_> = campaigns
            .values()
            .filter_map(Campaign::from_value)
            .collect();
        result.sort_by(|a, b| b.linked.cmp(&a.linked).then(a.ends_at.cmp(&b.ends_at)));
        Ok(CampaignDiscovery {
            campaigns: result,
            notice,
        })
    }
    pub async fn update_campaign(
        &self,
        s: &Session,
        c: &Campaign,
    ) -> Result<Option<Campaign>, String> {
        let detail = self
            .gql(
                s,
                persisted(
                    "DropCampaignDetails",
                    json!({"channelLogin":s.account.id,"dropID":c.id}),
                ),
            )
            .await?;
        let inventory = self
            .gql(
                s,
                persisted("Inventory", json!({"fetchRewardCampaigns":false})),
            )
            .await?;
        refreshed_campaign(
            &c.id,
            &inventory["currentUser"]["inventory"],
            &detail["user"]["dropCampaign"],
        )
    }
    async fn stream(&self, s: &Session, login: &str) -> Result<Option<Channel>, String> {
        let v = self
            .gql(
                s,
                persisted(
                    "VideoPlayerStreamInfoOverlayChannel",
                    json!({"channel":login}),
                ),
            )
            .await?;
        let u = &v["user"];
        if u["stream"].is_null() {
            return Ok(None);
        }
        Ok(Some(Channel {
            id: string(u, "id"),
            login: login.into(),
            name: string(u, "displayName"),
            broadcast_id: string(&u["stream"], "id"),
            game_id: string(&u["broadcastSettings"]["game"], "id"),
        }))
    }
    async fn eligible(&self, s: &Session, ch: &Channel, c: &Campaign) -> Result<bool, String> {
        if ch.game_id != c.game_id {
            return Ok(false);
        }
        let v = self
            .gql(
                s,
                persisted(
                    "DropsHighlightService_AvailableDrops",
                    json!({"channelID":ch.id}),
                ),
            )
            .await?;
        Ok(array(&v["channel"]["viewerDropCampaigns"])
            .iter()
            .any(|d| d["id"] == c.id))
    }
    pub async fn find_channel(
        &self,
        s: &Session,
        c: &Campaign,
        previous: Option<&Channel>,
    ) -> Result<Option<Channel>, String> {
        if let Some(old) = previous {
            if c.channels.is_empty() || c.channels.contains(&old.login) {
                if let Some(ch) = self.stream(s, &old.login).await? {
                    if self.eligible(s, &ch, c).await? {
                        return Ok(Some(ch));
                    }
                }
            }
        }
        let candidates = if !c.channels.is_empty() {
            c.channels.clone()
        } else {
            let redirect = self
                .gql(
                    s,
                    persisted("DirectoryGameRedirect", json!({"name":c.game})),
                )
                .await?;
            let slug = string(&redirect["game"], "slug");
            if slug.is_empty() {
                return Ok(None);
            }
            let v = self.gql(s,persisted("DirectoryPage_Game",json!({"limit":30,"slug":slug,"imageWidth":50,"includeCostreaming":false,"sortTypeIsRecency":false,
                "options":{"broadcasterLanguages":[],"freeformTags":null,"includeRestricted":["SUB_ONLY_LIVE"],"recommendationsContext":{"platform":"web"},"sort":"VIEWER_COUNT","systemFilters":["DROPS_ENABLED"],"tags":[],"requestID":"JIRA-VXP-2397"}}))).await?;
            array(&v["game"]["streams"]["edges"])
                .iter()
                .map(|e| string(&e["node"]["broadcaster"], "login"))
                .filter(|s| !s.is_empty())
                .collect()
        };
        for login in candidates {
            if let Some(ch) = self.stream(s, &login).await? {
                if self.eligible(s, &ch, c).await? {
                    return Ok(Some(ch));
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Ok(None)
    }
    pub async fn claim(&self, s: &Session, drop: &Drop) -> Result<(), String> {
        let id = drop
            .claim_id
            .as_ref()
            .ok_or("Twitch has not supplied a claim ID yet.")?;
        let v = self
            .gql(
                s,
                persisted(
                    "DropsPage_ClaimDropRewards",
                    json!({"input":{"dropInstanceID":id}}),
                ),
            )
            .await?;
        match v["claimDropRewards"]["status"].as_str() {
            Some("ELIGIBLE_FOR_ALL" | "DROP_INSTANCE_ALREADY_CLAIMED") => Ok(()),
            _ => Err("Twitch has not confirmed this reward claim. It will be retried.".into()),
        }
    }
    async fn watch_endpoint(&self, login: &str) -> Result<String, String> {
        let html = self
            .public_text(&format!("https://www.twitch.tv/{login}"))
            .await?;
        let spade = Regex::new(r#"(?i)"spade_?url":\s*"(https://[.\w\-/]+)""#).unwrap();
        let url = if let Some(cap) = spade.captures(&html) {
            cap[1].to_owned()
        } else {
            let settings =
                Regex::new(r#"src="(https://[\w.]+/config/settings\.[0-9a-f]{32}\.js)""#).unwrap();
            let cap = settings.captures(&html).ok_or("Twitch's watch endpoint could not be discovered. An adapter update may be required.")?;
            let js = self.public_text(&cap[1]).await?;
            spade
                .captures(&js)
                .ok_or("Twitch's watch endpoint is unavailable.")?[1]
                .to_owned()
        };
        if !trusted_asset_url(&url) {
            return Err("Unexpected watch endpoint from Twitch.".into());
        }
        Ok(url)
    }
    pub async fn watch(&self, s: &Session, c: &Campaign, ch: &Channel) -> Result<(), String> {
        let url = self.watch_endpoint(&ch.login).await?;
        let payload = json!([{"event":"minute-watched","properties":{"broadcast_id":ch.broadcast_id,"channel_id":ch.id,"channel":ch.login,"client_time":Utc::now().to_rfc3339(),"game":c.game,"game_id":c.game_id,"hidden":false,"is_live":true,"live":true,"logged_in":true,"minutes_logged":1,"muted":false,"user_id":s.account.id}}]);
        let response = self
            .http
            .post(url)
            .form(&[("data", STANDARD.encode(payload.to_string()))])
            .send()
            .await
            .map_err(|_| "Watch heartbeat failed. Retrying shortly.".to_string())?;
        if response.status().as_u16() != 204 {
            return Err(format!(
                "Twitch did not accept the watch heartbeat ({}).",
                response.status()
            ));
        }
        Ok(())
    }
    async fn public_text(&self, url: &str) -> Result<String, String> {
        if !trusted_asset_url(url) {
            return Err("Unexpected Twitch asset address.".into());
        }
        self.http
            .get(url)
            .send()
            .await
            .and_then(Response::error_for_status)
            .map_err(|_| "Could not load Twitch stream metadata.".to_string())?
            .text()
            .await
            .map_err(|_| "Invalid Twitch metadata.".into())
    }
}
fn campaign_dashboard(data: &Value) -> Result<Option<&[Value]>, String> {
    if !data["currentUser"].is_object() {
        return Err("Twitch did not return a campaign dashboard. Reconnect Twitch.".into());
    }
    match &data["currentUser"]["dropCampaigns"] {
        Value::Null => Ok(None),
        Value::Array(campaigns) => Ok(Some(campaigns)),
        _ => Err("Twitch returned an unexpected campaign list.".into()),
    }
}
// Only fresh Twitch responses can supply progress and eligibility. Never revive
// a missing campaign from the cached UI snapshot.
fn refreshed_campaign(
    id: &str,
    inventory: &Value,
    details: &Value,
) -> Result<Option<Campaign>, String> {
    let campaigns = inventory["dropCampaignsInProgress"]
        .as_array()
        .ok_or("Twitch inventory is unavailable. Reconnect your account.")?;
    let current = campaigns.iter().find(|v| v["id"].as_str() == Some(id));
    let details = (details["id"].as_str() == Some(id)).then_some(details);
    let data = match (current, details) {
        (Some(current), Some(details)) => merge(current, details),
        (Some(current), None) => current.clone(),
        (None, Some(details)) => details.clone(),
        (None, None) => return Ok(None),
    };
    Campaign::from_value(&data)
        .map(Some)
        .ok_or_else(|| "Twitch returned incomplete campaign data. Refresh again shortly.".into())
}
pub fn valid_twitch_link(value: &str) -> bool {
    Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && matches!(u.host_str(), Some("www.twitch.tv" | "twitch.tv"))
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none()
    })
}
fn trusted_asset_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.port().is_none()
            && u.username().is_empty()
            && u.password().is_none()
            && u.host_str().is_some_and(|h| {
                ["twitch.tv", "twitchcdn.net", "ttvnw.net"]
                    .iter()
                    .any(|root| h == *root || h.ends_with(&format!(".{root}")))
            })
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graphql_diagnostics_classify_failures_without_exposing_upstream_text() {
        for (message, expected) in [
            (
                "failed integrity check token=private",
                "browser session proof",
            ),
            ("PersistedQueryNotFound private", "query needs updating"),
            ("Unauthorized private", "saved authorization"),
            ("unexpected private", "unclassified GraphQL error"),
        ] {
            let error = graphql_error(
                &json!({"errors":[{"message":message}]}),
                "ViewerDropsDashboard",
            )
            .unwrap();
            assert!(error.contains("campaign list"));
            assert!(error.contains(expected));
            assert!(!error.contains("private"));
        }
        assert!(graphql_error(&json!({"data":{}}), "Inventory").is_none());
    }
    #[test]
    fn existing_code_sessions_remain_readable() {
        let session: Session =
            serde_json::from_value(json!({"token":"test", "account":{"id":"1","login":"test"}}))
                .unwrap();
        assert!(session.browser.is_none());
        assert_eq!(session.account.id, "1");
    }
    #[tokio::test]
    async fn transient_inventory_failure_retries_then_uses_fresh_data() {
        let mut calls = 0;
        let data = gql_with_retry("Inventory", || {
            calls += 1;
            std::future::ready(Ok(if calls == 1 {
                json!({"errors":[{"message":"service error"}], "data":{"stale":true}})
            } else {
                json!({"data":{"fresh":true}})
            }))
        })
        .await
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(data, json!({"fresh":true}));
    }
    #[tokio::test]
    async fn transient_inventory_failures_stop_after_three_attempts() {
        let mut calls = 0;
        let error = gql_with_retry("Inventory", || {
            calls += 1;
            std::future::ready(Ok(json!({"errors":[{"message":"service timeout"}]})))
        })
        .await
        .unwrap_err();
        assert_eq!(calls, 3);
        assert!(error.contains("inventory"));
        assert!(error.contains("temporarily unavailable"));
        assert!(!crate::notifications::requires_reconnect(&error));
    }
    #[tokio::test]
    async fn authorization_unknown_errors_and_mutations_are_never_retried() {
        for response in [
            json!({"errors":[{"message":"Unauthorized private"}, {"message":"service error"}]}),
            json!({"errors":[{"message":"PersistedQueryNotFound"}]}),
            json!({"errors":[{"message":"unknown private"}, {"message":"service error"}]}),
        ] {
            let mut calls = 0;
            let error = gql_with_retry("Inventory", || {
                calls += 1;
                std::future::ready(Ok(response.clone()))
            })
            .await
            .unwrap_err();
            assert_eq!(calls, 1);
            assert!(!error.contains("private"));
        }
        for operation in ["DropsPage_ClaimDropRewards", "UnknownOperation"] {
            let mut calls = 0;
            assert!(gql_with_retry(operation, || {
                calls += 1;
                std::future::ready(Ok(json!({"errors":[{"message":"service error"}]})))
            })
            .await
            .is_err());
            assert_eq!(calls, 1);
        }
    }
    #[tokio::test]
    async fn waiting_for_a_retry_can_be_cancelled() {
        let mut calls = 0;
        let result = tokio::time::timeout(
            Duration::from_millis(10),
            gql_with_retry("Inventory", || {
                calls += 1;
                std::future::ready(Ok(json!({"errors":[{"message":"service error"}]})))
            }),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
    #[test]
    fn service_and_rate_limit_codes_are_classified_without_leaking_messages() {
        for code in [
            "INTERNAL_SERVER_ERROR",
            "SERVICE_UNAVAILABLE",
            "REQUEST_TIMEOUT",
        ] {
            let error = graphql_error(
                &json!({"errors":[{"message":"private account data", "extensions":{"code":code}}]}),
                "Inventory",
            )
            .unwrap();
            assert!(error.contains("temporarily unavailable"));
            assert!(!error.contains("private"));
        }
        let error = graphql_error(
            &json!({"errors":[{"extensions":{"code":"RATE_LIMITED"}}]}),
            "Inventory",
        )
        .unwrap();
        assert!(error.contains("limiting requests"));
        assert!(!crate::notifications::requires_reconnect(&error));
    }
    #[cfg(target_os = "windows")]
    #[tokio::test]
    #[ignore = "Uses a background browser with the saved app login; verifies renewal without saving or claiming"]
    async fn live_browser_renewal() {
        let saved = keyring::Entry::new("app.dropfarmer.desktop", "twitch")
            .unwrap()
            .get_password()
            .expect("saved app session");
        let mut session: Session = serde_json::from_str(&saved).expect("session format");
        session.browser.as_mut().expect("browser login").expires_at = Utc::now().timestamp() - 60;
        let root = tempfile::tempdir().unwrap();
        let renewed = Twitch::new()
            .renew_browser(&session, root.path())
            .await
            .expect("renewal");
        assert_eq!(renewed.account.id, session.account.id);
        assert!(renewed.browser.as_ref().unwrap().validate().is_ok());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        println!("Expired proof renewed; account, campaign access, inventory and profile cleanup verified.");
    }
    #[cfg(target_os = "windows")]
    #[tokio::test]
    #[ignore = "Reads Twitch inventory using the saved session; prints only allowlisted diagnostics"]
    async fn live_inventory_diagnostic() {
        let saved = keyring::Entry::new("app.dropfarmer.desktop", "twitch")
            .unwrap()
            .get_password()
            .expect("saved app session");
        let session: Session = serde_json::from_str(&saved).expect("session format");
        let api = Twitch::new();
        let mut op = persisted("Inventory", json!({"fetchRewardCampaigns":false}));
        let mut request = api
            .http
            .post("https://gql.twitch.tv/gql")
            .header("Authorization", format!("OAuth {}", session.token))
            .header("Origin", "https://www.twitch.tv");
        println!("Browser session: {}", session.browser.is_some());
        if let Some(context) = &session.browser {
            context.validate().expect("valid browser context");
            op["extensions"]["persistedQuery"]["sha256Hash"] =
                json!("d86775d0ef16a63a33ad52e80eaff963b2d5b72fada7c991504a57496e1d8e4b");
            request = request
                .header("Client-ID", WEB_CLIENT_ID)
                .header("User-Agent", &context.user_agent)
                .header("Referer", "https://www.twitch.tv/");
            for (name, value) in &context.headers {
                request = request.header(name, value);
            }
        } else {
            request = request.header("Client-ID", CLIENT_ID);
        }
        let response = decode(request.json(&op).send().await)
            .await
            .expect("inventory response");
        for error in array(&response["errors"]) {
            let message = error["message"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let known = [
                "service error",
                "server error",
                "service timeout",
                "request cancelled",
                "service unavailable",
                "context deadline exceeded",
                "persistedquerynotfound",
                "unauthorized",
                "failed integrity check",
                "forbidden",
            ];
            println!(
                "Error kind: {}",
                known
                    .iter()
                    .find(|kind| **kind == message)
                    .copied()
                    .unwrap_or("unknown")
            );
            let path: Vec<_> = array(&error["path"])
                .iter()
                .map(|part| {
                    let part = part.as_str().unwrap_or("index");
                    if [
                        "currentUser",
                        "inventory",
                        "dropCampaignsInProgress",
                        "gameEventDrops",
                        "rewardCampaigns",
                        "index",
                    ]
                    .contains(&part)
                    {
                        part
                    } else {
                        "other"
                    }
                })
                .collect();
            println!("Error path: {path:?}");
        }
        println!(
            "Inventory object: {}; campaign array: {}; GraphQL errors: {}",
            response["data"]["currentUser"]["inventory"].is_object(),
            response["data"]["currentUser"]["inventory"]["dropCampaignsInProgress"].is_array(),
            array(&response["errors"]).len()
        );
        assert!(
            array(&response["errors"]).is_empty(),
            "Inventory returned the diagnostic errors above"
        );
    }
    #[cfg(target_os = "windows")]
    #[tokio::test]
    #[ignore = "Reads campaign discovery using the saved app session; no watch events or claims"]
    async fn live_campaign_discovery() {
        let saved = keyring::Entry::new("app.dropfarmer.desktop", "twitch")
            .unwrap()
            .get_password()
            .expect("saved app session");
        let session: Session = serde_json::from_str(&saved).unwrap();
        let api = Twitch::new();
        let data = api
            .gql(
                &session,
                persisted(
                    "ViewerDropsDashboard",
                    json!({"fetchRewardCampaigns":false}),
                ),
            )
            .await
            .unwrap();
        let discovery = api.campaigns(&session).await.unwrap();
        if campaign_dashboard(&data).unwrap().is_none() {
            assert!(
                discovery.notice.is_some(),
                "Limited discovery must be visible"
            );
        }
        println!(
            "Loaded {} campaigns. Discovery notice: {:?}",
            discovery.campaigns.len(),
            discovery.notice
        );
        for c in discovery.campaigns.iter().filter(|c| c.game == "Rust") {
            println!(
                "Rust campaign: {}; channels: {:?}; rewards: {:?}",
                c.name,
                c.channels,
                c.drops.iter().map(|d| &d.name).collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn missing_dashboard_is_not_an_empty_campaign_list() {
        assert!(
            campaign_dashboard(&json!({"currentUser":{"dropCampaigns":null}}))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            campaign_dashboard(&json!({"currentUser":{"dropCampaigns":[]}}))
                .unwrap()
                .unwrap()
                .len(),
            0
        );
        assert!(campaign_dashboard(&json!({"currentUser":null})).is_err());
        assert!(campaign_dashboard(&json!({"currentUser":{"dropCampaigns":"invalid"}})).is_err());
    }
    #[test]
    fn refresh_uses_inventory_when_details_are_missing() {
        let inventory = json!({"dropCampaignsInProgress":[{
            "id":"queued", "game":{"id":"game", "name":"Game"},
            "self":{"isAccountConnected":true},
            "timeBasedDrops":[{"id":"reward", "requiredMinutesWatched":60,
                "self":{"currentMinutesWatched":23,"isClaimed":false}}]
        }]});
        let campaign = refreshed_campaign("queued", &inventory, &Value::Null)
            .unwrap()
            .unwrap();
        assert!(campaign.linked);
        assert_eq!(campaign.drops[0].minutes, 23);
        assert_eq!(campaign.drops[0].required, 60);
        assert!(!campaign.drops[0].claimed);
    }
    #[test]
    fn refresh_merges_fresh_progress_with_detail_metadata() {
        let inventory = json!({"dropCampaignsInProgress":[{"id":"queued",
            "timeBasedDrops":[{"id":"reward","self":{"currentMinutesWatched":23}}]}]});
        let details = json!({"id":"queued", "game":{"id":"game"},
            "timeBasedDrops":[{"id":"reward","requiredMinutesWatched":60,
                "self":{"currentMinutesWatched":0}}]});
        let campaign = refreshed_campaign("queued", &inventory, &details)
            .unwrap()
            .unwrap();
        assert_eq!(campaign.game_id, "game");
        assert_eq!(campaign.drops[0].minutes, 23);
        assert_eq!(campaign.drops[0].required, 60);
        assert!(
            refreshed_campaign("queued", &json!({"dropCampaignsInProgress":[]}), &details)
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn refresh_distinguishes_unavailable_from_invalid_responses() {
        let empty = json!({"dropCampaignsInProgress":[]});
        assert!(refreshed_campaign("queued", &empty, &Value::Null)
            .unwrap()
            .is_none());
        assert!(
            refreshed_campaign("queued", &empty, &json!({"id":"other","game":{}}))
                .unwrap()
                .is_none()
        );
        assert!(refreshed_campaign("queued", &Value::Null, &Value::Null).is_err());
        assert!(refreshed_campaign("queued", &empty, &json!({"id":"queued"})).is_err());
    }
    #[cfg(target_os = "windows")]
    #[tokio::test]
    #[ignore = "Reads the saved app session and queued campaigns from Twitch; no claims or watch events"]
    async fn live_queued_campaign_refresh() {
        let saved = keyring::Entry::new("app.dropfarmer.desktop", "twitch")
            .unwrap()
            .get_password()
            .expect("saved app session");
        let session: Session = serde_json::from_str(&saved).expect("session format");
        let path = std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap())
            .join("app.dropfarmer.desktop/preferences.json");
        let prefs: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let api = Twitch::new();
        assert!(!array(&prefs["queue"]).is_empty(), "Queue a campaign first");
        for id in array(&prefs["queue"]) {
            let old = Campaign {
                id: id.as_str().unwrap().into(),
                ..Default::default()
            };
            let fresh = api
                .update_campaign(&session, &old)
                .await
                .expect("campaign refresh")
                .expect("campaign available");
            assert_eq!(fresh.id, old.id);
            assert!(!fresh.drops.is_empty(), "campaign has rewards");
            println!(
                "Refreshed {}: {} rewards, linked: {}",
                fresh.name,
                fresh.drops.len(),
                fresh.linked
            );
        }
    }
    #[tokio::test]
    #[ignore = "Queries the public Twitch game directory; no account or watch events"]
    async fn live_public_graphql() {
        let api = Twitch::new();
        let value = decode(
            api.http
                .post("https://gql.twitch.tv/gql")
                .header("Client-ID", CLIENT_ID)
                .json(&persisted("DirectoryGameRedirect", json!({"name":"Rust"})))
                .send()
                .await,
        )
        .await
        .expect("public GraphQL endpoint");
        assert!(array(&value["errors"]).is_empty(), "public query rejected");
        assert!(
            !string(&value["data"]["game"], "slug").is_empty(),
            "game slug missing"
        );
    }
    #[tokio::test]
    #[ignore = "Fetches public Twitch metadata; sends no watch events"]
    async fn live_watch_endpoint_discovery() {
        let url = Twitch::new()
            .watch_endpoint("twitch")
            .await
            .expect("discover watch endpoint");
        assert!(trusted_asset_url(&url));
    }
    #[tokio::test]
    #[ignore = "Contacts Twitch and requests a temporary device code; no account is authorized"]
    async fn live_device_authorization() {
        let api = Twitch::new();
        let (mut pending, code) = api
            .begin_login()
            .await
            .expect("device authorization endpoint");
        assert!(!code.user_code.is_empty());
        assert!(valid_twitch_link(&code.verification_uri));
        tokio::time::sleep(Duration::from_secs(code.interval)).await;
        assert!(api
            .poll_login(&mut pending)
            .await
            .expect("pending authorization response")
            .is_none());
    }
    #[test]
    fn external_links_are_scoped() {
        assert!(valid_twitch_link("https://www.twitch.tv/activate"));
        assert!(!valid_twitch_link("https://twitch.tv.evil.test/"));
        assert!(!valid_twitch_link("file:///etc/passwd"));
        assert!(!trusted_asset_url("https://twitch.tv@evil.test"));
    }
}
