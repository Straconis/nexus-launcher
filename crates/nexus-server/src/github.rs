//! GitHub calls made only by the server. The release token never leaves this process.

use std::time::Duration;

use async_trait::async_trait;
use nexus_protocol::ReleaseMeta;
use serde::Deserialize;

use crate::error::ApiFailure;

#[derive(Debug, Clone)]
pub struct DeviceCodes {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone)]
pub enum DevicePoll {
    Pending,
    SlowDown,
    Approved { access_token: String },
    Denied,
    Expired,
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub id: u64,
    pub login: String,
}

#[derive(Debug, Clone)]
pub struct GhAsset {
    pub name: String,
    pub api_url: String,
    pub digest: Option<String>,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct GhRelease {
    pub tag: String,
    pub assets: Vec<GhAsset>,
}

#[derive(Debug, thiserror::Error)]
pub enum GithubError {
    #[error("github request failed")]
    Failed,
}

#[async_trait]
pub trait Github: Send + Sync {
    async fn public_metadata(
        &self,
        _repo: &str,
        _client_id: Option<&str>,
        _client_secret: Option<&str>,
        _token: Option<&str>,
    ) -> Result<nexus_protocol::PublicMetadata, GithubError> {
        Err(GithubError::Failed)
    }
    async fn start_device(&self, client_id: &str) -> Result<DeviceCodes, GithubError>;
    async fn poll_device(
        &self,
        client_id: &str,
        client_secret: &str,
        device_code: &str,
    ) -> Result<DevicePoll, GithubError>;
    async fn current_user(&self, access_token: &str) -> Result<Identity, GithubError>;
    async fn user_by_login(&self, login: &str) -> Result<Identity, GithubError>;
    async fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
    ) -> Result<GhRelease, GithubError>;
    async fn fetch_bytes(
        &self,
        url: &str,
        token: Option<&str>,
        accept: &str,
    ) -> Result<Vec<u8>, GithubError>;
}

pub struct LiveGithub {
    http: reqwest::Client,
}

impl LiveGithub {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .user_agent(format!(
                "nexus-server/{} (+https://github.com/Straconis/nexus-launcher)",
                env!("CARGO_PKG_VERSION")
            ))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 8 {
                    return attempt.error("too many redirects");
                }
                match attempt.url().host_str() {
                    Some(host)
                        if attempt.url().scheme() == "https" && github_host_allowed(host) =>
                    {
                        attempt.follow()
                    }
                    _ => attempt.error("redirect host is not allowed"),
                }
            }))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|err| err.to_string())?;
        Ok(Self { http })
    }
}

