//! OAuth 2.1 (authorization-code + PKCE) for connecting to Robinhood's Agentic
//! MCP server, following the MCP authorization spec: discover the protected
//! resource's authorization server (RFC 9728 / RFC 8414), dynamically register a
//! public client (RFC 7591), run the browser authorization-code flow with PKCE
//! over a loopback redirect (RFC 8252), and persist the resulting tokens in the
//! OS keychain so the session survives restarts.
//!
//! NOTE: the live network handshake can only be exercised against Robinhood's
//! real server with a real login, so the side-effectful parts here are validated
//! by the user locally. The pure pieces (PKCE derivation, URL building, redirect
//! parsing, expiry math) are unit-tested below.

use std::time::Duration;

use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::error::{AppError, AppResult};
use crate::robinhood::ENDPOINT;

/// Keychain coordinates for the stored token blob.
const KEYRING_SERVICE: &str = "com.trendwave.app";
const KEYRING_ACCOUNT: &str = "robinhood-mcp";

/// Refresh a little before the token actually expires to avoid races.
const EXPIRY_SLACK_SECS: i64 = 60;

/// Everything we need to use and later refresh the connection. Persisted as JSON
/// in the OS keychain — never in SQLite or any plaintext file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix epoch seconds when the access token expires, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    pub token_endpoint: String,
    pub client_id: String,
    pub resource: String,
}

