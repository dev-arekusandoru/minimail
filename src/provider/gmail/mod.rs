//! Gmail adapter for [`MailProvider`]. All HTTP runs on a background thread.

pub mod auth;
pub mod convert;
pub mod quota;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use rand::{Rng, RngCore};
use serde_json::{Value, json};

pub use auth::{ClientConfig, Tokens};
pub use quota::Quota;

use super::{Body, Changes, MailProvider, Page, ProviderError, RemoteFlags, RemoteFolder, RemoteId, RemoteMessage, RemoteState, Scope, Window};
use crate::clock::Timestamp;

const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const BATCH: &str = "https://gmail.googleapis.com/batch/gmail/v1";
const PAGE: usize = 500;
const BATCH_MAX: usize = 25;
const METADATA_HEADERS: [&str; 5] = ["From", "To", "Cc", "Bcc", "Subject"];
const COST_GET: u32 = 5;
const COST_LIST: u32 = 5;
const COST_HISTORY: u32 = 2;
const COST_LABEL_LIST: u32 = 1;
const COST_LABEL_CREATE: u32 = 5;
const COST_PROFILE: u32 = 1;
const COST_MODIFY: u32 = 5;
const COST_TRASH: u32 = 5;
const RATE_RETRIES: u32 = 5;
const SERVER_RETRIES: u32 = 2;

struct Raw { status: u16, text: String, json: Value, content_type: String, retry_after: Option<Duration> }

enum RequestBody<'a> { Empty, Json(Value), Raw(&'a str, &'a str) }

pub struct GmailProvider {
    http: ureq::Agent,
    auth: Tokens,
    client: ClientConfig,
    user_folders: Option<HashSet<String>>,
    quota: Quota,
}

impl GmailProvider {
    /// `access` starts empty so the first request refreshes the token.
    pub fn new(client: ClientConfig, refresh_token: String) -> Self {
        Self::with_quota(client, refresh_token, Quota::new(quota::UNITS_PER_SEC, quota::BURST))
    }

    pub fn with_quota(client: ClientConfig, refresh_token: String, quota: Quota) -> Self {
        Self {
            http: ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into(),
            auth: Tokens { access: String::new(), refresh: refresh_token },
            client, user_folders: None, quota,
        }
    }

