use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    io::Read,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::{
    net::TcpStream,
    task::JoinHandle,
    time::{timeout, Instant},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

pub const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const GQL: &str = "https://gql.twitch.tv/gql";
const INTEGRITY: &str = "https://gql.twitch.tv/integrity";
const ALLOWED_HEADERS: &[&str] = &[
    "client-integrity",
    "client-version",
    "client-session-id",
    "x-device-id",
    "device-id",
    "accept-language",
];

// Credentials never implement Debug and never cross the frontend IPC boundary.
#[derive(Clone, Serialize, Deserialize)]
pub struct BrowserContext {
    pub user_agent: String,
    pub headers: BTreeMap<String, String>,
    pub expires_at: i64,
}
pub struct BrowserCredentials {
    pub token: String,
    pub context: BrowserContext,
}
impl BrowserContext {
    pub fn needs_renewal(&self) -> bool {
        self.expires_at <= Utc::now().timestamp() + 180
    }
    pub fn validate(&self) -> Result<(), String> {
        self.validate_structure()?;
        if self.expires_at <= Utc::now().timestamp() + 30 {
            return Err("Browser session needs renewal. Retry in a moment.".into());
        }
        Ok(())
    }
    // The proof's expiry is independent of the OAuth login's validity.
    pub fn validate_structure(&self) -> Result<(), String> {
        let printable = |v: &str| !v.is_empty() && v.bytes().all(|b| (32..=126).contains(&b));
        if self.expires_at <= 0
            || self.expires_at > Utc::now().timestamp() + 86400
            || self.user_agent.len() > 1024
            || !printable(&self.user_agent)
            || self.headers.iter().any(|(k, v)| {
                !ALLOWED_HEADERS.contains(&k.as_str()) || v.len() > 16384 || !printable(v)
            })
            || !self.headers.contains_key("client-integrity")
            || !(self.headers.contains_key("x-device-id") || self.headers.contains_key("device-id"))
        {
            return Err("Browser session data is invalid. Sign in again.".into());
        }
        Ok(())
    }
}

pub struct BrowserLogin {
    task: JoinHandle<Result<BrowserCredentials, String>>,
}
impl BrowserLogin {
    pub fn start(root: &Path) -> Result<Self, String> {
        let executable = browser_executable()?;
        std::fs::create_dir_all(root).map_err(|_| "Cannot create the browser sign-in folder.")?;
        let profile = tempfile::Builder::new()
            .prefix("signin-")
            .tempdir_in(root)
            .map_err(|_| "Cannot create a private browser profile.")?;
        Ok(Self {
            task: tokio::spawn(async move { capture(executable, profile, None).await }),
        })
    }
    pub async fn renew(root: &Path, token: String) -> Result<BrowserCredentials, String> {
        let executable = browser_executable()?;
        std::fs::create_dir_all(root).map_err(|_| "Cannot create the browser sign-in folder.")?;
        let profile = tempfile::Builder::new()
            .prefix("renew-")
            .tempdir_in(root)
            .map_err(|_| "Cannot create a private browser profile.")?;
        // Dropping this future also closes the browser and removes its temporary profile.
        timeout(
            Duration::from_secs(90),
            capture(executable, profile, Some(token)),
        )
        .await
        .map_err(|_| {
            "Browser renewal timed out. Retry or connect Twitch using browser sign-in.".to_string()
        })?
    }
    pub async fn poll(&mut self) -> Result<Option<BrowserCredentials>, String> {
        if !self.task.is_finished() {
            return Ok(None);
        }
        (&mut self.task)
            .await
            .map_err(|_| "Browser sign-in stopped. Try again.".to_string())?
            .map(Some)
    }
}
impl Drop for BrowserLogin {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn browser_executable() -> Result<PathBuf, String> {
    #[cfg(windows)]
    for (variable, suffix) in [
        ("PROGRAMFILES", "Google/Chrome/Application/chrome.exe"),
        ("LOCALAPPDATA", "Google/Chrome/Application/chrome.exe"),
        ("PROGRAMFILES(X86)", "Google/Chrome/Application/chrome.exe"),
        ("PROGRAMFILES(X86)", "Microsoft/Edge/Application/msedge.exe"),
        ("PROGRAMFILES", "Microsoft/Edge/Application/msedge.exe"),
        ("LOCALAPPDATA", "Microsoft/Edge/Application/msedge.exe"),
    ] {
        if let Some(base) = std::env::var_os(variable) {
            let path = PathBuf::from(base).join(suffix);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    Err(
        "Browser sign-in requires Chrome or Edge on Windows. Install one, or use a sign-in code."
            .into(),
    )
}

// A Windows job closes only this app's newly launched browser process tree,
// including on cancellation or app exit. Never attach to an everyday profile.
#[cfg(windows)]
struct BrowserJob(isize);
#[cfg(windows)]
impl BrowserJob {
    fn attach(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err("Cannot isolate the sign-in browser.".into());
            }
            let job = Self(handle as isize);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0
            {
                return Err("Cannot isolate the sign-in browser.".into());
            }
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for BrowserJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}
struct BrowserProcess {
    child: Child,
    port: u16,
    #[cfg(windows)]
    job: Option<BrowserJob>,
    profile: tempfile::TempDir,
}
impl BrowserProcess {
    fn launch(
        executable: &Path,
        profile: tempfile::TempDir,
        background: bool,
    ) -> Result<Self, String> {
        // Chrome treats --remote-debugging-port=0 as an automated browser.
        // Allocate a real loopback port for this interactive sign-in instead.
        let reservation = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|_| "Cannot allocate a local browser connection.")?;
        let port = reservation
            .local_addr()
            .map_err(|_| "Cannot allocate a local browser connection.")?
            .port();
        let startup = std::fs::File::create(profile.path().join("browser-startup.log"))
            .map_err(|_| "Cannot create the browser startup file.")?;
        let mut command = Command::new(executable);
        command
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .arg(format!("--remote-debugging-port={port}"))
            .args([
                "--remote-debugging-address=127.0.0.1",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-mode",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(startup));
        if background {
            command.arg("--no-startup-window");
        } else {
            command.args(["--new-window", "about:blank"]);
        }
        drop(reservation);
        let child = command
            .spawn()
            .map_err(|_| "Could not open the sign-in browser.")?;
        let mut owned = Self {
            child,
            port,
            #[cfg(windows)]
            job: None,
            profile,
        };
        #[cfg(windows)]
        {
            owned.job = Some(BrowserJob::attach(&owned.child)?);
        }
        Ok(owned)
    }
    fn is_closed(&mut self) -> bool {
        self.child
            .try_wait()
            .map_or(true, |status| status.is_some())
    }
    async fn endpoint(&mut self) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if self.is_closed() {
                return Err("The sign-in browser was closed.".into());
            }
            // Use only the endpoint emitted by our child. Do not probe a port
            // that another local process could acquire during browser startup.
            if let Ok(file) = std::fs::File::open(self.profile.path().join("browser-startup.log")) {
                let mut raw = String::new();
                let _ = file.take(65536).read_to_string(&mut raw);
                if let Some(endpoint) = startup_endpoint(&raw, self.port) {
                    return Ok(endpoint);
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Err("The sign-in browser did not start. Try the code option.".into())
    }
}
impl Drop for BrowserProcess {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            self.job.take();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // TempDir owns the exact unique directory created above and removes it.
    }
}
fn startup_endpoint(raw: &str, port: u16) -> Option<String> {
    if port == 0 {
        return None;
    }
    let prefix = format!("DevTools listening on ws://127.0.0.1:{port}/devtools/browser/");
    raw.lines().find_map(|line| {
        let id = line.trim().strip_prefix(&prefix)?;
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return None;
        }
        Some(format!("ws://127.0.0.1:{port}/devtools/browser/{id}"))
    })
}

struct Cdp {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    serial: u64,
    events: VecDeque<Value>,
}
impl Cdp {
    async fn read(&mut self) -> Result<Value, String> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    return serde_json::from_str(&text)
                        .map_err(|_| "Invalid browser response.".into())
                }
                Some(Ok(Message::Ping(data))) => {
                    self.socket
                        .send(Message::Pong(data))
                        .await
                        .map_err(|_| "The sign-in browser was closed.")?;
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                    return Err("The sign-in browser was closed.".into())
                }
                _ => {}
            }
        }
    }
    async fn command(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        self.serial += 1;
        let id = self.serial;
        let mut request = json!({"id":id,"method":method,"params":params});
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|_| "Cannot communicate with the sign-in browser.")?;
        timeout(Duration::from_secs(10), async {
            loop {
                let value = self.read().await?;
                if value["id"].as_u64() == Some(id) {
                    if value.get("error").is_some() {
                        return Err("The browser sign-in request failed.".into());
                    }
                    return Ok(value["result"].clone());
                }
                if value["method"].is_string() {
                    if self.events.len() >= 512 {
                        return Err("Too much browser activity. Restart sign-in.".into());
                    }
                    self.events.push_back(value);
                }
            }
        })
        .await
        .map_err(|_| "The sign-in browser stopped responding.".to_string())?
    }
    async fn event(&mut self) -> Result<Option<Value>, String> {
        if let Some(value) = self.events.pop_front() {
            return Ok(Some(value));
        }
        match timeout(Duration::from_secs(1), self.read()).await {
            Ok(result) => result.map(Some),
            Err(_) => Ok(None),
        }
    }
}

