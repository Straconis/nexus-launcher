//! Unauthenticated `GET /repos/{owner}/{repo}/releases/latest`.

use std::time::Duration;

use reqwest::header::{HeaderMap, ACCEPT, USER_AGENT};
use serde::Deserialize;

use crate::catalog::Platform;
use crate::checksum::{is_checksum_name, is_sums_filename, parse_digest};
use crate::model::PackageKind;

pub fn user_agent_value() -> String {
    format!(
        "nexus-launcher/{} (+https://github.com/Straconis/nexus-launcher)",
        env!("CARGO_PKG_VERSION")
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub html_url: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumRole {
    /// A `SHA256SUMS` style list that names many files.
    SumsList,
    /// A single-file sidecar such as `Tool.AppImage.sha256`.
    Sidecar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumAsset {
    pub name: String,
    pub url: String,
    pub role: ChecksumRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub kind: PackageKind,
    pub digest: Option<String>,
    pub checksum_asset: Option<ChecksumAsset>,
}

impl SelectedAsset {
    pub fn has_checksum(&self) -> bool {
        parse_digest(self.digest.as_deref()).is_some() || self.checksum_asset.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RateSnapshot {
    pub limit: Option<u32>,
    pub remaining: Option<u32>,
    pub reset_unix: Option<u64>,
}

impl RateSnapshot {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            limit: header_u32(headers, "x-ratelimit-limit"),
            remaining: header_u32(headers, "x-ratelimit-remaining"),
            reset_unix: header_u64(headers, "x-ratelimit-reset"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Release(Release),
    /// 404: no repository, or no published release. The card says Coming soon.
    Missing,
    RateLimited(String),
    Failed(String),
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhErrorBody {
    #[serde(default)]
    message: String,
}

pub fn http_client() -> Result<reqwest::blocking::Client, reqwest::Error> {
    reqwest::blocking::Client::builder()
        .user_agent(user_agent_value())
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(15 * 60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let too_many = attempt.previous().len() >= 10;
            let host = attempt.url().host_str().map(str::to_string);
            if too_many {
                return attempt.error("too many redirects");
            }
            match host {
                Some(host) if is_allowed_download_host(&host) => attempt.follow(),
                Some(host) => {
                    let message = format!("redirect host {host} is not allowed");
                    attempt.error(message)
                }
                None => attempt.error("redirect is missing a host"),
            }
        }))
        .build()
}

pub fn fetch_latest(
    client: &reqwest::blocking::Client,
    repo: &str,
) -> (FetchOutcome, RateSnapshot) {
    let url = match releases_url(repo) {
        Ok(url) => url,
        Err(err) => return (FetchOutcome::Failed(err), RateSnapshot::default()),
    };
    let response = match client
        .get(url)
        .header(ACCEPT, "application/vnd.github+json")
        .header(USER_AGENT, user_agent_value())
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
    {
        Ok(response) => response,
        Err(err) => {
            return (
                FetchOutcome::Failed(format!("network error: {err}")),
                RateSnapshot::default(),
            )
        }
    };
    let rate = RateSnapshot::from_headers(response.headers());
    let status = response.status().as_u16();
    let body = response.text().unwrap_or_default();
    (interpret(status, rate.remaining, &body), rate)
}

pub fn releases_url(repo: &str) -> Result<String, String> {
    if !crate::catalog::is_github_repo(repo) {
        return Err(format!("invalid GitHub repo '{repo}'"));
    }
    Ok(format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ))
}

pub fn interpret(status: u16, remaining: Option<u32>, body: &str) -> FetchOutcome {
    if status == 404 {
        return FetchOutcome::Missing;
    }
    let message = serde_json::from_str::<GhErrorBody>(body)
        .ok()
        .map(|body| body.message)
        .filter(|message| !message.is_empty());
    if status == 403 || status == 429 {
        let text = message.unwrap_or_else(|| format!("GitHub API status {status}"));
        let lower = text.to_ascii_lowercase();
        if remaining == Some(0) || lower.contains("rate limit") || lower.contains("secondary rate")
        {
            return FetchOutcome::RateLimited(text);
        }
        return FetchOutcome::Failed(text);
    }
    if (200..300).contains(&status) {
        return match serde_json::from_str::<GhRelease>(body) {
            Ok(release) => FetchOutcome::Release(Release {
                tag: release.tag_name,
                html_url: release.html_url,
                assets: release
                    .assets
                    .into_iter()
                    .map(|asset| Asset {
                        name: asset.name,
                        url: asset.browser_download_url,
                        size: asset.size,
                        digest: asset.digest,
                    })
                    .collect(),
            }),
            Err(err) => FetchOutcome::Failed(format!("could not parse release JSON: {err}")),
        };
    }
    FetchOutcome::Failed(message.unwrap_or_else(|| format!("GitHub API status {status}")))
}

pub fn banner_message(
    remaining: Option<u32>,
    reset_unix: Option<u64>,
    limited_detail: Option<&str>,
) -> Option<String> {
    let low = matches!(remaining, Some(n) if n < 10);
    if limited_detail.is_none() && !low {
        return None;
    }
    let prefix = if let Some(detail) = limited_detail {
        format!("GitHub API rate limit: {detail}")
    } else if remaining == Some(0) {
        "GitHub API rate limit reached.".to_string()
    } else {
        format!(
            "GitHub API rate limit is low ({} requests left).",
            remaining.unwrap_or(0)
        )
    };
    let when = reset_unix.and_then(|stamp| {
        chrono::DateTime::from_timestamp(stamp as i64, 0)
            .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
    });
    let budget = match (remaining, when) {
        (Some(n), Some(when)) => format!(" {n} requests remaining, resets {when}."),
        (None, Some(when)) => format!(" Resets {when}."),
        (Some(n), None) => format!(" {n} requests remaining."),
        (None, None) => String::new(),
    };
    Some(format!(
        "{prefix}{budget} Nexus Launcher does not send a token (60 requests/hour while unauthenticated)."
    ))
}

pub fn is_allowed_download_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host == "github.com"
        || host == "api.github.com"
        || host == "codeload.github.com"
        || host == "objects.githubusercontent.com"
        || host == "release-assets.githubusercontent.com"
        || host.ends_with(".githubusercontent.com")
        || host.ends_with(".github.com")
}

/// `true` when `latest` should replace an install recorded as `installed`.
pub fn is_newer_release(latest: &str, installed: &str) -> bool {
    let latest_norm = normalize_tag(latest);
    let installed_norm = normalize_tag(installed);
    if latest_norm.is_empty() || latest_norm == installed_norm {
        return false;
    }
    match (
        semver::Version::parse(&latest_norm),
        semver::Version::parse(&installed_norm),
    ) {
        (Ok(latest), Ok(installed)) => latest > installed,
        _ => true,
    }
}

fn normalize_tag(tag: &str) -> String {
    tag.trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_string()
}

#[derive(Clone, Copy)]
struct Classified {
    kind: PackageKind,
    os: Option<Platform>,
    arch: Option<&'static str>,
}

pub fn select_asset(
    assets: &[Asset],
    os: Platform,
    arch: &str,
    preference: &[String],
) -> Option<SelectedAsset> {
    let arch = normalize_arch(arch);
    let mut compatible: Vec<(i32, PackageKind, &Asset)> = Vec::new();
    for asset in assets {
        let Some(class) = classify_filename(&asset.name) else {
            continue;
        };
        if let Some(asset_os) = class.os {
            if asset_os != os {
                continue;
            }
        }
        if let Some(asset_arch) = class.arch {
            if asset_arch != arch {
                continue;
            }
        }
        if !is_allowed_download_host_url(&asset.url) {
            continue;
        }
        let mut score = match class.kind {
            PackageKind::Portable => 300,
            PackageKind::Archive => 220,
            PackageKind::Handoff => 80,
        };
        if class.arch == Some(arch) {
            score += 40;
        }
        if class.os == Some(os) {
            score += 15;
        }
        if os == Platform::Linux && asset.name.to_ascii_lowercase().ends_with(".appimage") {
            score += 25;
        }
        compatible.push((score, class.kind, asset));
    }
    if compatible.is_empty() {
        return None;
    }

    if !preference.is_empty() {
        for pref in preference {
            let needle = pref.to_ascii_lowercase();
            let mut matched: Vec<_> = compatible
                .iter()
                .filter(|(_, _, asset)| asset.name.to_ascii_lowercase().contains(&needle))
                .copied()
                .collect();
            if matched.is_empty() {
                continue;
            }
            matched.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.name.cmp(&b.2.name)));
            return Some(finish_selection(matched[0].2, matched[0].1, assets));
        }
    }

    compatible.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.name.cmp(&b.2.name)));
    Some(finish_selection(compatible[0].2, compatible[0].1, assets))
}