    fn send(&self, method: &str, url: &str, body: &RequestBody<'_>) -> Result<Raw, ProviderError> {
        let auth = format!("Bearer {}", self.auth.access);
        let mut res = match body {
            RequestBody::Empty if method == "GET" => self.http.get(url).header("Authorization", auth).call(),
            RequestBody::Empty => self.http.post(url).header("Authorization", auth).send_empty(),
            RequestBody::Json(v) => self.http.post(url).header("Authorization", auth).send_json(v.clone()),
            RequestBody::Raw(ct, text) => self.http.post(url).header("Authorization", auth).header("Content-Type", (*ct).to_owned()).send(text.as_bytes()),
        }.map_err(|e| ProviderError::Network(e.to_string()))?;
        let status = res.status().as_u16();
        let content_type = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
        let retry_after = res.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|s| s.trim().parse::<u64>().ok()).map(Duration::from_secs);
        let text = res.body_mut().read_to_string().map_err(|e| ProviderError::Network(e.to_string()))?;
        let json = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(Raw { status, text, json, content_type, retry_after })
    }

    fn execute(&mut self, units: u32, method: &str, url: &str, body: RequestBody<'_>) -> Result<Raw, ProviderError> {
        let mut refreshed = false;
        let mut rates = 0;
        let mut servers = 0;
        loop {
            let wait = self.quota.reserve(units, Instant::now());
            if !wait.is_zero() { std::thread::sleep(wait); }
            let raw = self.send(method, url, &body)?;
            if (200..300).contains(&raw.status) { return Ok(raw); }
            if raw.status == 401 {
                if refreshed { return Err(api_error(&raw)); }
                refreshed = true;
                self.auth.access = auth::refresh(&self.http, &self.client, &self.auth.refresh)?;
                continue;
            }
            if convert::is_rate_limited(raw.status, &raw.json) {
                self.quota.drain(Instant::now());
                if rates == RATE_RETRIES { return Err(ProviderError::RateLimited); }
                std::thread::sleep(raw.retry_after.unwrap_or_else(|| backoff(rates)));
                rates += 1;
                continue;
            }
            if raw.status >= 500 {
                if servers == SERVER_RETRIES { return Err(api_error(&raw)); }
                std::thread::sleep(backoff(servers));
                servers += 1;
                continue;
            }
            if raw.status == 404 && url.contains("/history") { return Err(ProviderError::CursorExpired); }
            return Err(api_error(&raw));
        }
    }

    fn call(&mut self, cost: u32, method: &str, url: &str, body: Option<Value>) -> Result<Value, ProviderError> {
        let body = body.map_or(RequestBody::Empty, RequestBody::Json);
        Ok(self.execute(cost, method, url, body)?.json)
    }

    fn user_folder_ids(&mut self) -> Result<HashSet<String>, ProviderError> {
        if self.user_folders.is_none() { self.folders()?; }
        Ok(self.user_folders.clone().unwrap_or_default())
    }

    fn list_all(&mut self, query: convert::ListQuery) -> Result<Vec<RemoteId>, ProviderError> {
        let mut ids = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut params = vec![("maxResults", PAGE.to_string()), ("q", query.q.clone())];
            if let Some(label) = &query.label { params.push(("labelIds", label.clone())); }
            if query.include_trash { params.push(("includeSpamTrash", "true".to_owned())); }
            if let Some(t) = &token { params.push(("pageToken", t.clone())); }
            let url = url_with(&format!("{BASE}/messages"), &params)?;
            let json = self.execute(COST_LIST, "GET", &url, RequestBody::Empty)?.json;
            ids.extend(convert::list_ids(&json));
            match json.get("nextPageToken").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => token = Some(next.to_owned()),
                _ => return Ok(ids),
            }
        }
    }

    fn fetch_batch(&mut self, ids: &[RemoteId], folders: &HashSet<String>) -> Result<Vec<RemoteMessage>, ProviderError> {
        let mut slots: Vec<Option<RemoteMessage>> = vec![None; ids.len()];
        let mut pending: Vec<usize> = (0..ids.len()).collect();
        let mut rate_tries = 0;
        let mut server_tries = 0;
        let mut refreshed = false;
        while !pending.is_empty() {
            let parts: Vec<(String, String)> = pending.iter().enumerate().map(|(n, &i)| (format!("item-{n}"), metadata_path(&ids[i]))).collect();
            let boundary = random_boundary();
            let body = convert::batch_body(&boundary, &parts);
            let ct = format!("multipart/mixed; boundary={boundary}");
            let raw = self.execute(COST_GET * pending.len() as u32, "POST", BATCH, RequestBody::Raw(&ct, &body))?;
            let parsed = convert::parse_batch(&raw.content_type, &raw.text);
            let mut throttled = Vec::new();
            let mut unauthorized = false;
            let mut rate_limited = false;
            let mut server_error = false;
            let mut retry_after = None;
            for part in parsed {
                let Some(n) = part.id.strip_prefix("response-item-").and_then(|n| n.parse::<usize>().ok()) else { continue };
                let Some(&slot) = pending.get(n) else { continue };
                if (200..300).contains(&part.status) {
                    if let Some(msg) = convert::message(&part.json, folders) { slots[slot] = Some(msg); }
                } else if part.status == 404 {
                    continue;
                } else if part.status == 401 {
                    unauthorized = true;
                } else if convert::is_rate_limited(part.status, &part.json) {
                    rate_limited = true;
                    retry_after = retry_after.max(part.retry_after);
                    throttled.push(slot);
                } else if part.status >= 500 {
                    server_error = true;
                    retry_after = retry_after.max(part.retry_after);
                    throttled.push(slot);
                } else {
                    return Err(ProviderError::Api { status: part.status, message: error_message(&part.json) });
                }
            }
            if unauthorized {
                if refreshed { return Err(ProviderError::Api { status: 401, message: "batch authorization failed".into() }); }
                refreshed = true;
                self.auth.access = auth::refresh(&self.http, &self.client, &self.auth.refresh)?;
                pending = (0..ids.len()).collect();
                continue;
            }
            if throttled.is_empty() { break; }
            if rate_limited {
                if rate_tries == RATE_RETRIES { return Err(ProviderError::RateLimited); }
                self.quota.drain(Instant::now());
                std::thread::sleep(retry_after.unwrap_or_else(|| backoff(rate_tries)));
                rate_tries += 1;
            } else if server_error {
                if server_tries == SERVER_RETRIES {
                    return Err(ProviderError::Api { status: 500, message: "batch sub-request failed".into() });
                }
                std::thread::sleep(retry_after.unwrap_or_else(|| backoff(server_tries)));
                server_tries += 1;
            }
            pending = throttled;
        }
        Ok(slots.into_iter().flatten().collect())
    }
}

