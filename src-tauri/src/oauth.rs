//! Generic OAuth2 Authorization Code + PKCE flow for connecting a
//! self-registered LLM provider app, plus secure token storage via the OS
//! keychain (Keychain / libsecret / Credential Manager, through `keyring`).
//!
//! There is no pre-registered autocut OAuth client for any vendor — doing
//! that for real would mean autocut owning a published app with each
//! provider. Instead the user plugs in their own OAuth app's `client_id`,
//! `auth_url` and `token_url` in Settings; this module runs the flow against
//! whatever they configured. That's the only honest way to ship this without
//! fabricating a vendor relationship that doesn't exist.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::settings::LlmSettings;

/// Fixed loopback port for the redirect URI. Desktop OAuth apps (gh, aws-cli,
/// gcloud) all use this pattern: the user's own OAuth app must list this
/// exact redirect URI (`http://127.0.0.1:<PORT>/callback`) as allowed.
pub const REDIRECT_PORT: u16 = 48219;

pub fn redirect_uri() -> String {
    format!("http://127.0.0.1:{REDIRECT_PORT}/callback")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

const KEYRING_SERVICE: &str = "autocut-llm";
const KEYRING_USER: &str = "default";

// Separate entry from the OAuth token above — a plain bearer API key for a
// custom OpenAI-compatible endpoint (no client_id/auth_url/token_url dance),
// e.g. a self-hosted gateway. Kept in its own keychain slot rather than
// folded into `TokenResponse` so the two auth modes never collide: setting
// one doesn't silently clobber the other, and `llm_status` can report which
// one is actually active.
const API_KEY_KEYRING_SERVICE: &str = "autocut-llm-apikey";
const API_KEY_KEYRING_USER: &str = "default";

/// Random, URL-safe PKCE verifier (43-128 chars per RFC 7636; we use 64) and
/// its S256 challenge.
pub fn generate_pkce() -> (String, String) {
    let verifier: String = {
        let mut rng = rand::thread_rng();
        (0..64)
            .map(|_| {
                const CHARS: &[u8] =
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
                CHARS[rng.gen_range(0..CHARS.len())] as char
            })
            .collect()
    };
    let challenge = pkce_challenge(&verifier);
    (verifier, challenge)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub fn random_state() -> String {
    let mut rng = rand::thread_rng();
    (0..24)
        .map(|_| {
            const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
            CHARS[rng.gen_range(0..CHARS.len())] as char
        })
        .collect()
}

pub fn authorize_url(settings: &LlmSettings, state: &str, challenge: &str) -> Result<String> {
    if settings.auth_url.is_empty() || settings.client_id.is_empty() {
        return Err(anyhow!(
            "LLM provider isn't configured yet — set client id and authorization URL in Settings"
        ));
    }
    let sep = if settings.auth_url.contains('?') {
        '&'
    } else {
        '?'
    };
    Ok(format!(
        "{base}{sep}response_type=code&client_id={client_id}&redirect_uri={redirect}&scope={scope}&state={state}&code_challenge={challenge}&code_challenge_method=S256",
        base = settings.auth_url,
        sep = sep,
        client_id = urlencode(&settings.client_id),
        redirect = urlencode(&redirect_uri()),
        scope = urlencode(&settings.scope),
        state = urlencode(state),
        challenge = urlencode(challenge),
    ))
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Block waiting for the OAuth redirect on the loopback listener, polling
/// `cancel` every 200ms so a "batalkan" button in the UI can abort a stuck
/// login. Returns the `code` and `state` query params from the first
/// well-formed `GET /callback?...` request.
pub fn await_redirect(expected_state: &str, cancel: Arc<AtomicBool>) -> Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
        .with_context(|| format!("binding loopback redirect listener on port {REDIRECT_PORT}"))?;
    listener
        .set_nonblocking(true)
        .context("setting listener non-blocking")?;

    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(anyhow!("login cancelled"));
        }
        match listener.accept() {
            Ok((mut stream, _addr)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut reader = BufReader::new(&stream);
                let mut request_line = String::new();
                reader
                    .read_line(&mut request_line)
                    .context("reading OAuth redirect request")?;
                let query = request_line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|path| path.split_once('?'))
                    .map(|(_, q)| q.to_string())
                    .unwrap_or_default();

                let params = parse_query(&query);
                let body = "<html><body style=\"font-family:sans-serif;padding:40px;\">\
                    autocut terhubung. Jendela ini boleh ditutup.</body></html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());

                let state = params.get("state").cloned().unwrap_or_default();
                if state != expected_state {
                    return Err(anyhow!("OAuth state mismatch — possible CSRF, aborting"));
                }
                if let Some(err) = params.get("error") {
                    return Err(anyhow!("provider returned an error: {err}"));
                }
                return params
                    .get("code")
                    .cloned()
                    .ok_or_else(|| anyhow!("redirect had no authorization code"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(anyhow!("loopback listener error: {e}")),
        }
    }
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_string(), urldecode(v)))
        .collect()
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Exchange an authorization code for tokens. Uses `ureq` directly rather
/// than a dedicated OAuth crate so the whole flow stays inspectable in one
/// module instead of behind a generic client abstraction.
pub fn exchange_code(settings: &LlmSettings, code: &str, verifier: &str) -> Result<TokenResponse> {
    if settings.token_url.is_empty() {
        return Err(anyhow!("no token URL configured for this provider"));
    }
    let body = format!(
        "grant_type=authorization_code&code={code}&redirect_uri={redirect}&client_id={client_id}&code_verifier={verifier}",
        code = urlencode(code),
        redirect = urlencode(&redirect_uri()),
        client_id = urlencode(&settings.client_id),
        verifier = urlencode(verifier),
    );
    let resp: TokenResponse = ureq::post(&settings.token_url)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&body)
        .context("exchanging authorization code for a token")?
        .into_json()
        .context("parsing token response")?;
    Ok(resp)
}