fn finish_selection(asset: &Asset, kind: PackageKind, assets: &[Asset]) -> SelectedAsset {
    let mut selected = SelectedAsset {
        name: asset.name.clone(),
        url: asset.url.clone(),
        size: asset.size,
        kind,
        digest: asset.digest.clone(),
        checksum_asset: None,
    };
    if parse_digest(selected.digest.as_deref()).is_some() {
        return selected;
    }
    let sidecar_names = [
        format!("{}.sha256", selected.name),
        format!("{}.sha256sum", selected.name),
    ];
    if let Some(sidecar) = assets.iter().find(|candidate| {
        sidecar_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&candidate.name))
            && is_allowed_download_host_url(&candidate.url)
    }) {
        selected.checksum_asset = Some(ChecksumAsset {
            name: sidecar.name.clone(),
            url: sidecar.url.clone(),
            role: ChecksumRole::Sidecar,
        });
        return selected;
    }
    if let Some(sums) = assets.iter().find(|candidate| {
        is_sums_filename(&candidate.name) && is_allowed_download_host_url(&candidate.url)
    }) {
        selected.checksum_asset = Some(ChecksumAsset {
            name: sums.name.clone(),
            url: sums.url.clone(),
            role: ChecksumRole::SumsList,
        });
    }
    selected
}

