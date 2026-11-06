//! Blocking HTTP client for the broker. Call it from the background worker.

use std::fmt;
use std::path::Path;

use nexus_protocol::{
    Account, AdminApp, CatalogApp, DevicePoll, DevicePollRequest, DeviceStart, Grant, GrantRequest,
    PublishRequest, ReleaseMeta, UpsertApp,
};

use crate::checksum;

pub struct CentralClient {
    http: reqwest::blocking::Client,
    base: reqwest::Url,
    token: Option<String>,
}

impl fmt::Debug for CentralClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CentralClient")
            .field("base", &self.base.as_str())
            .field("signed_in", &self.token.is_some())
            .finish()
    }
}

impl CentralClient {
    pub fn new(server_url: &str, token: Option<String>) -> Result<Self, String> {
        let base = reqwest::Url::parse(server_url).map_err(|err| err.to_string())?;
        let http = reqwest::blocking::Client::builder()
            .user_agent(crate::github::user_agent_value())
            .build()
            .map_err(|err| err.to_string())?;
        Ok(Self { http, base, token })
    }

    pub fn signed_in(&self) -> bool {
        self.token.is_some()
    }

    pub fn start_device(&self) -> Result<DeviceStart, String> {
        self.send("POST", "v1/auth/device/start", false, None)
    }

    pub fn poll_device(&self, poll_id: &str) -> Result<DevicePoll, String> {
        self.send(
            "POST",
            "v1/auth/device/poll",
            false,
            Some(
                serde_json::to_value(DevicePollRequest {
                    poll_id: poll_id.to_string(),
                })
                .map_err(|err| err.to_string())?,
            ),
        )
    }

    pub fn logout(&self) -> Result<(), String> {
        let _bytes = self.request("POST", "v1/auth/logout", true, None)?;
        Ok(())
    }

    pub fn account(&self) -> Result<Account, String> {
        self.send("GET", "v1/account", true, None)
    }

    pub fn catalog(&self) -> Result<Vec<CatalogApp>, String> {
        self.send("GET", "v1/catalog", true, None)
    }

    pub fn admin_apps(&self) -> Result<Vec<AdminApp>, String> {
        self.send("GET", "v1/admin/apps", true, None)
    }

    pub fn upsert_app(&self, app: &UpsertApp) -> Result<AdminApp, String> {
        self.send(
            "POST",
            "v1/admin/apps",
            true,
            Some(serde_json::to_value(app).map_err(|err| err.to_string())?),
        )
    }

    pub fn delete_app(&self, id: &str) -> Result<(), String> {
        let path = format!("v1/admin/apps/{id}");
        let (status, _headers, bytes) = self.request("DELETE", &path, true, None)?;
        if !status.is_success() {
            return Err(status_message(status, &bytes));
        }
        Ok(())
    }

    pub fn set_published(&self, id: &str, published: bool) -> Result<(), String> {
        let path = format!("v1/admin/apps/{id}/published");
        let body =
            serde_json::to_value(PublishRequest { published }).map_err(|err| err.to_string())?;
        let _bytes = self.request("POST", &path, true, Some(body))?;
        Ok(())
    }

    pub fn grants(&self, id: &str) -> Result<Vec<Grant>, String> {
        self.send("GET", &format!("v1/admin/apps/{id}/grants"), true, None)
    }

    pub fn grant(&self, id: &str, login: &str) -> Result<Grant, String> {
        self.send(
            "POST",
            &format!("v1/admin/apps/{id}/grants"),
            true,
            Some(
                serde_json::to_value(GrantRequest {
                    login: login.to_string(),
                })
                .map_err(|err| err.to_string())?,
            ),
        )
    }

    pub fn revoke(&self, id: &str, github_id: u64) -> Result<(), String> {
        let path = format!("v1/admin/apps/{id}/grants/{github_id}");
        let _bytes = self.request("DELETE", &path, true, None)?;
        Ok(())
    }

    pub fn release(&self, id: &str, os: &str, arch: &str) -> Result<ReleaseMeta, String> {
        let path = format!("v1/apps/{id}/release?os={os}&arch={arch}");
        self.send("GET", &path, true, None)
    }

    pub fn download_verified(
        &self,
        id: &str,
        os: &str,
        arch: &str,
        dest: &Path,
    ) -> Result<ReleaseMeta, String> {
        let meta = self.release(id, os, arch)?;
        let path = format!("v1/apps/{id}/download?os={os}&arch={arch}");
        let (status, headers, bytes) = self.request_raw("GET", &path, true, None)?;
        if !status.is_success() {
            return Err(status_message(status, &bytes));
        }
        let header_sha = headers
            .get("x-nexus-sha256")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !header_sha.eq_ignore_ascii_case(&meta.sha256) {
            return Err("download checksum header did not match the release metadata".into());
        }
        let got = checksum::sha256_bytes(&bytes);
        if !got.eq_ignore_ascii_case(&meta.sha256) {
            return Err("downloaded bytes did not match the SHA-256 checksum".into());
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(dest, &bytes).map_err(|err| err.to_string())?;
        Ok(meta)
    }

    fn send<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        auth: bool,
        body: Option<serde_json::Value>,
    ) -> Result<T, String> {
        let (status, _headers, bytes) = self.request(method, path, auth, body)?;
        if !status.is_success() {
            return Err(status_message(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|err| format!("response was not valid JSON: {err}"))
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        auth: bool,
        body: Option<serde_json::Value>,
    ) -> Result<(reqwest::StatusCode, reqwest::header::HeaderMap, Vec<u8>), String> {
        self.request_raw(method, path, auth, body)
    }

    fn request_raw(
        &self,
        method: &str,
        path: &str,
        auth: bool,
        body: Option<serde_json::Value>,
    ) -> Result<(reqwest::StatusCode, reqwest::header::HeaderMap, Vec<u8>), String> {
        let url = endpoint(&self.base, path)?;
        let method =
            reqwest::Method::from_bytes(method.as_bytes()).map_err(|err| err.to_string())?;
        let mut request = self.http.request(method, url);
        if auth {
            let Some(token) = &self.token else {
                return Err("sign in required".into());
            };
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        let response = request
            .send()
            .map_err(|err| format!("broker network error: {err}"))?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.bytes().map_err(|err| err.to_string())?.to_vec();
        Ok((status, headers, bytes))
    }
}

fn endpoint(base: &reqwest::Url, path: &str) -> Result<reqwest::Url, String> {
    let mut url = base.clone();
    let path = path.trim_start_matches('/');
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    url.set_path(&format!("/{path}"));
    if query.is_empty() {
        url.set_query(None);
    } else {
        url.set_query(Some(query));
    }
    if url.host() != base.host() || url.scheme() != base.scheme() {
        return Err("refusing to leave the configured broker host".into());
    }
    Ok(url)
}

fn status_message(status: reqwest::StatusCode, bytes: &[u8]) -> String {
    if let Ok(error) = serde_json::from_slice::<nexus_protocol::ApiError>(bytes) {
        return format!("{status}: {}", error.message);
    }
    format!("broker returned {status}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_the_session_token() {
        let client = CentralClient {
            http: reqwest::blocking::Client::new(),
            base: reqwest::Url::parse("http://127.0.0.1:8787").unwrap(),
            token: Some("session-secret-value".into()),
        };
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("session-secret-value"));
        assert!(rendered.contains("signed_in"));
    }
}