fn captured_request(request: &Value, user_agent: &str) -> Option<BrowserCredentials> {
    if request["url"].as_str() != Some(GQL) {
        return None;
    }
    let headers: BTreeMap<String, String> = request["headers"]
        .as_object()?
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|v| (k.to_ascii_lowercase(), v.to_owned())))
        .collect();
    if headers.get("client-id").map(String::as_str) != Some(WEB_CLIENT_ID) {
        return None;
    }
    let token = headers.get("authorization")?.strip_prefix("OAuth ")?;
    if token.is_empty()
        || token.len() > 512
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    let token = token.to_owned();
    let user_agent = headers
        .get("user-agent")
        .cloned()
        .unwrap_or_else(|| user_agent.into());
    let headers = headers
        .into_iter()
        .filter(|(k, _)| ALLOWED_HEADERS.contains(&k.as_str()))
        .collect();
    Some(BrowserCredentials {
        token,
        context: BrowserContext {
            user_agent,
            headers,
            expires_at: 0,
        },
    })
}

fn successful_catalog(data: &Value) -> bool {
    let accepted = |row: &Value| {
        row.get("errors")
            .is_none_or(|e| e.is_null() || e.as_array().is_some_and(|v| v.is_empty()))
            && row["data"]["currentUser"]["dropCampaigns"].is_array()
    };
    if let Some(rows) = data.as_array() {
        rows.iter().any(accepted)
    } else {
        accepted(data)
    }
}

