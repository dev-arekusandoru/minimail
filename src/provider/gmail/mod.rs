//! Gmail adapter for [`MailProvider`]. All HTTP runs on a background thread.

pub mod auth;
pub mod convert;

use std::collections::HashSet;
use std::time::Duration;

use serde_json::{Value, json};

pub use auth::{ClientConfig, Tokens};

use super::{Changes, MailProvider, ProviderError, RemoteFolder, RemoteId, RemoteMessage, RemoteState};

const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const PAGE: usize = 500;

pub struct GmailProvider {
    http: ureq::Agent,
    auth: Tokens,
    client: ClientConfig,
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

    /// Ids of the user's own labels, which stand for folders.
    fn user_folder_ids(&mut self) -> Result<HashSet<String>, ProviderError> {
        Ok(self
            .folders()?
            .into_iter()
            .map(|f| f.id)
            .collect())
    }
}

fn push_unique(target: &mut Vec<RemoteId>, ids: &[RemoteId]) {
    for id in ids {
        if !target.contains(id) {
            target.push(id.clone());
        }
    }
}

impl MailProvider for GmailProvider {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, ProviderError> {
        let json = self.call("GET", &format!("{BASE}/labels"), None)?;
        Ok(json
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
            .unwrap_or_default())
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

    fn recent(&mut self, limit: usize) -> Result<(Vec<RemoteId>, String), ProviderError> {
        let profile = self.call("GET", &format!("{BASE}/profile"), None)?;
        let cursor = profile
            .get("historyId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut ids = Vec::new();
        let mut page_token = String::new();
        while ids.len() < limit {
            let mut params = vec![
                ("maxResults", PAGE.to_string()),
                ("q", "-in:spam -in:drafts -in:chats".to_owned()),
            ];
            if !page_token.is_empty() {
                params.push(("pageToken", page_token.clone()));
            }
            let url = url::Url::parse_with_params(&format!("{BASE}/messages"), &params)
                .map_err(|e| ProviderError::Network(e.to_string()))?
                .to_string();
            let json = self.call("GET", &url, None)?;
            ids.extend(convert::list_ids(&json));
            match json.get("nextPageToken").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => page_token = next.to_owned(),
                _ => break,
            }
        }
        ids.truncate(limit);
        Ok((ids, cursor))
    }

    fn changes(&mut self, cursor: &str) -> Result<Changes, ProviderError> {
        let mut changed: Vec<RemoteId> = Vec::new();
        let mut removed: Vec<RemoteId> = Vec::new();
        let mut page_token = String::new();
        let mut next_cursor = cursor.to_owned();
        loop {
            let mut params = vec![("startHistoryId", cursor.to_owned())];
            params.extend(
                ["messageAdded", "messageDeleted", "labelAdded", "labelRemoved"]
                    .map(|t| ("historyTypes", t.to_owned())),
            );
            if !page_token.is_empty() {
                params.push(("pageToken", page_token.clone()));
            }
            let url = url::Url::parse_with_params(&format!("{BASE}/history"), &params)
                .map_err(|e| ProviderError::Network(e.to_string()))?
                .to_string();
            let json = self.call("GET", &url, None)?;
            if let Some(history_id) = json.get("historyId").and_then(Value::as_str) {
                next_cursor = history_id.to_owned();
            }
            for record in json
                .get("history")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                for entry in record
                    .get("messagesDeleted")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(id) = entry
                        .get("message")
                        .and_then(|m| m.get("id"))
                        .and_then(Value::as_str)
                    {
                        push_unique(&mut removed, &[id.to_owned()]);
                    }
                }
                for key in ["messagesAdded", "labelsAdded", "labelsRemoved"] {
                    for entry in record.get(key).and_then(Value::as_array).into_iter().flatten() {
                        if let Some(id) = entry
                            .get("message")
                            .and_then(|m| m.get("id"))
                            .and_then(Value::as_str)
                        {
                            push_unique(&mut changed, &[id.to_owned()]);
                        }
                    }
                }
            }
            match json.get("nextPageToken").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => page_token = next.to_owned(),
                _ => break,
            }
        }
        changed.retain(|id| !removed.contains(id));
        Ok(Changes {
            changed,
            removed,
            cursor: next_cursor,
        })
    }

    fn fetch(&mut self, ids: &[RemoteId]) -> Result<Vec<RemoteMessage>, ProviderError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let user_folders = self.user_folder_ids()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            match self.call("GET", &format!("{BASE}/messages/{id}?format=full"), None) {
                Ok(json) => out.extend(convert::message(&json, &user_folders)),
                Err(ProviderError::Api { status: 404, .. }) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(out)
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
}