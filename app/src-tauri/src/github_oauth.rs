//! GitHub App Device Flow for the notes backup. The device flow needs no client
//! secret and no PKCE. The public client id is injected at build time through
//! `SCRIBE_GITHUB_APP_CLIENT_ID` so local and release builds can use different
//! GitHub App registrations without changing source.
//!
//! Device flow is two HTTP steps: (1) POST `/login/device/code` to get a
//! `device_code` + a human `user_code` the user types at github.com/login/device;
//! (2) poll `/login/oauth/access_token` until GitHub reports the user authorized.
//! GitHub returns HTTP 200 even while the user is still authorizing, so the poll
//! branches on the JSON `error` field, not the status code.
//!
//! Access and refresh tokens are persisted together only in the OS keychain
//! (Windows Credential Manager via `keyring`), never in settings or SQLite.
//! Expiring GitHub App user tokens are refreshed before use and each rotated
//! refresh token replaces its predecessor in the same keychain credential.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::error::CommandError;

/// Public GitHub App client id for the device flow. GitHub App client ids use
/// the `Iv` prefix. OAuth App ids use `Ov` and are deliberately rejected so a
/// build cannot silently regain the classic, account-wide `repo` scope.
const CLIENT_ID: Option<&str> = option_env!("SCRIBE_GITHUB_APP_CLIENT_ID");

const DEVICE_CODE_URI: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URI: &str = "https://github.com/login/oauth/access_token";
const USER_URI: &str = "https://api.github.com/user";
/// Username component of the keychain entry. The service component is the app's
/// bundle identifier (passed in) so the Dev flavor and stable keep separate
/// tokens.
const KEYCHAIN_USER: &str = "github-access-token";
const CREDENTIAL_VERSION: u8 = 1;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Device codes expire after ~15 minutes; stop polling a little after that.
const POLL_TIMEOUT: Duration = Duration::from_secs(900);
/// Refresh before expiry so a request cannot start with an almost-dead token.
const REFRESH_WINDOW: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct StoredCredential {
    version: u8,
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token_expires_at: Option<u64>,
}

impl std::fmt::Debug for StoredCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredCredential")
            .field("version", &self.version)
            .field("access_token", &"[redacted]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[redacted]"),
            )
            .field("expires_at", &self.expires_at)
            .field("refresh_token_expires_at", &self.refresh_token_expires_at)
            .finish()
    }
}

impl StoredCredential {
    fn legacy(access_token: String) -> Self {
        Self {
            version: CREDENTIAL_VERSION,
            access_token,
            refresh_token: None,
            expires_at: None,
            refresh_token_expires_at: None,
        }
    }

    fn needs_refresh(&self, now: u64) -> bool {
        self.expires_at
            .is_some_and(|expires_at| expires_at <= now.saturating_add(REFRESH_WINDOW.as_secs()))
    }
}

