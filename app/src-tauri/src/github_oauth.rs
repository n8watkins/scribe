//! GitHub OAuth Device Flow for the notes backup. The device flow needs NO
//! client secret and NO PKCE — the client id is a public value safe to commit,
//! so there is no secrets file and no build.rs injection.
//!
//! Device flow is two HTTP steps: (1) POST `/login/device/code` to get a
//! `device_code` + a human `user_code` the user types at github.com/login/device;
//! (2) poll `/login/oauth/access_token` until GitHub reports the user authorized.
//! GitHub returns HTTP 200 even while the user is still authorizing, so the poll
//! branches on the JSON `error` field, not the status code.
//!
//! Only the long-lived `access_token` is persisted, and only in the OS keychain
//! (Windows Credential Manager via `keyring`) — never in the settings JSON or
//! the SQLite DB. GitHub OAuth-App device-flow tokens do not expire/rotate by
//! default, so there is no refresh-token dance.

use std::time::{Duration, Instant};

use crate::error::CommandError;

/// Public GitHub OAuth App client id for the device flow. Safe to commit (the
/// device flow uses no secret and no PKCE). The OAuth App must have "Enable
/// Device Flow" checked in its settings, or `/login/device/code` returns
/// `device_flow_disabled`. Until the real id is filled in this stays a
/// `REPLACE_…` placeholder and `is_configured()` is false.
pub const CLIENT_ID: &str = "REPLACE_WITH_GITHUB_OAUTH_APP_CLIENT_ID";

const DEVICE_CODE_URI: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URI: &str = "https://github.com/login/oauth/access_token";
const USER_URI: &str = "https://api.github.com/user";
/// `repo` (full) — required to create the PRIVATE backup repo and write
/// Contents-API commits. `public_repo` is insufficient for private repos.
const SCOPE: &str = "repo";

/// Username component of the keychain entry. The service component is the app's
/// bundle identifier (passed in) so the Dev flavor and stable keep separate
/// tokens.
const KEYCHAIN_USER: &str = "github-access-token";

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Device codes expire after ~15 minutes; stop polling a little after that.
const POLL_TIMEOUT: Duration = Duration::from_secs(900);

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
    !CLIENT_ID.starts_with("REPLACE_")
}

/// Step 1 of the device flow: ask GitHub for a device + user code.
pub fn request_device_code() -> Result<DeviceCode, CommandError> {
    if !is_configured() {
        return Err(not_configured());
    }
    let body = format!(
        "client_id={}&scope={}",
        form_encode(CLIENT_ID),
        form_encode(SCOPE),
    );
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
        expires_in: json.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(900),
        interval: json.get("interval").and_then(|v| v.as_u64()).unwrap_or(5),
    })
}

/// Step 2 of the device flow: poll until the user authorizes (or the code
/// expires / is denied). Returns the long-lived access token.
///
/// CRITICAL: GitHub returns HTTP 200 even when authorization is still pending,
/// so this branches on the JSON `error` field, not the status code.
pub fn poll_for_token(device_code: &str, interval_secs: u64) -> Result<String, CommandError> {
    let client = http_client()?;
    let deadline = Instant::now() + POLL_TIMEOUT;
    let mut interval = interval_secs.max(1);

    loop {
        if Instant::now() >= deadline {
            return Err(failure(
                "Timed out waiting for GitHub authorization. Please try again.",
            ));
        }
        std::thread::sleep(Duration::from_secs(interval));

        let body = format!(
            "client_id={}&device_code={}&grant_type=urn:ietf:params:oauth:grant-type:device_code",
            form_encode(CLIENT_ID),
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

        if let Some(token) = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
        {
            return Ok(token.to_string());
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
            Some("access_denied") => {
                return Err(failure("GitHub sign-in was denied."))
            }
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

/// Reads the stored access token (no refresh — GitHub tokens are long-lived).
/// This is the entry point every backup operation uses.
pub fn access_token(service: &str) -> Result<String, CommandError> {
    load_token(service)
}

/// Removes the stored access token. Signing out is best-effort: a missing entry
/// is success.
pub fn sign_out(service: &str) -> Result<(), CommandError> {
    let entry = keychain_entry(service)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(failure(format!("Could not clear the saved GitHub token. {error}"))),
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

/// Stores the access token in the keychain under `service`.
pub fn store_token(service: &str, token: &str) -> Result<(), CommandError> {
    keychain_entry(service)?
        .set_password(token)
        .map_err(|error| failure(format!("Could not save the GitHub token to the keychain. {error}")))
}

/// GET `/user` to learn the connected account's `login` (for display and to gate
/// org repo auto-create). api.github.com requires a User-Agent header, which the
/// shared `http_client()` builder sets.
pub fn fetch_login(service: &str) -> Result<String, CommandError> {
    let token = load_token(service)?;
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

// --- keychain ----------------------------------------------------------------

fn keychain_entry(service: &str) -> Result<keyring::Entry, CommandError> {
    keyring::Entry::new(service, KEYCHAIN_USER)
        .map_err(|error| failure(format!("The OS keychain is unavailable. {error}")))
}

fn load_token(service: &str) -> Result<String, CommandError> {
    match keychain_entry(service)?.get_password() {
        Ok(token) => Ok(token),
        Err(keyring::Error::NoEntry) => Err(CommandError::new(
            "github_not_signed_in",
            "Not signed in to GitHub. Open Settings → Sync and connect GitHub.",
        )),
        Err(error) => Err(failure(format!("Could not read the saved GitHub token. {error}"))),
    }
}

// --- helpers -----------------------------------------------------------------

fn http_client() -> Result<reqwest::blocking::Client, CommandError> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("Scribe/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| failure(error.to_string()))
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
        "This build has no GitHub OAuth client configured, so GitHub sync is unavailable.",
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

    /// Sequential HTTP mock: serves `responses` (status, body) in order,
    /// capturing each request. Mirrors the sequential-mock harness used in the
    /// other network modules' tests.
    fn mock_server(responses: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
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
            .body("client_id=x&scope=repo")
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
            (200, serde_json::json!({ "error": "authorization_pending" }).to_string()),
            (200, serde_json::json!({ "access_token": "ghs_secret", "token_type": "bearer" }).to_string()),
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
    fn unconfigured_build_reports_clearly() {
        if !is_configured() {
            let error = request_device_code().unwrap_err();
            assert_eq!(error.code, "github_not_configured");
        }
    }
}