#[async_trait]
impl Github for LiveGithub {
    async fn public_metadata(
        &self,
        repo: &str,
        client_id: Option<&str>,
        client_secret: Option<&str>,
        token: Option<&str>,
    ) -> Result<nexus_protocol::PublicMetadata, GithubError> {
        let client = reqwest::Client::builder()
            .user_agent("nexus-server-public-metadata")
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| GithubError::Failed)?;
        let request = |suffix: &str| {
            let request = client
                .get(format!("https://api.github.com/repos/{repo}/{suffix}"))
                .header("Accept", "application/vnd.github+json");
            if let Some(token) = token {
                request.bearer_auth(token)
            } else if let (Some(id), Some(secret)) = (client_id, client_secret) {
                request.basic_auth(id, Some(secret))
            } else {
                request
            }
        };
        let response = request("releases/latest")
            .send()
            .await
            .map_err(|_| GithubError::Failed)?;
        let release_json = if response.status() == reqwest::StatusCode::NOT_FOUND {
            None
        } else if response.status().is_success() {
            Some(response.text().await.map_err(|_| GithubError::Failed)?)
        } else {
            return Err(GithubError::Failed);
        };
        let languages = if let Ok(response) = request("languages").send().await {
            if response.status().is_success() {
                nexus_protocol::top_languages(response.json().await.unwrap_or_default())
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        Ok(nexus_protocol::PublicMetadata {
            release_json,
            languages,
            stale: false,
        })
    }

    async fn start_device(&self, client_id: &str) -> Result<DeviceCodes, GithubError> {
        let response = self
            .http
            .post("https://github.com/login/device/code")
            .header(reqwest::header::ACCEPT, "application/json")
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(format!(
                "client_id={}&scope=read:user",
                urlencoding(client_id)
            ))
            .send()
            .await
            .map_err(|_| GithubError::Failed)?;
        if !response.status().is_success() {
            return Err(GithubError::Failed);
        }
        let body: DeviceCodeBody = response.json().await.map_err(|_| GithubError::Failed)?;
        let complete = body
            .verification_uri_complete
            .unwrap_or_else(|| body.verification_uri.clone());
        Ok(DeviceCodes {
            device_code: body.device_code,
            user_code: body.user_code,
            verification_uri: body.verification_uri,
            verification_uri_complete: complete,
            expires_in: body.expires_in,
            interval: body.interval.unwrap_or(5),
        })
    }

    async fn poll_device(
        &self,
        client_id: &str,
        client_secret: &str,
        device_code: &str,
    ) -> Result<DevicePoll, GithubError> {
        let response = self
            .http
            .post("https://github.com/login/oauth/access_token")
            .header(reqwest::header::ACCEPT, "application/json")
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(format!(
                "client_id={}&client_secret={}&device_code={}&grant_type={}",
                urlencoding(client_id),
                urlencoding(client_secret),
                urlencoding(device_code),
                urlencoding("urn:ietf:params:oauth:grant-type:device_code")
            ))
            .send()
            .await
            .map_err(|_| GithubError::Failed)?;
        let body: TokenBody = response.json().await.map_err(|_| GithubError::Failed)?;
        if let Some(token) = body.access_token {
            return Ok(DevicePoll::Approved {
                access_token: token,
            });
        }
        Ok(match body.error.as_deref() {
            Some("authorization_pending") => DevicePoll::Pending,
            Some("slow_down") => DevicePoll::SlowDown,
            Some("access_denied") => DevicePoll::Denied,
            Some("expired_token") => DevicePoll::Expired,
            _ => return Err(GithubError::Failed),
        })
    }

    async fn current_user(&self, access_token: &str) -> Result<Identity, GithubError> {
        self.identity("https://api.github.com/user", Some(access_token))
            .await
    }

    async fn user_by_login(&self, login: &str) -> Result<Identity, GithubError> {
        self.identity(&format!("https://api.github.com/users/{login}"), None)
            .await
    }

    async fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
    ) -> Result<GhRelease, GithubError> {
        let url = format!("https://api.github.com/repos/{repo}/releases/latest");
        require_github_url(&url).map_err(|_| GithubError::Failed)?;
        let mut request = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|_| GithubError::Failed)?;
        if !response.status().is_success() {
            return Err(GithubError::Failed);
        }
        let body: ReleaseBody = response.json().await.map_err(|_| GithubError::Failed)?;
        Ok(GhRelease {
            tag: body.tag_name,
            assets: body
                .assets
                .into_iter()
                .map(|asset| GhAsset {
                    name: asset.name,
                    api_url: asset.url,
                    digest: asset.digest,
                    size: asset.size,
                })
                .collect(),
        })
    }

    async fn fetch_bytes(
        &self,
        url: &str,
        token: Option<&str>,
        accept: &str,
    ) -> Result<Vec<u8>, GithubError> {
        require_github_url(url).map_err(|_| GithubError::Failed)?;
        let mut request = self.http.get(url).header(reqwest::header::ACCEPT, accept);
        if let Some(token) = token_for_url(url, token) {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|_| GithubError::Failed)?;
        if !response.status().is_success() {
            return Err(GithubError::Failed);
        }
        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|_| GithubError::Failed)
    }
}

impl LiveGithub {
    async fn identity(&self, url: &str, token: Option<&str>) -> Result<Identity, GithubError> {
        require_github_url(url).map_err(|_| GithubError::Failed)?;
        let mut request = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|_| GithubError::Failed)?;
        if !response.status().is_success() {
            return Err(GithubError::Failed);
        }
        let body: UserBody = response.json().await.map_err(|_| GithubError::Failed)?;
        Ok(Identity {
            id: body.id,
            login: body.login,
        })
    }
}

/// The release credential is attached only to `api.github.com`. Redirect targets
/// get a signed URL from GitHub and must not also receive the token.
fn token_for_url<'a>(url: &str, token: Option<&'a str>) -> Option<&'a str> {
    let host = reqwest::Url::parse(url)
        .ok()?
        .host_str()?
        .to_ascii_lowercase();
    if host == "api.github.com" {
        token
    } else {
        None
    }
}

pub fn github_host_allowed(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host == "github.com"
        || host == "api.github.com"
        || host == "codeload.github.com"
        || host == "objects.githubusercontent.com"
        || host == "release-assets.githubusercontent.com"
        || host.ends_with(".githubusercontent.com")
}

pub fn require_github_url(url: &str) -> Result<(), ApiFailure> {
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        ApiFailure::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "bad_asset_url",
            "the release asset URL was rejected",
        )
    })?;
    if parsed.scheme() != "https"
        || parsed
            .host_str()
            .is_none_or(|host| !github_host_allowed(host))
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(ApiFailure::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "bad_asset_url",
            "the release asset URL was rejected",
        ));
    }
    Ok(())
}

pub async fn checksum_for_asset(
    github: &dyn Github,
    assets: &[GhAsset],
    asset_name: &str,
    token: Option<&str>,
) -> Result<String, ApiFailure> {
    checksum_from_sums(github, assets, asset_name, token).await
}

pub async fn download_asset(
    github: &dyn Github,
    release: &ReleaseMeta,
    assets_api_url: &str,
    token: Option<&str>,
) -> Result<Vec<u8>, ApiFailure> {
    require_github_url(assets_api_url)?;
    let bytes = github
        .fetch_bytes(assets_api_url, token, "application/octet-stream")
        .await
        .map_err(|_| {
            ApiFailure::new(
                axum::http::StatusCode::BAD_GATEWAY,
                "download_failed",
                "the release file could not be downloaded",
            )
        })?;
    if bytes.len() as u64 > 256 * 1024 * 1024 {
        return Err(ApiFailure::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "download_too_large",
            "the release file is larger than the server will proxy",
        ));
    }
    let got = sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(&release.sha256) {
        return Err(ApiFailure::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "checksum_mismatch",
            "the release file did not match its checksum",
        ));
    }
    Ok(bytes)
}