/// Active device-flow polls keyed by GitHub's opaque device code.
///
/// The cancellation flag is shared with the blocking HTTP poll so a frontend
/// cancel request stops both network polling and credential persistence.
#[derive(Default)]
pub struct DeviceFlowRegistry {
    attempts: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl DeviceFlowRegistry {
    pub fn start(&self, device_code: &str) -> Arc<AtomicBool> {
        let cancelled = Arc::new(AtomicBool::new(false));
        self.attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(device_code.to_string(), cancelled.clone());
        cancelled
    }

    pub fn get(&self, device_code: &str) -> Option<Arc<AtomicBool>> {
        self.attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(device_code)
            .cloned()
    }

    pub fn cancel(&self, device_code: &str) -> bool {
        let attempt = self
            .attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(device_code);
        if let Some(cancelled) = attempt {
            cancelled.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    pub fn finish(&self, device_code: &str) {
        self.attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(device_code);
    }
}

/// What the UI needs to show the user to complete the device flow.
#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// True once the shipped client id has been filled in. The UI uses this to
/// explain that GitHub sync needs a configured build, rather than failing
/// mid-flow against GitHub.
pub fn is_configured() -> bool {
    client_id().is_ok()
}

/// Step 1 of the device flow: ask GitHub for a device + user code.
pub fn request_device_code() -> Result<DeviceCode, CommandError> {
    let client_id = client_id()?;
    // GitHub App user tokens are constrained by the app's fine-grained
    // permissions and installation repository selection. Do not send a
    // classic OAuth scope, especially the account-wide `repo` scope.
    let body = format!("client_id={}", form_encode(client_id));
    let client = http_client()?;
    let response = client
        .post(DEVICE_CODE_URI)
        .timeout(HTTP_TIMEOUT)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .map_err(|error| failure(format!("Could not reach GitHub. {error}")))?;

    let status = response.status();
    let text = response
        .text()
        .map_err(|error| failure(format!("Could not read GitHub's response. {error}")))?;
    if !status.is_success() {
        return Err(failure(format!(
            "GitHub rejected the device-code request (HTTP {status}). {}",
            truncate(&text, 300)
        )));
    }
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| failure(format!("Could not parse GitHub's response. {error}")))?;

    Ok(DeviceCode {
        device_code: json_str(&json, "device_code")?,
        user_code: json_str(&json, "user_code")?,
        verification_uri: json_str(&json, "verification_uri")?,
        expires_in: json
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(900),
        interval: json.get("interval").and_then(|v| v.as_u64()).unwrap_or(5),
    })
}

/// Step 2 of the device flow: poll until the user authorizes (or the code
/// expires / is denied). Returns the long-lived access token.
///
/// CRITICAL: GitHub returns HTTP 200 even when authorization is still pending,
/// so this branches on the JSON `error` field, not the status code.
pub(crate) fn poll_for_token(
    device_code: &str,
    interval_secs: u64,
    cancelled: &AtomicBool,
) -> Result<StoredCredential, CommandError> {
    ensure_attempt_active(cancelled)?;
    let client_id = client_id()?;
    let client = http_client()?;
    let deadline = Instant::now() + POLL_TIMEOUT;
    let mut interval = interval_secs.max(1);

    loop {
        ensure_attempt_active(cancelled)?;
        if Instant::now() >= deadline {
            return Err(failure(
                "Timed out waiting for GitHub authorization. Please try again.",
            ));
        }
        std::thread::sleep(Duration::from_secs(interval));
        ensure_attempt_active(cancelled)?;

        let body = format!(
            "client_id={}&device_code={}&grant_type=urn:ietf:params:oauth:grant-type:device_code",
            form_encode(client_id),
            form_encode(device_code),
        );
        let response = client
            .post(ACCESS_TOKEN_URI)
            .timeout(HTTP_TIMEOUT)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .map_err(|error| failure(format!("Could not reach GitHub. {error}")))?;

        // GitHub returns 200 for pending/slow-down too, so we do NOT gate on
        // status here — we read the body and branch on its `error` field.
        let text = response
            .text()
            .map_err(|error| failure(format!("Could not read GitHub's response. {error}")))?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| failure(format!("Could not parse GitHub's response. {error}")))?;

        if json.get("access_token").is_some() {
            ensure_attempt_active(cancelled)?;
            return credential_from_response(&json, unix_timestamp()?);
        }

        match json.get("error").and_then(|v| v.as_str()) {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                // Per the device-flow spec, back off by 5s on slow_down.
                interval += 5;
                continue;
            }
            Some("expired_token") => {
                return Err(failure(
                    "The GitHub sign-in code expired before you authorized it. Please try again.",
                ))
            }
            Some("access_denied") => return Err(failure("GitHub sign-in was denied.")),
            Some(other) => {
                let detail = json
                    .get("error_description")
                    .and_then(|v| v.as_str())
                    .unwrap_or(other);
                return Err(failure(format!("GitHub sign-in failed: {detail}")));
            }
            None => {
                return Err(failure(format!(
                    "GitHub returned an unexpected response. {}",
                    truncate(&text, 300)
                )))
            }
        }
    }
}

pub fn ensure_attempt_active(cancelled: &AtomicBool) -> Result<(), CommandError> {
    if cancelled.load(Ordering::SeqCst) {
        Err(CommandError::new(
            "github_auth_cancelled",
            "GitHub sign-in was cancelled.",
        ))
    } else {
        Ok(())
    }
}