fn ready_credentials(
    mut value: BrowserCredentials,
    issued: &BTreeMap<String, i64>,
) -> Option<BrowserCredentials> {
    value.context.expires_at = *issued.get(value.context.headers.get("client-integrity")?)?;
    value.context.validate().ok()?;
    Some(value)
}

async fn capture(
    executable: PathBuf,
    profile: tempfile::TempDir,
    saved_token: Option<String>,
) -> Result<BrowserCredentials, String> {
    let background = saved_token.is_some();
    let mut process = BrowserProcess::launch(&executable, profile, background)?;
    let endpoint = process.endpoint().await?;
    let (socket, _) = timeout(Duration::from_secs(10), connect_async(endpoint))
        .await
        .map_err(|_| "Cannot connect to the sign-in browser.")?
        .map_err(|_| "Cannot connect to the sign-in browser.")?;
    let mut cdp = Cdp {
        socket,
        serial: 0,
        events: VecDeque::new(),
    };
    let version = cdp.command("Browser.getVersion", json!({}), None).await?;
    let user_agent = version["userAgent"]
        .as_str()
        .ok_or("Browser identity unavailable.")?
        .to_owned();
    let target = if background {
        // A hidden target has no browser window or tab to take focus from a game.
        // If unsupported, fail renewal instead of falling back to a visible window.
        let created = cdp
            .command(
                "Target.createTarget",
                json!({"url":"about:blank","hidden":true,"background":true}),
                None,
            )
            .await?;
        created["targetId"]
            .as_str()
            .ok_or("Background browser renewal is unavailable.")?
            .to_owned()
    } else {
        let targets = cdp.command("Target.getTargets", json!({}), None).await?;
        targets["targetInfos"]
            .as_array()
            .and_then(|list| {
                list.iter()
                    .find(|v| v["type"] == "page" && v["url"] == "about:blank")
            })
            .and_then(|v| v["targetId"].as_str())
            .ok_or("The sign-in browser tab is unavailable.")?
            .to_owned()
    };
    #[cfg(test)]
    if background {
        assert!(
            cdp.command(
                "Browser.getWindowForTarget",
                json!({"targetId":target}),
                None
            )
            .await
            .is_err(),
            "Renewal target must not have a browser window"
        );
    }
    let attached = cdp
        .command(
            "Target.attachToTarget",
            json!({"targetId":target,"flatten":true}),
            None,
        )
        .await?;
    let session = attached["sessionId"]
        .as_str()
        .ok_or("Cannot open the sign-in browser tab.")?
        .to_owned();
    cdp.command(
        "Network.enable",
        json!({"maxTotalBufferSize":2000000,"maxResourceBufferSize":1000000}),
        Some(&session),
    )
    .await?;
    if let Some(token) = saved_token {
        // Only this app's saved Twitch login enters this new, isolated profile.
        // Twitch itself must issue a fresh proof and accept the catalog request.
        cdp.command(
            "Network.setCookies",
            json!({"cookies":[{"name":"auth-token","value":token,
                "domain":".twitch.tv","path":"/","secure":true,"httpOnly":false}]}),
            Some(&session),
        )
        .await?;
    }
    cdp.command(
        "Page.navigate",
        json!({"url":"https://www.twitch.tv/drops/campaigns"}),
        Some(&session),
    )
    .await?;
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    let mut integrity_requests = HashSet::new();
    let mut issued: BTreeMap<String, i64> = BTreeMap::new();
    let mut requests: BTreeMap<String, BrowserCredentials> = BTreeMap::new();
    let mut successful_responses = HashSet::new();
    let mut verified: Vec<BrowserCredentials> = Vec::new();
    while Instant::now() < deadline {
        if process.is_closed() {
            return Err("The sign-in browser was closed. Try again or use a code.".into());
        }
        let Some(event) = cdp.event().await? else {
            continue;
        };
        if event["sessionId"].as_str() != Some(&session) {
            continue;
        }
        let p = &event["params"];
        match event["method"].as_str() {
            Some("Network.requestWillBeSent") => {
                if let Some(value) = captured_request(&p["request"], &user_agent) {
                    if value.context.headers.contains_key("client-integrity") {
                        if let Some(id) = p["requestId"].as_str() {
                            requests.insert(id.into(), value);
                        }
                    }
                }
            }
            Some("Network.responseReceived")
                if p["type"] != "Preflight" && p["response"]["status"].as_f64() == Some(200.0) =>
            {
                if let Some(id) = p["requestId"].as_str() {
                    if p["response"]["url"] == INTEGRITY {
                        integrity_requests.insert(id.to_owned());
                    } else if p["response"]["url"] == GQL && requests.contains_key(id) {
                        successful_responses.insert(id.to_owned());
                    }
                }
            }
            Some("Network.loadingFinished") => {
                if let Some(id) = p["requestId"].as_str() {
                    let integrity = integrity_requests.remove(id);
                    let candidate = requests.remove(id);
                    let success = successful_responses.remove(id);
                    if !integrity && !(success && candidate.is_some()) {
                        continue;
                    }
                    if let Ok(body) = cdp
                        .command(
                            "Network.getResponseBody",
                            json!({"requestId":id}),
                            Some(&session),
                        )
                        .await
                    {
                        let text = body["body"].as_str().unwrap_or_default();
                        let bytes = if body["base64Encoded"] == true {
                            STANDARD.decode(text).unwrap_or_default()
                        } else {
                            text.as_bytes().to_vec()
                        };
                        if let Ok(data) = serde_json::from_slice::<Value>(&bytes) {
                            if integrity {
                                if let (Some(token), Some(expiration)) =
                                    (data["token"].as_str(), data["expiration"].as_i64())
                                {
                                    issued.insert(token.into(), expiration / 1000);
                                }
                            } else if successful_catalog(&data) {
                                if let Some(candidate) = candidate {
                                    verified.push(candidate);
                                }
                            }
                        }
                    }
                }
            }
            Some("Network.loadingFailed") => {
                if let Some(id) = p["requestId"].as_str() {
                    requests.remove(id);
                    successful_responses.remove(id);
                    integrity_requests.remove(id);
                }
            }
            _ => {}
        }
        if issued.len() > 64
            || integrity_requests.len() > 64
            || requests.len() > 256
            || verified.len() > 64
        {
            return Err("Browser sign-in needs to be restarted.".into());
        }
        if let Some(index) = verified.iter().position(|value| {
            value
                .context
                .headers
                .get("client-integrity")
                .is_some_and(|token| issued.contains_key(token))
        }) {
            if let Some(result) = ready_credentials(verified.swap_remove(index), &issued) {
                let _ = cdp.command("Browser.close", json!({}), None).await;
                return Ok(result);
            }
        }
    }
    Err("Browser sign-in timed out after 15 minutes. Try again.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_requires_an_accepted_catalog_and_matching_proof() {
        assert!(!successful_catalog(
            &json!({"data":{"currentUser":{"inventory":{}}}})
        ));
        assert!(!successful_catalog(
            &json!({"data":{"currentUser":{"dropCampaigns":null}}})
        ));
        assert!(!successful_catalog(
            &json!({"errors":[{"message":"failed integrity check"}],"data":{"currentUser":{"dropCampaigns":[]}}})
        ));
        assert!(successful_catalog(
            &json!([{"data":{"currentUser":{"dropCampaigns":[]}}}])
        ));
        let mut issued = BTreeMap::new();
        issued.insert("different-proof".into(), Utc::now().timestamp() + 3600);
        assert!(
            ready_credentials(captured_request(&request(), "browser").unwrap(), &issued).is_none()
        );
        issued.insert("proof".into(), Utc::now().timestamp() + 3600);
        assert!(
            ready_credentials(captured_request(&request(), "browser").unwrap(), &issued).is_some()
        );
    }
    #[test]
    fn capture_preserves_the_actual_request_user_agent() {
        let mut request = request();
        request["headers"]["User-Agent"] = json!("Chrome/reduced-version");
        let captured = captured_request(&request, "Chrome/full-version").unwrap();
        assert_eq!(captured.context.user_agent, "Chrome/reduced-version");
    }
    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "Opens a separate blank browser window and checks cleanup; no Twitch login"]
    async fn live_browser_launch_and_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let profile = tempfile::Builder::new()
            .prefix("signin-")
            .tempdir_in(root.path())
            .unwrap();
        let profile_path = profile.path().to_owned();
        let mut process =
            BrowserProcess::launch(&browser_executable().unwrap(), profile, false).unwrap();
        let endpoint = process.endpoint().await.expect("owned browser endpoint");
        let (socket, _) = connect_async(endpoint).await.expect("browser connection");
        let mut cdp = Cdp {
            socket,
            serial: 0,
            events: VecDeque::new(),
        };
        let result = cdp
            .command("Browser.getVersion", json!({}), None)
            .await
            .unwrap();
        assert!(result["userAgent"].as_str().is_some());
        drop(cdp);
        drop(process);
        assert!(!profile_path.exists(), "temporary profile must be removed");
    }
    #[test]
    fn control_endpoint_is_always_owned_loopback() {
        assert_eq!(
            startup_endpoint("Startup message\nDevTools listening on ws://127.0.0.1:9222/devtools/browser/abc-123\n", 9222).unwrap(),
            "ws://127.0.0.1:9222/devtools/browser/abc-123"
        );
        for raw in [
            "DevTools listening on ws://127.0.0.1:9223/devtools/browser/abc",
            "DevTools listening on ws://evil.test:9222/devtools/browser/abc",
            "DevTools listening on ws://127.0.0.1:9222/devtools/browser/abc?redirect=evil",
            "DevTools listening on ws://127.0.0.1:9222/devtools/browser/",
        ] {
            assert!(startup_endpoint(raw, 9222).is_none());
        }
        assert!(startup_endpoint(
            "DevTools listening on ws://127.0.0.1:0/devtools/browser/abc",
            0
        )
        .is_none());
    }
    fn request() -> Value {
        json!({"url":GQL,"headers":{"Authorization":"OAuth test_token", "Client-ID":WEB_CLIENT_ID,"Client-Integrity":"proof","X-Device-ID":"device","Cookie":"never-copy","X-Unrelated":"never-copy"}})
    }
    #[test]
    fn only_twitch_web_headers_are_captured() {
        let mut request = request();
        let captured = captured_request(&request, "browser").unwrap();
        assert_eq!(captured.token, "test_token");
        assert_eq!(captured.context.headers.len(), 2);
        request["url"] = json!("https://gql.twitch.tv.evil.test/gql");
        assert!(captured_request(&request, "browser").is_none());
        request["url"] = json!(GQL);
        request["headers"]["Client-ID"] = json!("other-client");
        assert!(captured_request(&request, "browser").is_none());
    }
    #[test]
    fn incomplete_or_expired_context_is_rejected() {
        let mut context = captured_request(&request(), "browser").unwrap().context;
        assert!(context.validate().is_err());
        context.expires_at = Utc::now().timestamp() + 3600;
        assert!(context.validate().is_ok());
        context
            .headers
            .insert("authorization".into(), "OAuth wrong".into());
        assert!(context.validate().is_err());
        context.headers.remove("authorization");
        context.user_agent = "bad\r\nheader".into();
        assert!(context.validate().is_err());
    }
    #[test]
    fn expired_proof_can_be_restored_but_cannot_be_sent() {
        let mut context = captured_request(&request(), "browser").unwrap().context;
        context.expires_at = Utc::now().timestamp() - 60;
        assert!(context.validate_structure().is_ok());
        assert!(context.needs_renewal());
        assert!(context.validate().is_err());
        context.headers.insert("cookie".into(), "invalid".into());
        assert!(context.validate_structure().is_err());
    }
    #[test]
    fn renewal_starts_before_requests_would_expire() {
        let mut context = captured_request(&request(), "browser").unwrap().context;
        context.expires_at = Utc::now().timestamp() + 120;
        assert!(context.needs_renewal());
        assert!(context.validate().is_ok());
        context.expires_at = Utc::now().timestamp() + 3600;
        assert!(!context.needs_renewal());
    }
}