async fn checksum_from_sums(
    github: &dyn Github,
    assets: &[GhAsset],
    asset_name: &str,
    token: Option<&str>,
) -> Result<String, ApiFailure> {
    let Some(sums) = assets.iter().find(|asset| is_sums_name(&asset.name)) else {
        return Err(ApiFailure::new(
            axum::http::StatusCode::CONFLICT,
            "checksum_required",
            "the release does not publish a SHA-256 checksum",
        ));
    };
    require_github_url(&sums.api_url)?;
    let bytes = github
        .fetch_bytes(&sums.api_url, token, "application/octet-stream")
        .await
        .map_err(|_| {
            ApiFailure::new(
                axum::http::StatusCode::BAD_GATEWAY,
                "checksum_fetch_failed",
                "the checksum file could not be read",
            )
        })?;
    let text = String::from_utf8(bytes).map_err(|_| {
        ApiFailure::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "checksum_fetch_failed",
            "the checksum file could not be read",
        )
    })?;
    parse_sums(&text, asset_name).ok_or_else(|| {
        ApiFailure::new(
            axum::http::StatusCode::CONFLICT,
            "checksum_required",
            "the checksum file does not list this asset",
        )
    })
}

pub fn select_asset<'a>(assets: &'a [GhAsset], os: &str, arch: &str) -> Option<&'a GhAsset> {
    let os = os.to_ascii_lowercase();
    let arch = normalize_arch(arch);
    let mut best: Option<(i32, &GhAsset)> = None;
    for asset in assets {
        if is_sums_name(&asset.name) {
            continue;
        }
        let lower = asset.name.to_ascii_lowercase();
        if !matches_os(&lower, &os) || !matches_arch(&lower, arch) {
            continue;
        }
        let score = score_name(&lower, &os);
        if best.as_ref().is_none_or(|(current, _)| score > *current) {
            best = Some((score, asset));
        }
    }
    best.map(|(_, asset)| asset)
}

fn score_name(lower: &str, os: &str) -> i32 {
    let preferred = (os == "linux" && (lower.ends_with(".tar.gz") || lower.ends_with(".tgz")))
        || (os == "windows" && lower.ends_with(".zip"));
    if preferred {
        50
    } else if lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        40
    } else if lower.ends_with(".msi") || lower.ends_with(".deb") || lower.ends_with(".dmg") {
        10
    } else {
        20
    }
}

fn matches_os(lower: &str, os: &str) -> bool {
    match os {
        "linux" => {
            lower.contains("linux") || lower.ends_with(".deb") || lower.ends_with(".appimage")
        }
        "windows" => {
            lower.contains("windows")
                || lower.contains("win64")
                || lower.ends_with(".msi")
                || lower.ends_with(".exe")
        }
        "macos" => lower.contains("macos") || lower.contains("darwin") || lower.ends_with(".dmg"),
        _ => false,
    }
}

fn matches_arch(lower: &str, arch: &str) -> bool {
    match arch {
        "x86_64" => lower.contains("x86_64") || lower.contains("amd64") || lower.contains("x64"),
        "aarch64" => lower.contains("aarch64") || lower.contains("arm64"),
        _ => false,
    }
}

fn normalize_arch(arch: &str) -> &'static str {
    match arch {
        "amd64" | "x64" | "x86_64" => "x86_64",
        "arm64" | "aarch64" => "aarch64",
        _ => "other",
    }
}

fn is_sums_name(name: &str) -> bool {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "sha256sums" | "sha256sums.txt" | "sha256sum.txt" | "checksums.txt"
    )
}

pub fn parse_digest(digest: Option<&str>) -> Option<String> {
    let digest = digest?.trim();
    let hex = digest
        .strip_prefix("sha256:")
        .or_else(|| digest.strip_prefix("SHA256:"))
        .unwrap_or(digest);
    if hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(hex.to_ascii_lowercase())
    } else {
        None
    }
}

pub fn parse_sums(text: &str, filename: &str) -> Option<String> {
    let target = filename.to_ascii_lowercase();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let hex = parts.next()?;
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let name = parts.next()?.trim_start_matches('*');
        if name.eq_ignore_ascii_case(&target)
            || name.to_ascii_lowercase().ends_with(&format!("/{target}"))
        {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

fn urlencoding(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[derive(Deserialize)]
struct DeviceCodeBody {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Deserialize)]
struct TokenBody {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct UserBody {
    id: u64,
    login: String,
}

#[derive(Deserialize)]
struct ReleaseBody {
    tag_name: String,
    #[serde(default)]
    assets: Vec<AssetBody>,
}

#[derive(Deserialize)]
struct AssetBody {
    name: String,
    url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}