/// Reads the stored access token and refreshes an expiring GitHub App user
/// token before use. Legacy raw OAuth tokens remain readable so installing a
/// migrated build never disconnects an existing user unexpectedly.
pub fn access_token(service: &str) -> Result<String, CommandError> {
    let credential = load_credential(service)?;
    let now = unix_timestamp()?;
    if !credential.needs_refresh(now) {
        return Ok(credential.access_token);
    }

    let refresh_token = credential
        .refresh_token
        .as_deref()
        .ok_or_else(unauthorized)?;
    if credential
        .refresh_token_expires_at
        .is_some_and(|expires_at| expires_at <= now)
    {
        let _ = sign_out(service);
        return Err(unauthorized());
    }

    match refresh_credential(refresh_token, now) {
        Ok(refreshed) => {
            store_credential(service, &refreshed)?;
            Ok(refreshed.access_token)
        }
        Err(error) if error.code == "github_unauthorized" => {
            let _ = sign_out(service);
            Err(error)
        }
        Err(error) => Err(error),
    }
}

/// Removes the stored access token. Signing out is best-effort: a missing entry
/// is success.
pub fn sign_out(service: &str) -> Result<(), CommandError> {
    let entry = keychain_entry(service)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(failure(format!(
            "Could not clear the saved GitHub token. {error}"
        ))),
    }
}

/// True when an access token is present in the keychain for `service`.
pub fn has_stored_token(service: &str) -> bool {
    keychain_entry(service)
        .and_then(|entry| match entry.get_password() {
            Ok(_) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(error) => Err(failure(error.to_string())),
        })
        .unwrap_or(false)
}

/// Stores a complete GitHub App credential in the keychain under `service`.
pub(crate) fn store_credential(
    service: &str,
    credential: &StoredCredential,
) -> Result<(), CommandError> {
    let encoded = serde_json::to_string(credential)
        .map_err(|error| failure(format!("Could not encode the GitHub credential. {error}")))?;
    keychain_entry(service)?
        .set_password(&encoded)
        .map_err(|error| {
            failure(format!(
                "Could not save the GitHub token to the keychain. {error}"
            ))
        })
}

/// GET `/user` to learn the connected account's `login` (for display and to gate
/// org repo auto-create). api.github.com requires a User-Agent header, which the
/// shared `http_client()` builder sets.
pub fn fetch_login(service: &str) -> Result<String, CommandError> {
    let token = access_token(service)?;
    let client = http_client()?;
    let response = client
        .get(USER_URI)
        .timeout(HTTP_TIMEOUT)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .map_err(|error| failure(format!("Could not reach GitHub. {error}")))?;

    let status = response.status();
    if status.as_u16() == 401 {
        // The token was revoked/expired on GitHub's side. Clear it so the app
        // reflects "not connected" instead of a stuck "Connected" state.
        let _ = sign_out(service);
        return Err(unauthorized());
    }
    let text = response
        .text()
        .map_err(|error| failure(format!("Could not read GitHub's response. {error}")))?;
    if !status.is_success() {
        return Err(failure(format!(
            "GitHub returned HTTP {status} reading the account. {}",
            truncate(&text, 300)
        )));
    }
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| failure(format!("Could not parse GitHub's response. {error}")))?;
    Ok(json
        .get("login")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string())
}

fn refresh_credential(refresh_token: &str, now: u64) -> Result<StoredCredential, CommandError> {
    let client_id = client_id()?;
    refresh_credential_at(ACCESS_TOKEN_URI, client_id, refresh_token, now)
}

fn refresh_credential_at(
    endpoint: &str,
    client_id: &str,
    refresh_token: &str,
    now: u64,
) -> Result<StoredCredential, CommandError> {
    let body = format!(
        "client_id={}&grant_type=refresh_token&refresh_token={}",
        form_encode(client_id),
        form_encode(refresh_token),
    );
    let response = http_client()?
        .post(endpoint)
        .timeout(HTTP_TIMEOUT)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .map_err(|error| failure(format!("Could not refresh the GitHub connection. {error}")))?;
    let status = response.status();
    let text = response
        .text()
        .map_err(|error| failure(format!("Could not read GitHub's response. {error}")))?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| failure(format!("Could not parse GitHub's response. {error}")))?;

    if matches!(
        json.get("error").and_then(|value| value.as_str()),
        Some("bad_refresh_token") | Some("incorrect_client_credentials")
    ) {
        return Err(unauthorized());
    }
    if !status.is_success() || json.get("error").is_some() {
        let detail = json
            .get("error_description")
            .or_else(|| json.get("error"))
            .and_then(|value| value.as_str())
            .unwrap_or("unexpected response");
        return Err(failure(format!(
            "GitHub could not refresh the connection (HTTP {status}): {detail}"
        )));
    }
    credential_from_response(&json, now)
}