fn metadata_path(id: &RemoteId) -> String {
    let headers: String = METADATA_HEADERS.iter().map(|h| format!("&metadataHeaders={h}")).collect();
    format!("/gmail/v1/users/me/messages/{id}?format=metadata{headers}")
}

fn random_boundary() -> String {
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

fn backoff(step: u32) -> Duration {
    Duration::from_secs(1u64 << step.min(6)) + Duration::from_millis(rand::thread_rng().gen_range(0..250))
}

fn error_message(json: &Value) -> String {
    json.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn api_error(raw: &Raw) -> ProviderError {
    ProviderError::Api { status: raw.status, message: if raw.json.is_null() { raw.text.trim().to_owned() } else { error_message(&raw.json) } }
}

fn url_with(base: &str, params: &[(&str, String)]) -> Result<String, ProviderError> {
    url::Url::parse_with_params(base, params).map(|u| u.to_string()).map_err(|e| ProviderError::Network(e.to_string()))
}

impl MailProvider for GmailProvider {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, ProviderError> {
        let json = self.call(COST_LABEL_LIST, "GET", &format!("{BASE}/labels"), None)?;
        let folders: Vec<RemoteFolder> = json.get("labels").and_then(Value::as_array).map(|labels| labels.iter().filter(|l| l.get("type").and_then(Value::as_str) == Some("user")).filter_map(|l| Some(RemoteFolder { id: l.get("id")?.as_str()?.to_owned(), path: l.get("name")?.as_str()?.to_owned() })).collect()).unwrap_or_default();
        self.user_folders = Some(folders.iter().map(|f| f.id.clone()).collect());
        Ok(folders)
    }

    fn create_folder(&mut self, path: &str) -> Result<RemoteFolder, ProviderError> {
        let json = self.call(COST_LABEL_CREATE, "POST", &format!("{BASE}/labels"), Some(json!({"name":path,"labelListVisibility":"labelShow","messageListVisibility":"show"})))?;
        let id = json.get("id").and_then(Value::as_str).ok_or_else(|| ProviderError::Api { status: 200, message: "label creation returned no id".into() })?.to_owned();
        let name = json.get("name").and_then(Value::as_str).unwrap_or(path).to_owned();
        Ok(RemoteFolder { id, path: name })
    }

    fn cursor(&mut self) -> Result<String, ProviderError> {
        Ok(self.call(COST_PROFILE, "GET", &format!("{BASE}/profile"), None)?.get("historyId").and_then(Value::as_str).unwrap_or_default().to_owned())
    }

    fn list(&mut self, scope: &Scope, window: Window, page: Option<&str>, max: usize) -> Result<Page, ProviderError> {
        let query = convert::list_query(scope, window);
        let mut params = vec![("maxResults", max.min(PAGE).to_string()), ("q", query.q)];
        if let Some(label) = query.label { params.push(("labelIds", label)); }
        if query.include_trash { params.push(("includeSpamTrash", "true".to_owned())); }
        if let Some(token) = page { params.push(("pageToken", token.to_owned())); }
        let url = url_with(&format!("{BASE}/messages"), &params)?;
        let json = self.execute(COST_LIST, "GET", &url, RequestBody::Empty)?.json;
        Ok(Page { ids: convert::list_ids(&json), next: json.get("nextPageToken").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_owned) })
    }

    fn snapshot(&mut self, since: Timestamp) -> Result<Vec<(RemoteId, RemoteFlags)>, ProviderError> {
        let folders = self.folders()?;
        let window = Window::Since(since);
        let mut scopes = vec![Scope::Archive];
        scopes.extend(folders.iter().rev().map(|f| Scope::Folder(f.id.clone())));
        scopes.extend([Scope::Inbox, Scope::Trash]);
        let mut flags: Vec<(RemoteId, RemoteFlags)> = Vec::new();
        let mut index: std::collections::HashMap<RemoteId, usize> = Default::default();
        for scope in &scopes {
            let state = match scope { Scope::Inbox => RemoteState::Inbox, Scope::Archive => RemoteState::Archived, Scope::Trash => RemoteState::Trash, Scope::Folder(id) => RemoteState::Folder(id.clone()) };
            for id in self.list_all(convert::list_query(scope, window))? {
                match index.get(&id) { Some(&i) => flags[i].1.state = state.clone(), None => { index.insert(id.clone(), flags.len()); flags.push((id, RemoteFlags { state: state.clone(), unread: false })); } }
            }
        }
        let mut unread = convert::list_query(&Scope::Trash, window);
        unread.q = unread.q.replace(" in:trash", " is:unread");
        for id in self.list_all(unread)? { if let Some(&i) = index.get(&id) { flags[i].1.unread = true; } }
        Ok(flags)
    }

    fn changes(&mut self, cursor: &str) -> Result<Changes, ProviderError> {
        let folders = self.user_folder_ids()?;
        let mut out = Changes { cursor: cursor.to_owned(), ..Changes::default() };
        let mut token: Option<String> = None;
        loop {
            let mut params = vec![("startHistoryId", cursor.to_owned())];
            params.extend(["messageAdded", "messageDeleted", "labelAdded", "labelRemoved"].map(|t| ("historyTypes", t.to_owned())));
            if let Some(t) = &token { params.push(("pageToken", t.clone())); }
            let url = url_with(&format!("{BASE}/history"), &params)?;
            let json = self.execute(COST_HISTORY, "GET", &url, RequestBody::Empty)?.json;
            convert::merge_history(&json, &folders, &mut out);
            match json.get("nextPageToken").and_then(Value::as_str) { Some(next) if !next.is_empty() => token = Some(next.to_owned()), _ => break }
        }
        Ok(out)
    }

    fn fetch_headers(&mut self, ids: &[RemoteId]) -> Result<Vec<RemoteMessage>, ProviderError> {
        if ids.is_empty() { return Ok(Vec::new()); }
        let folders = self.user_folder_ids()?;
        let mut messages = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(BATCH_MAX) { messages.extend(self.fetch_batch(chunk, &folders)?); }
        // Batch responses are already reassembled in request order.
        Ok(messages)
    }

    fn fetch_body(&mut self, id: &RemoteId) -> Result<Body, ProviderError> {
        let json = self.call(COST_GET, "GET", &format!("{BASE}/messages/{id}?format=full"), None)?;
        Ok(convert::body(&json))
    }

    fn move_message(&mut self, id: &RemoteId, from: &RemoteState, to: &RemoteState) -> Result<(), ProviderError> {
        if matches!(from, RemoteState::Trash) && !matches!(to, RemoteState::Trash) { self.call(COST_TRASH, "POST", &format!("{BASE}/messages/{id}/untrash"), None)?; }
        if matches!(to, RemoteState::Trash) { self.call(COST_TRASH, "POST", &format!("{BASE}/messages/{id}/trash"), None)?; return Ok(()); }
        let (add, remove) = convert::label_ops(from, to);
        if add.is_empty() && remove.is_empty() { return Ok(()); }
        self.call(COST_MODIFY, "POST", &format!("{BASE}/messages/{id}/modify"), Some(json!({"addLabelIds":add,"removeLabelIds":remove})))?;
        Ok(())
    }

    fn set_read(&mut self, id: &RemoteId, read: bool) -> Result<(), ProviderError> {
        let ops = if read { json!({"removeLabelIds":["UNREAD"]}) } else { json!({"addLabelIds":["UNREAD"]}) };
        self.call(COST_MODIFY, "POST", &format!("{BASE}/messages/{id}/modify"), Some(ops))?;
        Ok(())
    }
}
