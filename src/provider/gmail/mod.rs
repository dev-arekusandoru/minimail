//! Gmail adapter for [`MailProvider`]. All HTTP runs on a background thread.

pub mod auth;
pub mod convert;

use std::collections::HashSet;
use std::time::Duration;

use serde_json::{Value, json};

pub use auth::{ClientConfig, Tokens};

use super::{
    Body, Changes, MailProvider, Page, ProviderError, RemoteFlags, RemoteFolder, RemoteId,
    RemoteMessage, RemoteState, Scope, Window,
};
use crate::clock::Timestamp;

const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const PAGE: usize = 500;
const METADATA_HEADERS: [&str; 5] = ["From", "To", "Cc", "Bcc", "Subject"];

pub struct GmailProvider {
    http: ureq::Agent,
    auth: Tokens,
    client: ClientConfig,
    /// Ids of user labels, fetched lazily and refreshed by `folders()`.
    user_folders: Option<HashSet<String>>,
}

impl GmailProvider {
    /// `access` starts empty so the first request refreshes the token.
    pub fn new(client: ClientConfig, refresh_token: String) -> Self {
        Self {
            http: ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(30)))
                .build()
                .into(),
            auth: Tokens {
                access: String::new(),
                refresh: refresh_token,
            },
            client,
            user_folders: None,
        }
    }

    /// Issues one API request, refreshing the access token once on 401.
    fn call(
        &mut self,
        method: &str,
        url: &str,
        body: Option<Value>,
    ) -> Result<Value, ProviderError> {
        let attempt = self.send(method, url, body.clone());
        let err = match attempt {
            Ok(v) => return Ok(v),
            Err(e) => e,
        };
        if !matches!(&err, ProviderError::Api { status: 401, .. }) {
            return Err(err);
        }
        self.auth.access = auth::refresh(&self.http, &self.client, &self.auth.refresh)?;
        self.send(method, url, body)
    }

    fn send(
        &self,
        method: &str,
        url: &str,
        body: Option<Value>,
    ) -> Result<Value, ProviderError> {
        let mut res = match (method, &body) {
            ("GET", _) => self
                .http
                .get(url)
                .header("Authorization", format!("Bearer {}", self.auth.access))
                .call(),
            (_, Some(value)) => self
                .http
                .post(url)
                .header("Authorization", format!("Bearer {}", self.auth.access))
                .send_json(value.clone()),
            (_, None) => self
                .http
                .post(url)
                .header("Authorization", format!("Bearer {}", self.auth.access))
                .send_empty(),
        }
        .map_err(|e| ProviderError::Network(e.to_string()))?;
        let status = res.status().as_u16();
        let text = res
            .body_mut()
            .read_to_string()
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        let json: Value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::Null)
        };
        if (200..300).contains(&status) {
            return Ok(json);
        }
        if status == 401 {
            return Err(ProviderError::Api {
                status: 401,
                message: String::new(),
            });
        }
        if status == 404 && url.contains("/history") {
            return Err(ProviderError::CursorExpired);
        }
        if convert::is_rate_limited(status, &json) {
            return Err(ProviderError::RateLimited);
        }
        let message = json
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| text.trim().to_owned());
        Err(ProviderError::Api { status, message })
    }

    /// Ids of the user's own labels, which stand for folders. Cached; refreshed
    /// by every [`MailProvider::folders`] call.
    fn user_folder_ids(&mut self) -> Result<HashSet<String>, ProviderError> {
        if self.user_folders.is_none() {
            self.folders()?;
        }
        Ok(self.user_folders.clone().unwrap_or_default())
    }

    /// Every id matching `query`, following page tokens.
    fn list_all(&mut self, query: convert::ListQuery) -> Result<Vec<RemoteId>, ProviderError> {
        let mut ids = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut params = vec![("maxResults", PAGE.to_string()), ("q", query.q.clone())];
            if let Some(label) = &query.label {
                params.push(("labelIds", label.clone()));
            }
            if query.include_trash {
                params.push(("includeSpamTrash", "true".to_owned()));
            }
            if let Some(token) = &page_token {
                params.push(("pageToken", token.clone()));
            }
            let json = self.call("GET", &url_with(&format!("{BASE}/messages"), &params)?, None)?;
            ids.extend(convert::list_ids(&json));
            match json.get("nextPageToken").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => page_token = Some(next.to_owned()),
                _ => return Ok(ids),
            }
        }
    }
}

fn url_with(base: &str, params: &[(&str, String)]) -> Result<String, ProviderError> {
    url::Url::parse_with_params(base, params)
        .map(|u| u.to_string())
        .map_err(|e| ProviderError::Network(e.to_string()))
}