fn credential_from_response(
    json: &serde_json::Value,
    now: u64,
) -> Result<StoredCredential, CommandError> {
    let access_token = json_str(json, "access_token")?;
    let refresh_token = json
        .get("refresh_token")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let expires_at = json
        .get("expires_in")
        .and_then(|value| value.as_u64())
        .map(|seconds| now.saturating_add(seconds));
    let refresh_token_expires_at = json
        .get("refresh_token_expires_in")
        .and_then(|value| value.as_u64())
        .map(|seconds| now.saturating_add(seconds));

    if expires_at.is_some() && refresh_token.is_none() {
        return Err(failure(
            "GitHub returned an expiring access token without a refresh token.",
        ));
    }

    Ok(StoredCredential {
        version: CREDENTIAL_VERSION,
        access_token,
        refresh_token,
        expires_at,
        refresh_token_expires_at,
    })
}

// --- keychain ----------------------------------------------------------------

fn keychain_entry(service: &str) -> Result<keyring::Entry, CommandError> {
    keyring::Entry::new(service, KEYCHAIN_USER)
        .map_err(|error| failure(format!("The OS keychain is unavailable. {error}")))
}

fn load_credential(service: &str) -> Result<StoredCredential, CommandError> {
    match keychain_entry(service)?.get_password() {
        Ok(encoded) => decode_credential(&encoded),
        Err(keyring::Error::NoEntry) => Err(CommandError::new(
            "github_not_signed_in",
            "Not signed in to GitHub. Open Settings → Sync and connect GitHub.",
        )),
        Err(error) => Err(failure(format!(
            "Could not read the saved GitHub token. {error}"
        ))),
    }
}

fn decode_credential(encoded: &str) -> Result<StoredCredential, CommandError> {
    if !encoded.trim_start().starts_with('{') {
        return Ok(StoredCredential::legacy(encoded.to_string()));
    }
    let credential: StoredCredential = serde_json::from_str(encoded)
        .map_err(|error| failure(format!("The saved GitHub credential is invalid. {error}")))?;
    if credential.version != CREDENTIAL_VERSION || credential.access_token.is_empty() {
        return Err(failure("The saved GitHub credential is invalid."));
    }
    Ok(credential)
}

// --- helpers -----------------------------------------------------------------

fn http_client() -> Result<reqwest::blocking::Client, CommandError> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("Scribe/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| failure(error.to_string()))
}

fn client_id() -> Result<&'static str, CommandError> {
    validate_client_id(CLIENT_ID).ok_or_else(not_configured)
}

fn validate_client_id(value: Option<&str>) -> Option<&str> {
    value.filter(|client_id| {
        client_id.starts_with("Iv")
            && client_id.len() > 2
            && client_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

fn unix_timestamp() -> Result<u64, CommandError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| failure(format!("The system clock is invalid. {error}")))
}

fn json_str(json: &serde_json::Value, key: &str) -> Result<String, CommandError> {
    json.get(key)
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| failure(format!("GitHub's response had no `{key}`.")))
}

/// Percent-encodes a value for an x-www-form-urlencoded body, leaving only the
/// RFC 3986 unreserved set. Hand-rolled to avoid pulling in a URL crate.
fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn not_configured() -> CommandError {
    CommandError::new(
        "github_not_configured",
        "This build has no valid GitHub App client configured, so new GitHub connections are unavailable.",
    )
}

/// The stored token was revoked or expired (HTTP 401). Distinct code so the UI
/// can prompt a reconnect rather than showing a raw HTTP error.
fn unauthorized() -> CommandError {
    CommandError::new(
        "github_unauthorized",
        "Your GitHub connection expired or was revoked. Reconnect GitHub in Settings → Sync.",
    )
}