pub fn is_allowed_download_host_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .filter(|url| url.scheme() == "https")
        .and_then(|url| url.host_str().map(|host| host.to_string()))
        .is_some_and(|host| is_allowed_download_host(&host))
}

fn classify_filename(name: &str) -> Option<Classified> {
    let lower = name.to_ascii_lowercase();
    if is_checksum_name(&lower) || rejected_extension(&lower) {
        return None;
    }
    // bz2/xz/zst are classified as archives so a lone asset is not ignored.
    // Extraction rejects them and tells the user v1 only unpacks zip and tar.gz.
    let kind = if is_archive_name(&lower) {
        PackageKind::Archive
    } else if is_handoff_name(&lower) {
        PackageKind::Handoff
    } else if lower.ends_with(".exe") || lower.ends_with(".appimage") || is_portable_binary(&lower)
    {
        PackageKind::Portable
    } else {
        return None;
    };
    Some(Classified {
        kind,
        os: infer_os(&lower),
        arch: infer_arch(&lower),
    })
}

fn is_archive_name(lower: &str) -> bool {
    lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".zip")
        || lower.ends_with(".tar.bz2")
        || lower.ends_with(".tar.xz")
        || lower.ends_with(".tar.zst")
}

fn is_handoff_name(lower: &str) -> bool {
    lower.ends_with(".msi")
        || lower.ends_with(".dmg")
        || lower.ends_with(".deb")
        || (lower.ends_with(".exe") && is_setup_exe(lower))
}