impl MailProvider for GmailProvider {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, ProviderError> {
        let json = self.call("GET", &format!("{BASE}/labels"), None)?;
        let folders: Vec<RemoteFolder> = json
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| {
                labels
                    .iter()
                    .filter(|l| l.get("type").and_then(Value::as_str) == Some("user"))
                    .filter_map(|l| {
                        let id = l.get("id").and_then(Value::as_str)?;
                        let path = l.get("name").and_then(Value::as_str)?;
                        Some(RemoteFolder {
                            id: id.to_owned(),
                            path: path.to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.user_folders = Some(folders.iter().map(|f| f.id.clone()).collect());
        Ok(folders)
    }

    fn create_folder(&mut self, path: &str) -> Result<RemoteFolder, ProviderError> {
        let json = self.call(
            "POST",
            &format!("{BASE}/labels"),
            Some(json!({
                "name": path,
                "labelListVisibility": "labelShow",
                "messageListVisibility": "show",
            })),
        )?;
        let id = json
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::Api {
                status: 200,
                message: "label creation returned no id".into(),
            })?
            .to_owned();
        let name = json
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(path)
            .to_owned();
        Ok(RemoteFolder { id, path: name })
    }

    fn cursor(&mut self) -> Result<String, ProviderError> {
        let profile = self.call("GET", &format!("{BASE}/profile"), None)?;
        Ok(profile
            .get("historyId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    fn list(
        &mut self,
        scope: &Scope,
        window: Window,
        page: Option<&str>,
        max: usize,
    ) -> Result<Page, ProviderError> {
        let query = convert::list_query(scope, window);
        let mut params = vec![("maxResults", max.min(PAGE).to_string()), ("q", query.q)];
        if let Some(label) = query.label {
            params.push(("labelIds", label));
        }
        if query.include_trash {
            params.push(("includeSpamTrash", "true".to_owned()));
        }
        if let Some(token) = page {
            params.push(("pageToken", token.to_owned()));
        }
        let json = self.call("GET", &url_with(&format!("{BASE}/messages"), &params)?, None)?;
        Ok(Page {
            ids: convert::list_ids(&json),
            next: json
                .get("nextPageToken")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .map(str::to_owned),
        })
    }

    fn snapshot(&mut self, since: Timestamp) -> Result<Vec<(RemoteId, RemoteFlags)>, ProviderError> {
        let folders = self.folders()?;
        let window = Window::Since(since);
        // Later scopes win, so order from weakest to strongest: a message in
        // the inbox and a label counts as Inbox; anything in trash is Trash.
        let mut scopes = vec![Scope::Archive];
        scopes.extend(folders.iter().rev().map(|f| Scope::Folder(f.id.clone())));
        scopes.extend([Scope::Inbox, Scope::Trash]);
        let mut flags: Vec<(RemoteId, RemoteFlags)> = Vec::new();
        let mut index: std::collections::HashMap<RemoteId, usize> = Default::default();
        for scope in &scopes {
            let state = match scope {
                Scope::Inbox => RemoteState::Inbox,
                Scope::Archive => RemoteState::Archived,
                Scope::Trash => RemoteState::Trash,
                Scope::Folder(id) => RemoteState::Folder(id.clone()),
            };
            for id in self.list_all(convert::list_query(scope, window))? {
                match index.get(&id) {
                    Some(&i) => flags[i].1.state = state.clone(),
                    None => {
                        index.insert(id.clone(), flags.len());
                        flags.push((id, RemoteFlags { state: state.clone(), unread: false }));
                    }
                }
            }
        }
        // One listing for unread mail anywhere in the window (trash included).
        let mut unread = convert::list_query(&Scope::Trash, window);
        unread.q = unread.q.replace(" in:trash", " is:unread");
        for id in self.list_all(unread)? {
            if let Some(&i) = index.get(&id) {
                flags[i].1.unread = true;
            }
        }
        Ok(flags)
    }

    fn changes(&mut self, cursor: &str) -> Result<Changes, ProviderError> {
        let user_folders = self.user_folder_ids()?;
        let mut out = Changes {
            cursor: cursor.to_owned(),
            ..Changes::default()
        };
        let mut page_token: Option<String> = None;
        loop {
            let mut params = vec![("startHistoryId", cursor.to_owned())];
            params.extend(
                ["messageAdded", "messageDeleted", "labelAdded", "labelRemoved"]
                    .map(|t| ("historyTypes", t.to_owned())),
            );
            if let Some(token) = &page_token {
                params.push(("pageToken", token.clone()));
            }
            let json = self.call("GET", &url_with(&format!("{BASE}/history"), &params)?, None)?;
            convert::merge_history(&json, &user_folders, &mut out);
            match json.get("nextPageToken").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => page_token = Some(next.to_owned()),
                _ => break,
            }
        }
        Ok(out)
    }

    fn fetch_headers(&mut self, ids: &[RemoteId]) -> Result<Vec<RemoteMessage>, ProviderError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let user_folders = self.user_folder_ids()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let url = format!(
                "{BASE}/messages/{id}?format=metadata{}",
                METADATA_HEADERS.map(|h| format!("&metadataHeaders={h}")).concat()
            );
            match self.call("GET", &url, None) {
                Ok(json) => out.extend(convert::message(&json, &user_folders)),
                Err(ProviderError::Api { status: 404, .. }) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    fn fetch_body(&mut self, id: &RemoteId) -> Result<Body, ProviderError> {
        let json = self.call("GET", &format!("{BASE}/messages/{id}?format=full"), None)?;
        Ok(convert::body(&json))
    }

    fn move_message(
        &mut self,
        id: &RemoteId,
        from: &RemoteState,
        to: &RemoteState,
    ) -> Result<(), ProviderError> {
        if matches!(from, RemoteState::Trash) && !matches!(to, RemoteState::Trash) {
            self.call("POST", &format!("{BASE}/messages/{id}/untrash"), None)?;
        }
        if matches!(to, RemoteState::Trash) {
            self.call("POST", &format!("{BASE}/messages/{id}/trash"), None)?;
            return Ok(());
        }
        let (add, remove) = convert::label_ops(from, to);
        if add.is_empty() && remove.is_empty() {
            return Ok(());
        }
        self.call(
            "POST",
            &format!("{BASE}/messages/{id}/modify"),
            Some(json!({ "addLabelIds": add, "removeLabelIds": remove })),
        )?;
        Ok(())
    }

    fn set_read(&mut self, id: &RemoteId, read: bool) -> Result<(), ProviderError> {
        let ops = if read {
            json!({ "removeLabelIds": ["UNREAD"] })
        } else {
            json!({ "addLabelIds": ["UNREAD"] })
        };
        self.call("POST", &format!("{BASE}/messages/{id}/modify"), Some(ops))?;
        Ok(())
    }
}