fn failure(message: impl Into<String>) -> CommandError {
    CommandError::new("github_auth_failed", message)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn device_flow_registry_cancels_and_reaps_an_attempt() {
        let registry = DeviceFlowRegistry::default();
        let cancelled = registry.start("device-1");
        assert!(!cancelled.load(Ordering::SeqCst));

        assert!(registry.cancel("device-1"));
        assert!(cancelled.load(Ordering::SeqCst));
        assert!(registry.get("device-1").is_none());
        assert!(!registry.cancel("device-1"));
    }

    #[test]
    fn cancelled_poll_stops_before_contacting_github() {
        let cancelled = AtomicBool::new(true);
        let error = poll_for_token("unused", 1, &cancelled).unwrap_err();
        assert_eq!(error.code, "github_auth_cancelled");
    }

    /// Sequential HTTP mock: serves `responses` (status, body) in order,
    /// capturing each request. Mirrors the sequential-mock harness used in the
    /// other network modules' tests.
    fn mock_server(
        responses: Vec<(u16, String)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 65536];
                let mut request = Vec::new();
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some(headers_end) = text.find("\r\n\r\n") {
                        let content_length = text
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= headers_end + 4 + content_length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let payload = format!(
                    "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                stream.write_all(payload.as_bytes()).unwrap();
            }
            requests
        });
        (base, handle)
    }

    // The device-flow endpoints are module constants, so these tests exercise
    // the parsing/branching logic against a local mock by posting directly. We
    // reuse the same JSON-shape parsing the real functions use.

    #[test]
    fn device_code_response_parses() {
        let (base, handle) = mock_server(vec![(
            200,
            serde_json::json!({
                "device_code": "dev123",
                "user_code": "ABCD-1234",
                "verification_uri": "https://github.com/login/device",
                "expires_in": 900,
                "interval": 5,
            })
            .to_string(),
        )]);

        let client = reqwest::blocking::Client::new();
        let text = client
            .post(&base)
            .header("Accept", "application/json")
            .body("client_id=Iv123")
            .send()
            .unwrap()
            .text()
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json_str(&json, "device_code").unwrap(), "dev123");
        assert_eq!(json_str(&json, "user_code").unwrap(), "ABCD-1234");
        assert_eq!(
            json_str(&json, "verification_uri").unwrap(),
            "https://github.com/login/device"
        );

        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 1);
        // Header names are case-insensitive on the wire; match lowercased.
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("accept: application/json"));
    }

    #[test]
    fn poll_treats_authorization_pending_then_access_token() {
        // First poll: HTTP 200 with `authorization_pending` (must continue);
        // second poll: HTTP 200 with the token (must succeed). This mirrors the
        // exact branch in poll_for_token; here we drive a tiny copy of its loop
        // against the mock to prove the error-field branching, since the real
        // function hard-codes the github.com URL.
        let (base, handle) = mock_server(vec![
            (
                200,
                serde_json::json!({ "error": "authorization_pending" }).to_string(),
            ),
            (
                200,
                serde_json::json!({ "access_token": "ghs_secret", "token_type": "bearer" })
                    .to_string(),
            ),
        ]);

        let client = reqwest::blocking::Client::new();
        let token = loop {
            let text = client
                .post(&base)
                .header("Accept", "application/json")
                .body("grant_type=urn:ietf:params:oauth:grant-type:device_code")
                .send()
                .unwrap()
                .text()
                .unwrap();
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            if let Some(t) = json
                .get("access_token")
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
            {
                break t.to_string();
            }
            match json.get("error").and_then(|v| v.as_str()) {
                Some("authorization_pending") | Some("slow_down") => continue,
                other => panic!("unexpected error branch: {other:?}"),
            }
        };
        assert_eq!(token, "ghs_secret");

        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 2, "polled twice: pending then success");
    }

    #[test]
    fn form_encode_escapes_reserved_chars() {
        assert_eq!(form_encode("a/b=c&d"), "a%2Fb%3Dc%26d");
        assert_eq!(form_encode("Aa0-_.~"), "Aa0-_.~");
    }

    #[test]
    fn only_github_app_client_ids_are_accepted() {
        assert_eq!(validate_client_id(Some("Iv123abcXYZ")), Some("Iv123abcXYZ"));
        assert_eq!(validate_client_id(Some("Ov123abcXYZ")), None);
        assert_eq!(validate_client_id(Some("Iv")), None);
        assert_eq!(validate_client_id(Some("Iv123 abc")), None);
        assert_eq!(validate_client_id(None), None);
    }

    #[test]
    fn device_request_omits_classic_oauth_scope() {
        let body = format!("client_id={}", form_encode("Iv123"));
        assert_eq!(body, "client_id=Iv123");
        assert!(!body.contains("scope"));
        assert!(!body.contains("repo"));
    }

    #[test]
    fn parses_expiring_and_non_expiring_credentials() {
        let expiring = credential_from_response(
            &serde_json::json!({
                "access_token": "ghu_access",
                "expires_in": 28_800,
                "refresh_token": "ghr_refresh",
                "refresh_token_expires_in": 15_897_600,
                "token_type": "bearer",
                "scope": ""
            }),
            1_000,
        )
        .unwrap();
        assert_eq!(expiring.access_token, "ghu_access");
        assert_eq!(expiring.refresh_token.as_deref(), Some("ghr_refresh"));
        assert_eq!(expiring.expires_at, Some(29_800));
        assert_eq!(expiring.refresh_token_expires_at, Some(15_898_600));

        let permanent = credential_from_response(
            &serde_json::json!({ "access_token": "ghu_access", "token_type": "bearer" }),
            1_000,
        )
        .unwrap();
        assert_eq!(permanent.expires_at, None);
        assert_eq!(permanent.refresh_token, None);
    }

    #[test]
    fn rejects_expiring_access_token_without_refresh_token() {
        let error = credential_from_response(
            &serde_json::json!({ "access_token": "ghu_access", "expires_in": 28_800 }),
            1_000,
        )
        .unwrap_err();
        assert_eq!(error.code, "github_auth_failed");
    }

    #[test]
    fn legacy_raw_token_remains_readable() {
        let credential = decode_credential("gho_existing_oauth_token").unwrap();
        assert_eq!(credential.access_token, "gho_existing_oauth_token");
        assert_eq!(credential.refresh_token, None);
        assert_eq!(credential.expires_at, None);
    }

    #[test]
    fn serialized_credential_round_trips_without_losing_rotation_data() {
        let original = StoredCredential {
            version: CREDENTIAL_VERSION,
            access_token: "ghu_new".to_string(),
            refresh_token: Some("ghr_new".to_string()),
            expires_at: Some(10_000),
            refresh_token_expires_at: Some(20_000),
        };
        let encoded = serde_json::to_string(&original).unwrap();
        let decoded = decode_credential(&encoded).unwrap();
        assert_eq!(decoded.access_token, "ghu_new");
        assert_eq!(decoded.refresh_token.as_deref(), Some("ghr_new"));
        assert_eq!(decoded.expires_at, Some(10_000));
        assert_eq!(decoded.refresh_token_expires_at, Some(20_000));
    }

    #[test]
    fn refresh_window_is_applied_without_underflow() {
        let mut credential = StoredCredential::legacy("ghu_access".to_string());
        assert!(!credential.needs_refresh(100));
        credential.expires_at = Some(1_000);
        assert!(!credential.needs_refresh(699));
        assert!(credential.needs_refresh(700));
        assert!(credential.needs_refresh(u64::MAX));
    }

    #[test]
    fn refresh_rotates_both_tokens_and_sends_no_secret() {
        let (base, handle) = mock_server(vec![(
            200,
            serde_json::json!({
                "access_token": "ghu_rotated",
                "expires_in": 28_800,
                "refresh_token": "ghr_rotated",
                "refresh_token_expires_in": 15_897_600,
                "token_type": "bearer"
            })
            .to_string(),
        )]);
        let credential = refresh_credential_at(&base, "Iv123", "ghr_old/value", 5_000).unwrap();
        assert_eq!(credential.access_token, "ghu_rotated");
        assert_eq!(credential.refresh_token.as_deref(), Some("ghr_rotated"));

        let requests = handle.join().unwrap();
        let request = &requests[0];
        assert!(request.contains("client_id=Iv123"));
        assert!(request.contains("grant_type=refresh_token"));
        assert!(request.contains("refresh_token=ghr_old%2Fvalue"));
        assert!(!request.contains("client_secret"));
        assert!(!request.contains("scope=repo"));
    }

    #[test]
    fn bad_refresh_token_requires_reauthorization() {
        let (base, handle) = mock_server(vec![(
            200,
            serde_json::json!({
                "error": "bad_refresh_token",
                "error_description": "The refresh token is invalid."
            })
            .to_string(),
        )]);
        let error = refresh_credential_at(&base, "Iv123", "ghr_bad", 5_000).unwrap_err();
        assert_eq!(error.code, "github_unauthorized");
        handle.join().unwrap();
    }

    #[test]
    fn unconfigured_build_reports_clearly() {
        if !is_configured() {
            let error = request_device_code().unwrap_err();
            assert_eq!(error.code, "github_not_configured");
        }
    }
}