pub fn store_token(token: &TokenResponse) -> Result<()> {
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).context("opening OS keychain entry")?;
    let json = serde_json::to_string(token).context("serializing token")?;
    entry
        .set_password(&json)
        .context("writing token to OS keychain")
}

pub fn load_token() -> Result<Option<TokenResponse>> {
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).context("opening OS keychain entry")?;
    match entry.get_password() {
        Ok(json) => Ok(Some(
            serde_json::from_str(&json).context("parsing stored token")?,
        )),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow!("reading token from OS keychain: {e}")),
    }
}

pub fn delete_token() -> Result<()> {
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).context("opening OS keychain entry")?;
    match entry.delete_password() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow!("removing token from OS keychain: {e}")),
    }
}

/// Store a plain bearer API key for a custom OpenAI-compatible provider —
/// the "I just have a key and a base URL, no OAuth app" path.
pub fn store_api_key(key: &str) -> Result<()> {
    let entry = keyring::Entry::new(API_KEY_KEYRING_SERVICE, API_KEY_KEYRING_USER)
        .context("opening OS keychain entry")?;
    entry
        .set_password(key)
        .context("writing API key to OS keychain")
}

pub fn load_api_key() -> Result<Option<String>> {
    let entry = keyring::Entry::new(API_KEY_KEYRING_SERVICE, API_KEY_KEYRING_USER)
        .context("opening OS keychain entry")?;
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow!("reading API key from OS keychain: {e}")),
    }
}

pub fn delete_api_key() -> Result<()> {
    let entry = keyring::Entry::new(API_KEY_KEYRING_SERVICE, API_KEY_KEYRING_USER)
        .context("opening OS keychain entry")?;
    match entry.delete_password() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow!("removing API key from OS keychain: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_known_rfc7636_vector() {
        // From RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = pkce_challenge(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn generated_pkce_pair_is_self_consistent() {
        let (verifier, challenge) = generate_pkce();
        assert_eq!(pkce_challenge(&verifier), challenge);
        assert_eq!(verifier.len(), 64);
    }

    #[test]
    fn query_parsing_decodes_percent_and_plus() {
        let params = parse_query("code=abc%2Fdef&state=hello+world");
        assert_eq!(params.get("code").unwrap(), "abc/def");
        assert_eq!(params.get("state").unwrap(), "hello world");
    }

    #[test]
    fn authorize_url_rejects_unconfigured_provider() {
        let settings = LlmSettings::default();
        assert!(authorize_url(&settings, "s", "c").is_err());
    }

    #[test]
    fn authorize_url_includes_pkce_and_redirect() {
        let settings = LlmSettings {
            provider_label: "Test".into(),
            client_id: "abc123".into(),
            auth_url: "https://example.com/authorize".into(),
            token_url: "https://example.com/token".into(),
            scope: "read write".into(),
            api_base: "https://example.com/v1".into(),
            model: "test-model".into(),
            auto_rewrite: false,
        };
        let url = authorize_url(&settings, "STATE1", "CHALLENGE1").unwrap();
        assert!(url.starts_with("https://example.com/authorize?"));
        assert!(url.contains("client_id=abc123"));
        assert!(url.contains("code_challenge=CHALLENGE1"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A48219%2Fcallback"));
    }
}