impl StoredAuth {
    fn is_access_valid(&self) -> bool {
        match self.expires_at {
            Some(exp) => now_secs() + EXPIRY_SLACK_SECS < exp,
            None => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Keychain persistence
// ---------------------------------------------------------------------------

fn keyring_entry() -> AppResult<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| AppError::Robinhood(format!("keychain unavailable: {e}")))
}

pub fn load_auth() -> Option<StoredAuth> {
    let entry = keyring_entry().ok()?;
    let json = entry.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

pub fn save_auth(auth: &StoredAuth) -> AppResult<()> {
    let json = serde_json::to_string(auth)?;
    keyring_entry()?
        .set_password(&json)
        .map_err(|e| AppError::Robinhood(format!("could not save credentials: {e}")))
}

pub fn clear_auth() -> AppResult<()> {
    if let Ok(entry) = keyring_entry() {
        // Treat "no entry" as already-cleared.
        match entry.delete_credential() {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(AppError::Robinhood(format!("could not clear credentials: {e}"))),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Token acquisition / refresh
// ---------------------------------------------------------------------------

/// Return a usable access token, refreshing transparently if the stored one has
/// expired. Errors with `RobinhoodNotConnected` when there is nothing stored.
pub async fn ensure_access_token(http: &reqwest::Client) -> AppResult<String> {
    let auth = load_auth().ok_or(AppError::RobinhoodNotConnected)?;
    if auth.is_access_valid() {
        return Ok(auth.access_token);
    }
    let refreshed = refresh(http, &auth).await?;
    save_auth(&refreshed)?;
    Ok(refreshed.access_token)
}

async fn refresh(http: &reqwest::Client, auth: &StoredAuth) -> AppResult<StoredAuth> {
    let refresh_token = auth
        .refresh_token
        .as_deref()
        .ok_or(AppError::RobinhoodNotConnected)?;

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", auth.client_id.as_str()),
        ("resource", auth.resource.as_str()),
    ];
    let resp = http
        .post(&auth.token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| AppError::Robinhood(format!("token refresh failed: {e}")))?;

    if !resp.status().is_success() {
        // Refresh token rejected → force a fresh authorization.
        return Err(AppError::RobinhoodNotConnected);
    }
    let token: TokenResponse = resp
        .json()
        .await
        .map_err(|e| AppError::Robinhood(format!("malformed token response: {e}")))?;

    Ok(StoredAuth {
        access_token: token.access_token,
        // Reuse the previous refresh token if the server didn't rotate it.
        refresh_token: token.refresh_token.or_else(|| auth.refresh_token.clone()),
        expires_at: token.expires_in.map(|s| now_secs() + s),
        token_endpoint: auth.token_endpoint.clone(),
        client_id: auth.client_id.clone(),
        resource: auth.resource.clone(),
    })
}

// ---------------------------------------------------------------------------
// Full authorization-code flow
// ---------------------------------------------------------------------------

/// Run the interactive connect flow. `open_url` is handed the authorization URL
/// to launch in the user's browser (the command layer wires this to Tauri's
/// opener). Returns the stored auth on success and also persists it.
pub async fn connect<F>(http: &reqwest::Client, open_url: F) -> AppResult<StoredAuth>
where
    F: FnOnce(&str) -> AppResult<()> + Send,
{
    let resource = ENDPOINT.to_string();

    let meta = discover_metadata(http, &resource).await?;
    let client_id = if let Some(existing) = load_auth().map(|a| a.client_id) {
        existing
    } else {
        String::new()
    };

    // Bind the loopback listener first so we can register/authorize with the
    // exact redirect URI (including the OS-assigned port).
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AppError::Robinhood(format!("could not open local callback port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::Robinhood(e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let client_id = if client_id.is_empty() {
        register_client(http, &meta, &redirect_uri).await?
    } else {
        client_id
    };

    let verifier = random_b64url(32);
    let challenge = pkce_challenge(&verifier);
    let state = random_b64url(16);
    let auth_url = build_authorize_url(BuildAuthorizeUrl {
        authorization_endpoint: &meta.authorization_endpoint,
        client_id: &client_id,
        redirect_uri: &redirect_uri,
        code_challenge: &challenge,
        state: &state,
        resource: &resource,
        scope: meta.scopes_supported.as_deref(),
    });

    open_url(&auth_url)?;

    let code = wait_for_code(listener, &state).await?;

    let token = exchange_code(http, ExchangeCode {
        token_endpoint: &meta.token_endpoint,
        client_id: &client_id,
        code: &code,
        redirect_uri: &redirect_uri,
        verifier: &verifier,
        resource: &resource,
    })
    .await?;

    let stored = StoredAuth {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: token.expires_in.map(|s| now_secs() + s),
        token_endpoint: meta.token_endpoint,
        client_id,
        resource,
    };
    save_auth(&stored)?;
    Ok(stored)
}

#[derive(Debug, Clone)]
struct AuthMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    scopes_supported: Option<String>,
}

/// Resolve the authorization server's endpoints. First ask the protected
/// resource which authorization server(s) it trusts (RFC 9728), then read that
/// server's metadata (RFC 8414 / OpenID discovery). Falls back to the resource
/// origin as the issuer when the resource metadata document is absent.
async fn discover_metadata(http: &reqwest::Client, resource: &str) -> AppResult<AuthMetadata> {
    let issuer = match fetch_json(http, &well_known(resource, "oauth-protected-resource")).await {
        Some(doc) => doc
            .get("authorization_servers")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| origin_of(resource)),
        None => origin_of(resource),
    };

    // Try the OAuth AS metadata doc, then the OpenID Connect variant.
    let doc = match fetch_json(http, &well_known(&issuer, "oauth-authorization-server")).await {
        Some(d) => d,
        None => fetch_json(http, &well_known(&issuer, "openid-configuration"))
            .await
            .ok_or_else(|| {
                AppError::Robinhood(format!(
                    "could not discover Robinhood's authorization server at {issuer}"
                ))
            })?,
    };

    let authorization_endpoint = doc
        .get("authorization_endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Robinhood("authorization server is missing authorization_endpoint".into()))?
        .to_string();
    let token_endpoint = doc
        .get("token_endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Robinhood("authorization server is missing token_endpoint".into()))?
        .to_string();
    let registration_endpoint = doc
        .get("registration_endpoint")
        .and_then(Value::as_str)
        .map(str::to_string);
    let scopes_supported = doc
        .get("scopes_supported")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|s| !s.is_empty());

    Ok(AuthMetadata {
        authorization_endpoint,
        token_endpoint,
        registration_endpoint,
        scopes_supported,
    })
}

/// Dynamically register a public (PKCE, no-secret) native client.
async fn register_client(
    http: &reqwest::Client,
    meta: &AuthMetadata,
    redirect_uri: &str,
) -> AppResult<String> {
    let endpoint = meta.registration_endpoint.as_deref().ok_or_else(|| {
        AppError::Robinhood(
            "Robinhood's authorization server does not support dynamic client registration.".into(),
        )
    })?;

    let body = serde_json::json!({
        "client_name": "TrendWave",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native",
    });
    let resp = http
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Robinhood(format!("client registration failed: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Robinhood(format!(
            "client registration rejected ({status}): {text}"
        )));
    }
    let doc: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Robinhood(format!("malformed registration response: {e}")))?;
    doc.get("client_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| AppError::Robinhood("registration response had no client_id".into()))
}

struct ExchangeCode<'a> {
    token_endpoint: &'a str,
    client_id: &'a str,
    code: &'a str,
    redirect_uri: &'a str,
    verifier: &'a str,
    resource: &'a str,
}

async fn exchange_code(http: &reqwest::Client, x: ExchangeCode<'_>) -> AppResult<TokenResponse> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", x.code),
        ("redirect_uri", x.redirect_uri),
        ("client_id", x.client_id),
        ("code_verifier", x.verifier),
        ("resource", x.resource),
    ];
    let resp = http
        .post(x.token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| AppError::Robinhood(format!("token exchange failed: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Robinhood(format!(
            "token exchange rejected ({status}): {text}"
        )));
    }
    resp.json()
        .await
        .map_err(|e| AppError::Robinhood(format!("malformed token response: {e}")))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// Accept exactly one loopback redirect, validate `state`, and return the `code`.