fn rejected_extension(lower: &str) -> bool {
    const SKIP: &[&str] = &[
        "txt", "md", "markdown", "json", "sig", "asc", "pem", "html", "xml", "pdf", "png", "svg",
        "yml", "yaml", "toml", "css", "js", "map", "checksum",
    ];
    lower
        .rsplit_once('.')
        .map(|(_, ext)| SKIP.contains(&ext))
        .unwrap_or(false)
}

fn is_setup_exe(lower: &str) -> bool {
    let stem = lower.trim_end_matches(".exe");
    stem.contains("setup") || stem.contains("install")
}

fn is_portable_binary(lower: &str) -> bool {
    if !lower.contains('.') || lower.ends_with(".bin") || lower.ends_with(".run") {
        return true;
    }
    infer_os(lower).is_some() || infer_arch(lower).is_some()
}

fn infer_os(lower: &str) -> Option<Platform> {
    if lower.ends_with(".appimage")
        || lower.ends_with(".deb")
        || has_token(lower, "linux")
        || has_token(lower, "linux-gnu")
    {
        return Some(Platform::Linux);
    }
    if lower.ends_with(".msi")
        || lower.ends_with(".exe")
        || has_token(lower, "windows")
        || has_token(lower, "win64")
        || has_token(lower, "win32")
        || has_token(lower, "msvc")
    {
        return Some(Platform::Windows);
    }
    if lower.ends_with(".dmg")
        || has_token(lower, "macos")
        || has_token(lower, "darwin")
        || has_token(lower, "osx")
    {
        return Some(Platform::Macos);
    }
    None
}

fn infer_arch(lower: &str) -> Option<&'static str> {
    if lower.contains("aarch64") || has_token(lower, "arm64") {
        Some("aarch64")
    } else if lower.contains("x86_64") || has_token(lower, "amd64") || has_token(lower, "x64") {
        Some("x86_64")
    } else if has_token(lower, "i686") || has_token(lower, "i386") {
        Some("x86")
    } else {
        None
    }
}

fn has_token(lower: &str, token: &str) -> bool {
    // Tokens are split on punctuation, so `linux-gnu` is two tokens. Callers that
    // pass a hyphenated token also match the joined filename.
    if token.contains('-') && lower.contains(token) {
        return true;
    }
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == token)
}

fn normalize_arch(arch: &str) -> &'static str {
    match arch {
        "amd64" | "x64" | "x86_64" => "x86_64",
        "arm64" | "aarch64" => "aarch64",
        "i686" | "i386" | "x86" => "x86",
        // Unknown host arches fail closed against an asset that names an arch.
        _ => "other",
    }
}

