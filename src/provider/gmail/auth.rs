//! OAuth 2.0 installed-app flow (PKCE) against Google's endpoints, plus token
//! refresh. Blocking by design: callers run it on a background thread.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::ProviderError;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const PROFILE_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me/profile";
const SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";
/// How long to wait for the browser to come back with the code.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub struct ClientConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl ClientConfig {
    /// Reads the OAuth desktop-client credentials from the environment.
    /// `None` when either is missing or blank.
    pub fn from_env() -> Option<Self> {
        let client_id = std::env::var("MAIL_CLASSIFIER_GOOGLE_CLIENT_ID").ok()?;
        let client_secret = std::env::var("MAIL_CLASSIFIER_GOOGLE_CLIENT_SECRET").ok()?;
        if client_id.trim().is_empty() || client_secret.trim().is_empty() {
            return None;
        }
        Some(Self {
            client_id,
            client_secret,
        })
    }
}

pub struct Tokens {
    pub access: String,
    pub refresh: String,
}

/// An agent that reports non-2xx statuses as ordinary responses.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into()
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn random_b64(len: usize) -> String {
    let mut buf = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut buf);
    b64(&buf)
}

fn token_call(http: &ureq::Agent, form: &[(&str, &str)]) -> Result<Value, ProviderError> {
    let mut res = http
        .post(TOKEN_URL)
        .send_form(form.iter().copied())
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    let status = res.status().as_u16();
    let text = res
        .body_mut()
        .read_to_string()
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = json
            .get("error_description")
            .and_then(Value::as_str)
            .or_else(|| json.get("error").and_then(Value::as_str))
            .unwrap_or(text.trim())
            .to_owned();
        return Err(ProviderError::Auth(message));
    }
    Ok(json)
}

/// Exchanges a refresh token for a fresh access token.
pub fn refresh(
    http: &ureq::Agent,
    client: &ClientConfig,
    refresh_token: &str,
) -> Result<String, ProviderError> {
    let json = token_call(
        http,
        &[
            ("client_id", &client.client_id),
            ("client_secret", &client.client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ],
    )?;
    json.get("access_token")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ProviderError::Auth("no access token".into()))
}

/// Runs the loopback OAuth flow: opens `url` in the browser, waits for the
/// redirect, and returns the tokens plus the account email address.
pub fn sign_in(
    client: &ClientConfig,
    open: impl FnOnce(&str),
) -> Result<(Tokens, String), ProviderError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| ProviderError::Network(format!("cannot bind callback port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| ProviderError::Network(e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let verifier = random_b64(32);
    let challenge = b64(&Sha256::digest(verifier.as_bytes()));
    let state = random_b64(16);

    let url = url::Url::parse_with_params(
        AUTH_URL,
        &[
            ("client_id", client.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ],
    )
    .map_err(|e| ProviderError::Auth(e.to_string()))?
    .to_string();
    open(&url);

    let code = wait_for_code(&listener, &state)?;

    let http = agent();
    let json = token_call(
        &http,
        &[
            ("code", code.as_str()),
            ("client_id", client.client_id.as_str()),
            ("client_secret", client.client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ],
    )?;
    let refresh = json
        .get("refresh_token")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::Auth("no refresh token".into()))?
        .to_owned();
    let access = json
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::Auth("no access token".into()))?
        .to_owned();

    let email = profile_email(&http, &access)?;
    Ok((Tokens { access, refresh }, email))
}

fn profile_email(http: &ureq::Agent, access: &str) -> Result<String, ProviderError> {
    let mut res = http
        .get(PROFILE_URL)
        .header("Authorization", format!("Bearer {access}"))
        .call()
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    let status = res.status().as_u16();
    let text = res
        .body_mut()
        .read_to_string()
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(ProviderError::Auth(format!("profile request failed ({status})")));
    }
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("emailAddress")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(|| ProviderError::Auth("no account email".into()))
}