/// Times out so a user who closes the browser doesn't hang the app forever.
async fn wait_for_code(listener: TcpListener, expected_state: &str) -> AppResult<String> {
    let accept = async {
        loop {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|e| AppError::Robinhood(format!("callback listener error: {e}")))?;

            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let target = request.lines().next().unwrap_or("");

            // Ignore favicon / unrelated probes; keep waiting for the callback.
            if !target.contains("/callback") {
                let _ = respond(&mut stream, "Waiting for authorization…").await;
                continue;
            }

            let (code, state, error) = parse_redirect(target);
            if let Some(error) = error {
                let _ = respond(&mut stream, "Authorization failed. You can close this tab.").await;
                return Err(AppError::Robinhood(format!("authorization denied: {error}")));
            }
            if state.as_deref() != Some(expected_state) {
                let _ = respond(&mut stream, "Authorization state mismatch. You can close this tab.").await;
                return Err(AppError::Robinhood("authorization state mismatch".into()));
            }
            match code {
                Some(code) => {
                    let _ = respond(
                        &mut stream,
                        "TrendWave is connected to Robinhood. You can close this tab and return to the app.",
                    )
                    .await;
                    return Ok(code);
                }
                None => {
                    let _ = respond(&mut stream, "No authorization code received. You can close this tab.").await;
                    return Err(AppError::Robinhood("no authorization code in redirect".into()));
                }
            }
        }
    };

    match tokio::time::timeout(Duration::from_secs(300), accept).await {
        Ok(result) => result,
        Err(_) => Err(AppError::Robinhood(
            "timed out waiting for Robinhood authorization (5 min).".into(),
        )),
    }
}

async fn respond(stream: &mut tokio::net::TcpStream, message: &str) -> std::io::Result<()> {
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>TrendWave</title></head>\
         <body style=\"font-family:system-ui;background:#020617;color:#e2e8f0;display:flex;\
         align-items:center;justify-content:center;height:100vh;margin:0\">\
         <p style=\"font-size:16px\">{message}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

struct BuildAuthorizeUrl<'a> {
    authorization_endpoint: &'a str,
    client_id: &'a str,
    redirect_uri: &'a str,
    code_challenge: &'a str,
    state: &'a str,
    resource: &'a str,
    scope: Option<&'a str>,
}

fn build_authorize_url(p: BuildAuthorizeUrl<'_>) -> String {
    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}&resource={}",
        p.authorization_endpoint,
        pct(p.client_id),
        pct(p.redirect_uri),
        pct(p.code_challenge),
        pct(p.state),
        pct(p.resource),
    );
    if let Some(scope) = p.scope {
        url.push_str("&scope=");
        url.push_str(&pct(scope));
    }
    url
}