fn header_u32(headers: &HeaderMap, name: &str) -> Option<u32> {
    headers.get(name)?.to_str().ok()?.parse().ok()
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.into(),
            url: format!("https://github.com/Straconis/tool/releases/download/v1/{name}"),
            size: 10,
            digest: None,
        }
    }

    #[test]
    fn prefers_appimage_over_deb_and_msi_over_nothing() {
        let assets = vec![
            asset("tool_1.0.0_amd64.deb"),
            asset("tool-x86_64.AppImage"),
            asset("tool-windows-setup.exe"),
        ];
        let selected = select_asset(&assets, Platform::Linux, "x86_64", &[]).unwrap();
        assert_eq!(selected.name, "tool-x86_64.AppImage");
        assert_eq!(selected.kind, PackageKind::Portable);

        let windows = vec![asset("Tool-setup.exe"), asset("Tool-win-x64.zip")];
        let selected = select_asset(&windows, Platform::Windows, "x86_64", &[]).unwrap();
        assert_eq!(selected.name, "Tool-win-x64.zip");
        assert_eq!(selected.kind, PackageKind::Archive);

        let mac = vec![asset("Tool.dmg")];
        let selected = select_asset(&mac, Platform::Macos, "aarch64", &[]).unwrap();
        assert_eq!(selected.kind, PackageKind::Handoff);
        assert!(select_asset(&mac, Platform::Linux, "x86_64", &[]).is_none());
    }

    #[test]
    fn preference_and_checksum_sources() {
        let mut appimage = asset("Tool.AppImage");
        appimage.digest =
            Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        let assets = vec![asset("Tool-linux.tar.gz"), appimage, asset("SHA256SUMS")];
        let prefer_archive =
            select_asset(&assets, Platform::Linux, "x86_64", &[".tar.gz".into()]).unwrap();
        assert_eq!(prefer_archive.name, "Tool-linux.tar.gz");
        assert_eq!(
            prefer_archive.checksum_asset.unwrap().role,
            ChecksumRole::SumsList
        );

        let digested = select_asset(&assets, Platform::Linux, "x86_64", &[]).unwrap();
        assert!(digested.digest.is_some());
        assert!(digested.checksum_asset.is_none());
        assert!(digested.has_checksum());

        let bare = vec![asset("Tool-linux-x86_64")];
        let selected = select_asset(&bare, Platform::Linux, "x86_64", &[]).unwrap();
        assert!(!selected.has_checksum());
        assert_eq!(selected.kind, PackageKind::Portable);
    }

    #[test]
    fn sidecar_is_attached() {
        let assets = vec![
            asset("Tool-linux.AppImage"),
            asset("Tool-linux.AppImage.sha256"),
        ];
        let selected = select_asset(&assets, Platform::Linux, "x86_64", &[]).unwrap();
        assert_eq!(selected.checksum_asset.unwrap().role, ChecksumRole::Sidecar);
    }

    #[test]
    fn interpret_release_missing_and_rate_limit() {
        let body = r#"{
            "tag_name": "v1.2.0",
            "html_url": "https://github.com/Straconis/tool/releases/tag/v1.2.0",
            "assets": [{
                "name": "tool.AppImage",
                "browser_download_url": "https://github.com/Straconis/tool/releases/download/v1.2.0/tool.AppImage",
                "size": 4,
                "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }]
        }"#;
        match interpret(200, Some(50), body) {
            FetchOutcome::Release(release) => {
                assert_eq!(release.tag, "v1.2.0");
                assert_eq!(release.assets.len(), 1);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            interpret(404, Some(40), "{\"message\":\"Not Found\"}"),
            FetchOutcome::Missing
        ));
        assert!(matches!(
            interpret(
                403,
                Some(0),
                "{\"message\":\"API rate limit exceeded for 1.2.3.4\"}"
            ),
            FetchOutcome::RateLimited(_)
        ));
        assert!(banner_message(Some(0), Some(1_700_000_000), None).is_some());
        assert!(banner_message(Some(40), None, None).is_none());
        assert!(banner_message(Some(40), None, Some("slow down")).is_some());
    }

    #[test]
    fn version_compare_and_hosts() {
        assert!(is_newer_release("v1.3.0", "v1.2.0"));
        assert!(!is_newer_release("v1.2.0", "1.2.0"));
        assert!(!is_newer_release("v1.0.0", "v1.2.0"));
        assert!(is_newer_release("build-9", "build-8"));
        assert!(is_allowed_download_host("objects.githubusercontent.com"));
        assert!(is_allowed_download_host(
            "release-assets.githubusercontent.com"
        ));
        assert!(!is_allowed_download_host("example.com"));
        assert!(!is_allowed_download_host_url("http://github.com/a"));
    }

    #[test]
    fn skips_checksum_files_and_arch_mismatch() {
        let assets = vec![
            asset("SHA256SUMS"),
            asset("tool-aarch64.AppImage"),
            asset("tool-x86_64.AppImage"),
        ];
        let selected = select_asset(&assets, Platform::Linux, "x86_64", &[]).unwrap();
        assert_eq!(selected.name, "tool-x86_64.AppImage");
    }
}