/// Accepts callback connections until one carries an OAuth response, replying
/// to each with a small page the user can read.
fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String, ProviderError> {
    listener
        .set_nonblocking(false)
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    for _ in 0..32 {
        let (mut stream, _) = listener
            .accept()
            .map_err(|e| ProviderError::Network(format!("callback failed: {e}")))?;
        stream
            .set_read_timeout(Some(CALLBACK_TIMEOUT))
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        let mut buf = [0u8; 8192];
        let n = stream
            .read(&mut buf)
            .map_err(|e| ProviderError::Network(format!("callback read failed: {e}")))?;
        let request = String::from_utf8_lossy(&buf[..n]);
        let Some(path) = request.split_whitespace().nth(1) else {
            continue;
        };
        let Ok(parsed) = url::Url::parse(&format!("http://127.0.0.1{path}")) else {
            continue;
        };
        let mut code = None;
        let mut error = None;
        let mut got_state = None;
        for (key, value) in parsed.query_pairs() {
            match key.as_ref() {
                "code" => code = Some(value.into_owned()),
                "error" => error = Some(value.into_owned()),
                "state" => got_state = Some(value.into_owned()),
                _ => {}
            }
        }
        // A request with neither `code` nor `error` is browser noise (a
        // favicon, a prefetch); keep listening instead of failing the sign-in.
        if code.is_none() && error.is_none() {
            continue;
        }
        let outcome: Result<(), ProviderError> = if let Some(why) = error {
            Err(ProviderError::Auth(format!("authorization failed: {why}")))
        } else if got_state.as_deref() != Some(state) {
            Err(ProviderError::Auth("state mismatch".into()))
        } else {
            Ok(())
        };
        let page = match &outcome {
            Ok(()) => "Signed in — you can close this tab.".to_owned(),
            Err(e) => format!("Sign-in failed: {e}"),
        };
        let result = outcome.and_then(|()| code.ok_or(ProviderError::Auth("no code".into())));
        let html = format!("<!doctype html><meta charset=\"utf-8\"><title>mail-classifier</title><p>{page}");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
            html.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        return result;
    }
    Err(ProviderError::Auth("no authorization code received".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives `wait_for_code` over a loopback socket, returning its verdict and
    /// the page it wrote back to the browser.
    fn callback(paths: &[&str], state: &str) -> (Result<String, ProviderError>, String) {
        let paths: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let client = std::thread::spawn(move || {
            let mut reply = String::new();
            for path in &paths {
                let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
                stream
                    .write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
                    .expect("write");
                let mut buf = [0u8; 1024];
                if let Ok(n) = stream.read(&mut buf) {
                    reply.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            }
            reply
        });
        let result = wait_for_code(&listener, state);
        let reply = client.join().expect("client");
        (result, reply)
    }

    #[test]
    fn pkce_challenge_matches_the_s256_of_the_verifier() {
        // RFC 7636 appendix B vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            b64(&Sha256::digest(verifier.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn callback_returns_the_code_and_reports_success() {
        let (result, reply) = callback(&["/?code=abc123&state=st"], "st");
        assert_eq!(result.ok().as_deref(), Some("abc123"));
        assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
        assert!(reply.contains("Signed in — you can close this tab."), "{reply}");
    }

    #[test]
    fn callback_skips_stray_requests_before_the_code() {
        let (result, _) = callback(&["/favicon.ico", "/?code=zz&state=st"], "st");
        assert_eq!(result.ok().as_deref(), Some("zz"));
    }

    #[test]
    fn callback_rejects_a_mismatched_state() {
        let (result, reply) = callback(&["/?code=abc&state=other"], "st");
        assert!(matches!(result, Err(ProviderError::Auth(_))), "{result:?}");
        assert!(reply.contains("state mismatch"), "{reply}");
    }

    #[test]
    fn callback_surfaces_a_denied_consent() {
        let (result, reply) = callback(&["/?error=access_denied&state=st"], "st");
        assert!(matches!(result, Err(ProviderError::Auth(_))), "{result:?}");
        assert!(reply.contains("access_denied"), "{reply}");
    }
}