/// PKCE S256 challenge: base64url(sha256(verifier)), no padding.
fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn random_b64url(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Parse the `code`, `state`, and `error` params out of an HTTP request target
/// line like `GET /callback?code=abc&state=xyz HTTP/1.1`.
fn parse_redirect(request_line: &str) -> (Option<String>, Option<String>, Option<String>) {
    let path = request_line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let value = pct_decode(v);
            match k {
                "code" => code = Some(value),
                "state" => state = Some(value),
                "error" => error = Some(value),
                _ => {}
            }
        }
    }
    (code, state, error)
}

/// Minimal percent-encoding for query values (encode everything that isn't an
/// RFC 3986 unreserved character).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn well_known(base: &str, doc: &str) -> String {
    format!("{}/.well-known/{}", base.trim_end_matches('/'), doc)
}

fn origin_of(url: &str) -> String {
    // scheme://host[:port]
    if let Some(scheme_end) = url.find("://") {
        let rest = &url[scheme_end + 3..];
        let host_end = rest.find('/').unwrap_or(rest.len());
        return url[..scheme_end + 3 + host_end].to_string();
    }
    url.to_string()
}

async fn fetch_json(http: &reqwest::Client, url: &str) -> Option<Value> {
    let resp = http.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        // The canonical example from RFC 7636 Appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(pkce_challenge(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn authorize_url_has_required_pkce_params() {
        let url = build_authorize_url(BuildAuthorizeUrl {
            authorization_endpoint: "https://auth.example.com/authorize",
            client_id: "abc123",
            redirect_uri: "http://127.0.0.1:5400/callback",
            code_challenge: "chal",
            state: "st",
            resource: "https://agent.robinhood.com/mcp/trading",
            scope: Some("trading read"),
        });
        assert!(url.starts_with("https://auth.example.com/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("client_id=abc123"));
        // redirect_uri and resource must be percent-encoded.
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5400%2Fcallback"));
        assert!(url.contains("resource=https%3A%2F%2Fagent.robinhood.com%2Fmcp%2Ftrading"));
        assert!(url.contains("scope=trading%20read"));
    }

    #[test]
    fn parse_redirect_extracts_code_and_state() {
        let (code, state, error) = parse_redirect("GET /callback?code=abc123&state=xyz HTTP/1.1");
        assert_eq!(code.as_deref(), Some("abc123"));
        assert_eq!(state.as_deref(), Some("xyz"));
        assert!(error.is_none());
    }

    #[test]
    fn parse_redirect_surfaces_error_param() {
        let (code, _, error) = parse_redirect("GET /callback?error=access_denied HTTP/1.1");
        assert!(code.is_none());
        assert_eq!(error.as_deref(), Some("access_denied"));
    }

    #[test]
    fn parse_redirect_decodes_percent_escapes() {
        let (code, _, _) = parse_redirect("GET /callback?code=a%2Bb%2Fc HTTP/1.1");
        assert_eq!(code.as_deref(), Some("a+b/c"));
    }

    #[test]
    fn origin_strips_path() {
        assert_eq!(origin_of("https://agent.robinhood.com/mcp/trading"), "https://agent.robinhood.com");
        assert_eq!(well_known("https://x.com/", "oauth-authorization-server"), "https://x.com/.well-known/oauth-authorization-server");
    }

    #[test]
    fn expiry_validity_respects_slack() {
        let mut auth = StoredAuth {
            access_token: "t".into(),
            refresh_token: None,
            expires_at: Some(now_secs() + 3600),
            token_endpoint: "te".into(),
            client_id: "c".into(),
            resource: "r".into(),
        };
        assert!(auth.is_access_valid());
        auth.expires_at = Some(now_secs() + 5); // within slack → treated as expired
        assert!(!auth.is_access_valid());
        auth.expires_at = None; // unknown expiry → assume usable, let a 401 force reconnect
        assert!(auth.is_access_valid());
    }
}
