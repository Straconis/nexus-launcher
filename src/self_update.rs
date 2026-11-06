//! Replace this running launcher from a newer GitHub release.
//!
//! Catalog apps still use `releases/latest`. This module lists releases so a
//! prerelease build can see a newer alpha. GitHub's latest endpoint hides those.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::catalog::Platform;
use crate::checksum::{self, parse_digest};
use crate::github::{self, ListOutcome, ListedRelease, SelectedAsset};
use crate::install::{self, InstallError};
use crate::model::PackageKind;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO: &str = "Straconis/nexus-launcher";

const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PAGES: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub tag: String,
    pub html_url: String,
    pub asset: SelectedAsset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    pub tag: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    Unsupported(String),
    Available { offer: Offer, skipped: Option<Skip> },
    Blocked { tag: String, reason: String },
    Failed(String),
    RateLimited(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub current: &'static str,
    pub outcome: Outcome,
    pub rate: Option<github::RateSnapshot>,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("{message} The current program was not changed.")]
    Rejected { message: String },
    #[error(
        "checksum mismatch: expected {expected}, got {got}. The download was deleted and the current program was not changed."
    )]
    Checksum { expected: String, got: String },
    #[error(
        "cannot write in {path}. Nexus Launcher will not ask for administrator rights. Move the program to a folder you can write, or download the release by hand. The current program was not changed."
    )]
    NotWritable { path: String },
    #[error("{message}")]
    Replace { message: String },
}

#[derive(Debug)]
pub struct ArchiveApply<'a> {
    pub archive: &'a Path,
    pub expected_sha256: &'a str,
    pub target_exe: &'a Path,
    pub staging_dir: &'a Path,
    pub state_path: &'a Path,
    pub tag: &'a str,
    pub asset_name: &'a str,
    pub platform: Platform,
    pub preflight: bool,
}

pub struct DownloadApply {
    pub offer: Offer,
    pub target_exe: PathBuf,
    pub staging_dir: PathBuf,
    pub state_path: PathBuf,
    pub platform: Platform,
}

pub fn official_asset_name(os: Platform, arch: &str) -> Option<&'static str> {
    let arch = match arch {
        "amd64" | "x64" | "x86_64" => "x86_64",
        other => other,
    };
    match (os, arch) {
        (Platform::Linux, "x86_64") => Some("nexus-launcher-linux-x86_64.tar.gz"),
        (Platform::Windows, "x86_64") => Some("nexus-launcher-windows-x86_64.zip"),
        _ => None,
    }
}

pub fn official_binary_name(os: Platform) -> &'static str {
    match os {
        Platform::Windows => "nexus-launcher.exe",
        Platform::Linux | Platform::Macos => "nexus-launcher",
    }
}

pub fn update_home() -> PathBuf {
    install::default_data_root()
        .unwrap_or_else(|_| std::env::temp_dir().join("nexus-launcher-self-update"))
}

pub fn staging_dir(home: &Path) -> PathBuf {
    home.join("self-update").join("staging")
}

pub fn state_path(home: &Path) -> PathBuf {
    home.join("self-update").join("applied.json")
}

pub fn running_executable() -> std::io::Result<PathBuf> {
    let path = std::env::current_exe()?;
    Ok(path.canonicalize().unwrap_or(path))
}

/// Ask GitHub which published release should replace this build, if any.
pub fn check(client: &reqwest::blocking::Client) -> Report {
    if let Ok(exe) = running_executable() {
        cleanup_backups(&exe);
    }
    let current = match Version::parse(VERSION) {
        Ok(version) => version,
        Err(err) => {
            return Report {
                current: VERSION,
                outcome: Outcome::Failed(format!("built-in version is not semver: {err}")),
                rate: None,
            };
        }
    };
    let os = crate::catalog::current_platform();
    let arch = crate::catalog::current_arch();
    if official_asset_name(os, arch).is_none() {
        return Report {
            current: VERSION,
            outcome: Outcome::Unsupported(unsupported_message(os, arch)),
            rate: None,
        };
    }
    let applied = read_applied_tag(&state_path(&update_home()));
    let mut releases = Vec::new();
    let mut rate = None;
    for page in 1..=MAX_PAGES {
        let (outcome, snapshot) = github::fetch_release_page(client, REPO, page, 100);
        rate = Some(snapshot);
        match outcome {
            ListOutcome::Releases(page_items) => {
                let full = page_items.len() == 100;
                releases.extend(page_items);
                if !full {
                    break;
                }
            }
            ListOutcome::Missing => {
                return Report {
                    current: VERSION,
                    outcome: Outcome::Failed(
                        "GitHub returned no release list for this repository.".into(),
                    ),
                    rate,
                };
            }
            ListOutcome::RateLimited(message) => {
                return Report {
                    current: VERSION,
                    outcome: Outcome::RateLimited(message),
                    rate,
                };
            }
            ListOutcome::Failed(message) => {
                return Report {
                    current: VERSION,
                    outcome: Outcome::Failed(message),
                    rate,
                };
            }
        }
    }
    Report {
        current: VERSION,
        outcome: select_update(&releases, &current, os, arch, applied.as_deref()),
        rate,
    }
}

pub fn select_update(
    releases: &[ListedRelease],
    current: &Version,
    os: Platform,
    arch: &str,
    applied_tag: Option<&str>,
) -> Outcome {
    if official_asset_name(os, arch).is_none() {
        return Outcome::Unsupported(unsupported_message(os, arch));
    }
    let mut ranked: Vec<(Version, &ListedRelease)> = Vec::new();
    for release in releases {
        if release.draft {
            continue;
        }
        let Some(version) = parse_release_version(&release.tag) else {
            continue;
        };
        if version <= *current {
            continue;
        }
        if !channel_accepts(current, &version, release.prerelease) {
            continue;
        }
        if suppresses(applied_tag, &release.tag, &version) {
            continue;
        }
        ranked.push((version, release));
    }
    ranked.sort_by(|left, right| right.0.cmp(&left.0));
    let mut skipped = None;
    for (_, release) in ranked {
        match offer_from(release, os, arch) {
            Ok(offer) => {
                return Outcome::Available { offer, skipped };
            }
            Err(reason) => {
                if skipped.is_none() {
                    skipped = Some(Skip {
                        tag: release.tag.clone(),
                        reason,
                    });
                }
            }
        }
    }
    if let Some(skip) = skipped {
        return Outcome::Blocked {
            tag: skip.tag,
            reason: skip.reason,
        };
    }
    Outcome::UpToDate
}

pub fn channel_accepts(
    current: &Version,
    release_version: &Version,
    github_prerelease: bool,
) -> bool {
    !(current.pre.is_empty() && (github_prerelease || !release_version.pre.is_empty()))
}

pub fn parse_release_version(tag: &str) -> Option<Version> {
    Version::parse(&normalize_tag(tag)).ok()
}

pub fn download_and_apply<F>(
    client: &reqwest::blocking::Client,
    request: &DownloadApply,
    mut progress: F,
) -> Result<PathBuf, UpdateError>
where
    F: FnMut(&str, Option<f32>),
{
    if request.target_exe.starts_with(&request.staging_dir) {
        return Err(UpdateError::Rejected {
            message: "refusing to replace a program inside the staging directory".into(),
        });
    }
    if request.staging_dir.exists() {
        fs::remove_dir_all(&request.staging_dir).map_err(|err| UpdateError::Rejected {
            message: format!("could not clear the staging folder: {err}"),
        })?;
    }
    fs::create_dir_all(&request.staging_dir).map_err(|err| UpdateError::Rejected {
        message: format!("could not create the staging folder: {err}"),
    })?;
    let archive = request.staging_dir.join(&request.offer.asset.name);
    let result = (|| {
        let sha = resolve_sha(client, &request.offer, &mut progress)?;
        progress("Downloading update", Some(0.08));
        install::download_file(
            client,
            &request.offer.asset.url,
            &archive,
            MAX_ARCHIVE_BYTES,
            |got, total| {
                let fraction = if total == 0 {
                    0.4
                } else {
                    0.08 + 0.7 * (got as f32 / total as f32)
                };
                progress(
                    &format!("Downloading {}", format_bytes(got)),
                    Some(fraction.min(0.8)),
                );
            },
        )
        .map_err(map_install)?;
        apply_verified_archive(&ArchiveApply {
            archive: &archive,
            expected_sha256: &sha,
            target_exe: &request.target_exe,
            staging_dir: &request.staging_dir,
            state_path: &request.state_path,
            tag: &request.offer.tag,
            asset_name: &request.offer.asset.name,
            platform: request.platform,
            preflight: true,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&request.staging_dir);
    }
    result.map(|_| request.target_exe.clone())
}

pub fn apply_verified_archive(spec: &ArchiveApply<'_>) -> Result<(), UpdateError> {
    if spec.target_exe.starts_with(spec.staging_dir) {
        return Err(UpdateError::Rejected {
            message: "refusing to replace a program inside the staging directory".into(),
        });
    }
    let original_len = fs::metadata(spec.target_exe)
        .map_err(|_| UpdateError::Rejected {
            message: format!(
                "the running program is not a file ({})",
                spec.target_exe.display()
            ),
        })?
        .len();
    if !spec.target_exe.is_file() || original_len == 0 {
        return Err(UpdateError::Rejected {
            message: "the running program is not a usable file".into(),
        });
    }
    match install::verify_sha256(spec.archive, spec.expected_sha256) {
        Ok(()) => {}
        Err(InstallError::Checksum { expected, got }) => {
            return Err(UpdateError::Checksum { expected, got });
        }
        Err(err) => return Err(map_install(err)),
    }
    let payload = spec.staging_dir.join("payload");
    install::extract_archive(spec.archive, &payload).map_err(map_install)?;
    let binary = find_official_binary(&payload, spec.platform)?;
    if spec.preflight {
        preflight(&binary, spec.staging_dir)?;
    }
    replace_executable(spec.target_exe, &binary)?;
    if let Err(err) = write_applied(spec) {
        // The new file is already in place. A missing state record must not
        // roll that back; version comparison still blocks a downgrade.
        let _ = err;
    }
    let _ = fs::remove_dir_all(spec.staging_dir);
    Ok(())
}

pub fn replace_executable(target: &Path, source: &Path) -> Result<(), UpdateError> {
    if !source.is_file() {
        return Err(UpdateError::Rejected {
            message: "the new build is missing".into(),
        });
    }
    let source_len = fs::metadata(source).map(|meta| meta.len()).unwrap_or(0);
    if source_len == 0 {
        return Err(UpdateError::Rejected {
            message: "the new build is empty".into(),
        });
    }
    if !target.is_file() {
        return Err(UpdateError::Rejected {
            message: format!("the running program is not a file ({})", target.display()),
        });
    }
    let parent = target.parent().ok_or_else(|| UpdateError::Rejected {
        message: "the running program has no folder".into(),
    })?;
    probe_writable(parent)?;
    let staged = sibling(target, ".new");
    if staged.exists() {
        fs::remove_file(&staged).map_err(|err| UpdateError::NotWritable {
            path: format!("{} ({err})", staged.display()),
        })?;
    }
    fs::copy(source, &staged).map_err(|err| UpdateError::Rejected {
        message: format!("could not copy the new build beside the program: {err}"),
    })?;
    sync_file(&staged).map_err(|err| UpdateError::Rejected {
        message: format!("could not finish writing the new build: {err}"),
    })?;
    set_executable(&staged).map_err(|err| UpdateError::Rejected {
        message: format!("could not mark the new build executable: {err}"),
    })?;
    match commit_swap(target, &staged) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&staged);
            Err(err)
        }
    }
}

pub fn cleanup_backups(exe: &Path) {
    let Some(name) = exe.file_name().and_then(|value| value.to_str()) else {
        return;
    };
    let Some(parent) = exe.parent() else {
        return;
    };
    let prefix = format!("{name}.old");
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if (file_name == prefix || file_name.starts_with(&format!("{prefix}-")))
            && entry.path().is_file()
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

pub fn spawn_restart(exe: &Path) -> std::io::Result<std::process::Child> {
    let mut command = Command::new(exe);
    command.args(std::env::args_os().skip(1));
    if let Ok(dir) = std::env::current_dir() {
        command.current_dir(dir);
    }
    command.spawn()
}

fn unsupported_message(os: Platform, arch: &str) -> String {
    format!(
        "self-update publishes Linux x86_64 (.tar.gz) and Windows x86_64 (.zip) only. This build is {} {arch}.",
        os.label()
    )
}

fn offer_from(release: &ListedRelease, os: Platform, arch: &str) -> Result<Offer, String> {
    let expected = official_asset_name(os, arch)
        .ok_or_else(|| "no official package for this platform".to_string())?;
    let Some(asset) = release.assets.iter().find(|asset| {
        asset.name.eq_ignore_ascii_case(expected)
            && github::is_allowed_download_host_url(&asset.url)
    }) else {
        return Err(format!("no {expected}"));
    };
    let checksum =
        github::checksum_asset_for(&asset.name, asset.digest.as_deref(), &release.assets);
    if parse_digest(asset.digest.as_deref()).is_none() && checksum.is_none() {
        return Err("no SHA-256 checksum".into());
    }
    Ok(Offer {
        tag: release.tag.clone(),
        html_url: release.html_url.clone(),
        asset: SelectedAsset {
            name: asset.name.clone(),
            url: asset.url.clone(),
            size: asset.size,
            kind: PackageKind::Archive,
            digest: asset.digest.clone(),
            checksum_asset: checksum,
        },
    })
}

fn suppresses(applied: Option<&str>, tag: &str, version: &Version) -> bool {
    let Some(applied) = applied.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    if applied.eq_ignore_ascii_case(tag) || normalize_tag(applied) == normalize_tag(tag) {
        return true;
    }
    parse_release_version(applied).is_some_and(|applied_version| &applied_version == version)
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

fn resolve_sha<F>(
    client: &reqwest::blocking::Client,
    offer: &Offer,
    progress: &mut F,
) -> Result<String, UpdateError>
where
    F: FnMut(&str, Option<f32>),
{
    if let Some(hex) = parse_digest(offer.asset.digest.as_deref()) {
        return Ok(hex);
    }
    let Some(checksum) = &offer.asset.checksum_asset else {
        return Err(UpdateError::Rejected {
            message: "this release has no SHA-256 checksum".into(),
        });
    };
    if !github::is_allowed_download_host_url(&checksum.url) {
        return Err(UpdateError::Rejected {
            message: format!("refusing checksum URL {}", checksum.url),
        });
    }
    progress("Downloading checksum", Some(0.02));
    let text =
        install::download_limited(client, &checksum.url, 2 * 1024 * 1024).map_err(map_install)?;
    checksum::parse_checksum_file(&text, &offer.asset.name).ok_or_else(|| UpdateError::Rejected {
        message: format!("checksum file does not list {}", offer.asset.name),
    })
}

fn find_official_binary(root: &Path, platform: Platform) -> Result<PathBuf, UpdateError> {
    let expected = official_binary_name(platform);
    let mut found = Vec::new();
    collect_files(root, &mut found).map_err(|err| UpdateError::Rejected {
        message: format!("could not read the archive: {err}"),
    })?;
    found.retain(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(expected))
            && path.is_file()
    });
    found.sort_by_key(|path| path.components().count());
    let Some(path) = found.into_iter().next() else {
        return Err(UpdateError::Rejected {
            message: format!("the archive does not contain {expected}"),
        });
    };
    if fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) == 0 {
        return Err(UpdateError::Rejected {
            message: format!("the archive contains an empty {expected}"),
        });
    }
    Ok(path)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn preflight(binary: &Path, staging: &Path) -> Result<(), UpdateError> {
    set_executable(binary).map_err(|err| UpdateError::Rejected {
        message: format!("could not mark the new build executable: {err}"),
    })?;
    let catalog = staging.join("preflight-apps.toml");
    fs::write(
        &catalog,
        "[[app]]\nid = \"stat-tracker\"\nname = \"Stat Tracker\"\n",
    )
    .map_err(|err| UpdateError::Rejected {
        message: format!("could not write a check catalog: {err}"),
    })?;
    let mut child = Command::new(binary)
        .arg("--check")
        .arg(&catalog)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| UpdateError::Rejected {
            message: format!("could not start the new build ({err})"),
        })?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > Duration::from_secs(30) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(UpdateError::Rejected {
                    message: "the new build did not finish --check within 30 seconds".into(),
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(err) => {
                return Err(UpdateError::Rejected {
                    message: format!("could not wait for the new build ({err})"),
                });
            }
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err(UpdateError::Rejected {
            message: format!("the new build failed --check ({status})"),
        })
    }
}

fn probe_writable(dir: &Path) -> Result<(), UpdateError> {
    let probe = dir.join(format!(".nexus-launcher-write-test-{}", std::process::id()));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(err) => Err(UpdateError::NotWritable {
            path: format!("{} ({err})", dir.display()),
        }),
    }
}

fn commit_swap(target: &Path, staged: &Path) -> Result<(), UpdateError> {
    let backup = backup_destination(target);
    if let Err(err) = fs::rename(target, &backup) {
        return Err(UpdateError::Replace {
            message: format!(
                "could not move the running program aside ({err}). The current program was not changed."
            ),
        });
    }
    if let Err(err) = fs::rename(staged, target) {
        let restored = fs::rename(&backup, target).is_ok();
        if restored {
            return Err(UpdateError::Replace {
                message: format!(
                    "could not move the new program into place ({err}). The previous program was restored."
                ),
            });
        }
        return Err(UpdateError::Replace {
            message: format!(
                "could not move the new program into place ({err}). The previous program is at {}.",
                backup.display()
            ),
        });
    }
    if let Err(err) = set_executable(target) {
        return Err(UpdateError::Replace {
            message: format!(
                "the new program is in place but could not be marked executable ({err})."
            ),
        });
    }
    Ok(())
}

fn backup_destination(target: &Path) -> PathBuf {
    let primary = sibling(target, ".old");
    if !primary.exists() || fs::remove_file(&primary).is_ok() {
        return primary;
    }
    sibling(
        target,
        &format!(
            ".old-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0)
        ),
    )
}

fn sibling(target: &Path, suffix: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "nexus-launcher".into());
    target.with_file_name(format!("{name}{suffix}"))
}

fn sync_file(path: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.sync_all()
}

fn set_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct AppliedRecord {
    schema: u32,
    applied_tag: String,
    asset_name: String,
    sha256: String,
    applied_at: String,
}

fn write_applied(spec: &ArchiveApply<'_>) -> std::io::Result<()> {
    if let Some(parent) = spec.state_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let record = AppliedRecord {
        schema: 1,
        applied_tag: spec.tag.to_string(),
        asset_name: spec.asset_name.to_string(),
        sha256: spec.expected_sha256.to_string(),
        applied_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    };
    let body = serde_json::to_vec_pretty(&record).unwrap_or_else(|_| b"{}".to_vec());
    let mut file = File::create(spec.state_path)?;
    file.write_all(&body)?;
    file.sync_all()?;
    Ok(())
}

fn read_applied_tag(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let record: AppliedRecord = serde_json::from_str(&text).ok()?;
    let tag = record.applied_tag.trim();
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

fn map_install(err: InstallError) -> UpdateError {
    match err {
        InstallError::Checksum { expected, got } => UpdateError::Checksum { expected, got },
        other => UpdateError::Rejected {
            message: other.to_string(),
        },
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes < KIB {
        format!("{bytes:.0} B")
    } else if bytes < KIB * KIB {
        format!("{:.1} KB", bytes / KIB)
    } else {
        format!("{:.1} MB", bytes / (KIB * KIB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Asset;
    use std::io::{Cursor, Read};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nexus-self-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.into(),
            url: format!("https://github.com/Straconis/nexus-launcher/releases/download/v1/{name}"),
            size: 12,
            digest: None,
        }
    }

    fn digested(name: &str) -> Asset {
        let mut asset = asset(name);
        asset.digest =
            Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        asset
    }

    fn listed(tag: &str, prerelease: bool, assets: Vec<Asset>) -> ListedRelease {
        ListedRelease {
            tag: tag.into(),
            html_url: format!("https://github.com/Straconis/nexus-launcher/releases/tag/{tag}"),
            draft: false,
            prerelease,
            assets,
        }
    }

    fn linux_archive() -> Asset {
        digested("nexus-launcher-linux-x86_64.tar.gz")
    }

    fn windows_archive() -> Asset {
        digested("nexus-launcher-windows-x86_64.zip")
    }

    #[test]
    fn prerelease_channel_sees_newer_alpha_that_latest_would_hide() {
        // releases/latest would return only the stable tag. That tag is older
        // than the running alpha, so a latest-only check would say up to date.
        let releases = vec![
            listed("v0.1.0", false, vec![linux_archive()]),
            listed("v0.1.2-alpha", true, vec![linux_archive()]),
        ];
        let current = version("0.1.1-alpha");
        let outcome = select_update(&releases, &current, Platform::Linux, "x86_64", None);
        match outcome {
            Outcome::Available { offer, skipped } => {
                assert_eq!(offer.tag, "v0.1.2-alpha");
                assert!(skipped.is_none());
                assert!(offer.asset.digest.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(version("0.1.0") < current);
    }

    #[test]
    fn stable_channel_ignores_newer_alphas_and_does_not_downgrade() {
        let releases = vec![
            listed("v0.2.0-alpha", true, vec![linux_archive()]),
            listed("v0.1.2", false, vec![linux_archive()]),
            listed("v0.1.0", false, vec![linux_archive()]),
            listed("nightly", false, vec![linux_archive()]),
        ];
        let outcome = select_update(
            &releases,
            &version("0.1.1"),
            Platform::Linux,
            "x86_64",
            None,
        );
        match outcome {
            Outcome::Available { offer, .. } => assert_eq!(offer.tag, "v0.1.2"),
            other => panic!("unexpected {other:?}"),
        }

        let same = select_update(
            &releases,
            &version("0.1.2"),
            Platform::Linux,
            "x86_64",
            None,
        );
        assert_eq!(same, Outcome::UpToDate);

        let newer = select_update(
            &[listed("v0.1.2-alpha", true, vec![linux_archive()])],
            &version("0.1.2"),
            Platform::Linux,
            "x86_64",
            None,
        );
        assert_eq!(newer, Outcome::UpToDate);
    }

    #[test]
    fn alpha_can_move_to_a_newer_stable_and_skips_equal_versions() {
        let stable = select_update(
            &[listed("v0.1.1", false, vec![linux_archive()])],
            &version("0.1.1-alpha"),
            Platform::Linux,
            "x86_64",
            None,
        );
        match stable {
            Outcome::Available { offer, .. } => assert_eq!(offer.tag, "v0.1.1"),
            other => panic!("unexpected {other:?}"),
        }
        let same = select_update(
            &[listed("v0.1.1-alpha", true, vec![linux_archive()])],
            &version("0.1.1-alpha"),
            Platform::Windows,
            "x86_64",
            None,
        );
        assert_eq!(same, Outcome::UpToDate);
    }

    #[test]
    fn applied_tag_suppresses_a_repeat_and_a_later_tag_still_shows() {
        let releases = vec![
            listed("v0.1.3-alpha", true, vec![linux_archive()]),
            listed("v0.1.2-alpha", true, vec![linux_archive()]),
        ];
        let held = select_update(
            &releases,
            &version("0.1.1-alpha"),
            Platform::Linux,
            "x86_64",
            Some("v0.1.2-alpha"),
        );
        match held {
            Outcome::Available { offer, .. } => assert_eq!(offer.tag, "v0.1.3-alpha"),
            other => panic!("unexpected {other:?}"),
        }
        let stuck = select_update(
            &[listed("0.1.2-alpha", true, vec![linux_archive()])],
            &version("0.1.1-alpha"),
            Platform::Linux,
            "x86_64",
            Some("v0.1.2-alpha"),
        );
        assert_eq!(stuck, Outcome::UpToDate);
    }

    #[test]
    fn missing_checksum_blocks_and_a_lower_verified_release_is_offered() {
        let releases = vec![
            listed(
                "v0.1.3-alpha",
                true,
                vec![asset("nexus-launcher-linux-x86_64.tar.gz")],
            ),
            listed(
                "v0.1.2-alpha",
                true,
                vec![linux_archive(), asset("SHA256SUMS.txt")],
            ),
        ];
        // The 0.1.2 asset also carries a digest, so the sums file is optional.
        match select_update(
            &releases,
            &version("0.1.1-alpha"),
            Platform::Linux,
            "x86_64",
            None,
        ) {
            Outcome::Available { offer, skipped } => {
                assert_eq!(offer.tag, "v0.1.2-alpha");
                let skipped = skipped.unwrap();
                assert_eq!(skipped.tag, "v0.1.3-alpha");
                assert!(skipped.reason.contains("SHA-256"));
            }
            other => panic!("unexpected {other:?}"),
        }

        match select_update(
            &[listed(
                "v0.1.3-alpha",
                true,
                vec![asset("nexus-launcher-linux-x86_64.tar.gz")],
            )],
            &version("0.1.1-alpha"),
            Platform::Linux,
            "x86_64",
            None,
        ) {
            Outcome::Blocked { tag, reason } => {
                assert_eq!(tag, "v0.1.3-alpha");
                assert!(reason.contains("SHA-256"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn only_the_official_archive_for_this_platform_is_eligible() {
        let releases = vec![listed(
            "v0.1.2-alpha",
            true,
            vec![
                digested("nexus-launcher-linux-x86_64.AppImage"),
                digested("nexus-launcher-linux-x86_64.deb"),
                digested("nexus-launcher-windows-x86_64.msi"),
                digested("nexus-launcher-macos-aarch64.tar.gz"),
                windows_archive(),
            ],
        )];
        match select_update(
            &releases,
            &version("0.1.1-alpha"),
            Platform::Windows,
            "x86_64",
            None,
        ) {
            Outcome::Available { offer, .. } => {
                assert_eq!(offer.asset.name, "nexus-launcher-windows-x86_64.zip");
            }
            other => panic!("unexpected {other:?}"),
        }
        match select_update(
            &releases,
            &version("0.1.1-alpha"),
            Platform::Linux,
            "x86_64",
            None,
        ) {
            Outcome::Blocked { reason, .. } => {
                assert!(reason.contains("nexus-launcher-linux-x86_64.tar.gz"))
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            select_update(
                &releases,
                &version("0.1.1-alpha"),
                Platform::Macos,
                "aarch64",
                None
            ),
            Outcome::Unsupported(_)
        ));
        assert_eq!(
            official_asset_name(Platform::Linux, "x86_64"),
            Some("nexus-launcher-linux-x86_64.tar.gz")
        );
        assert_eq!(
            official_asset_name(Platform::Windows, "amd64"),
            Some("nexus-launcher-windows-x86_64.zip")
        );
    }

    #[test]
    fn draft_and_disallowed_url_are_ignored() {
        let mut evil = digested("nexus-launcher-linux-x86_64.tar.gz");
        evil.url = "https://example.com/nexus-launcher-linux-x86_64.tar.gz".into();
        let mut draft = listed("v0.1.4-alpha", true, vec![linux_archive()]);
        draft.draft = true;
        let outcome = select_update(
            &[draft, listed("v0.1.2-alpha", true, vec![evil])],
            &version("0.1.1-alpha"),
            Platform::Linux,
            "x86_64",
            None,
        );
        assert!(matches!(
            outcome,
            Outcome::Blocked { .. } | Outcome::UpToDate
        ));
    }

    #[test]
    fn sums_file_counts_as_a_checksum_source() {
        let package = asset("nexus-launcher-linux-x86_64.tar.gz");
        let sums = asset("SHA256SUMS.txt");
        match select_update(
            &[listed("v0.1.2-alpha", true, vec![package, sums])],
            &version("0.1.1-alpha"),
            Platform::Linux,
            "x86_64",
            None,
        ) {
            Outcome::Available { offer, .. } => {
                assert_eq!(
                    offer.asset.checksum_asset.unwrap().role,
                    github::ChecksumRole::SumsList
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn replaces_an_open_file_and_leaves_neighbors_untouched() {
        let root = scratch("swap");
        let target = root.join(official_binary_name(crate::catalog::current_platform()));
        fs::write(&target, b"OLD-LAUNCHER").unwrap();
        fs::create_dir_all(root.join("catalog")).unwrap();
        fs::write(root.join("catalog").join("apps.toml"), b"custom catalog").unwrap();
        fs::create_dir_all(root.join("apps").join("stat-tracker")).unwrap();
        fs::write(
            root.join("apps").join("stat-tracker").join("install.json"),
            b"keep apps",
        )
        .unwrap();
        let source = root.join("incoming-bin");
        fs::write(&source, b"NEW-LAUNCHER").unwrap();

        let mut held = File::open(&target).unwrap();
        replace_executable(&target, &source).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"NEW-LAUNCHER");
        let mut from_old_handle = Vec::new();
        held.read_to_end(&mut from_old_handle).unwrap();
        assert_eq!(from_old_handle, b"OLD-LAUNCHER");
        assert_eq!(
            fs::read(root.join("catalog").join("apps.toml")).unwrap(),
            b"custom catalog"
        );
        assert_eq!(
            fs::read(root.join("apps").join("stat-tracker").join("install.json")).unwrap(),
            b"keep apps"
        );
        assert!(sibling(&target, ".old").is_file());
        cleanup_backups(&target);
        assert!(!sibling(&target, ".old").is_file());
        assert_eq!(fs::read(&target).unwrap(), b"NEW-LAUNCHER");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_second_rename_restores_the_original() {
        let root = scratch("rollback");
        let target = root.join("nexus-launcher");
        fs::write(&target, b"OLD").unwrap();
        let missing = root.join("does-not-exist");
        let err = commit_swap(&target, &missing).unwrap_err();
        assert!(err.to_string().contains("restored") || err.to_string().contains("previous"));
        assert_eq!(fs::read(&target).unwrap(), b"OLD");
        assert!(!sibling(&target, ".old").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_checksum_and_missing_binary_do_not_replace() {
        let root = scratch("reject");
        let target = root.join(official_binary_name(Platform::Linux));
        fs::write(&target, b"OLD").unwrap();
        fs::create_dir_all(root.join("catalog")).unwrap();
        fs::write(root.join("catalog").join("apps.toml"), b"custom").unwrap();
        let archive = root.join("nexus-launcher-linux-x86_64.tar.gz");
        write_targz(
            &archive,
            &[
                ("nexus-launcher-linux-x86_64/catalog/apps.toml", b"seed"),
                ("nexus-launcher-linux-x86_64/README.md", b"notes"),
            ],
        );
        let sha = checksum::sha256_file(&archive).unwrap();
        let staging = root.join("staging");
        let err = apply_verified_archive(&ArchiveApply {
            archive: &archive,
            expected_sha256: &sha,
            target_exe: &target,
            staging_dir: &staging,
            state_path: &root.join("applied.json"),
            tag: "v0.1.2-alpha",
            asset_name: "nexus-launcher-linux-x86_64.tar.gz",
            platform: Platform::Linux,
            preflight: false,
        })
        .unwrap_err();
        assert!(err.to_string().contains("does not contain"));
        assert_eq!(fs::read(&target).unwrap(), b"OLD");
        assert_eq!(
            fs::read(root.join("catalog").join("apps.toml")).unwrap(),
            b"custom"
        );
        assert!(!root.join("applied.json").exists());

        write_targz(
            &archive,
            &[(
                "nexus-launcher-linux-x86_64/nexus-launcher",
                b"#!/bin/sh\nexit 0\n",
            )],
        );
        let err = apply_verified_archive(&ArchiveApply {
            archive: &archive,
            expected_sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            target_exe: &target,
            staging_dir: &staging,
            state_path: &root.join("applied.json"),
            tag: "v0.1.2-alpha",
            asset_name: "nexus-launcher-linux-x86_64.tar.gz",
            platform: Platform::Linux,
            preflight: false,
        })
        .unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"));
        assert_eq!(fs::read(&target).unwrap(), b"OLD");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn verified_archive_replaces_only_the_executable() {
        let root = scratch("apply");
        let platform = crate::catalog::current_platform();
        if official_asset_name(platform, crate::catalog::current_arch()).is_none() {
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let binary_name = official_binary_name(platform);
        let target = root.join(binary_name);
        fs::write(&target, b"OLD").unwrap();
        fs::create_dir_all(root.join("catalog")).unwrap();
        fs::write(root.join("catalog").join("apps.toml"), b"custom catalog").unwrap();
        fs::create_dir_all(root.join("apps").join("snipscratch")).unwrap();
        fs::write(
            root.join("apps").join("snipscratch").join("install.json"),
            b"installed",
        )
        .unwrap();

        let asset_name = official_asset_name(platform, "x86_64").unwrap();
        let archive = root.join(asset_name);
        let inner = format!("pkg/{binary_name}");
        let payload = b"NEW-BYTES-FROM-ARCHIVE";
        match platform {
            Platform::Windows => write_zip(&archive, &[(&inner, payload)]),
            _ => write_targz(&archive, &[(&inner, payload)]),
        }
        let sha = checksum::sha256_file(&archive).unwrap();
        let staging = root.join("staging");
        apply_verified_archive(&ArchiveApply {
            archive: &archive,
            expected_sha256: &sha,
            target_exe: &target,
            staging_dir: &staging,
            state_path: &root.join("state").join("applied.json"),
            tag: "v0.1.2-alpha",
            asset_name,
            platform,
            preflight: false,
        })
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), payload);
        assert_eq!(
            fs::read(root.join("catalog").join("apps.toml")).unwrap(),
            b"custom catalog"
        );
        assert_eq!(
            fs::read(root.join("apps").join("snipscratch").join("install.json")).unwrap(),
            b"installed"
        );
        let record: AppliedRecord = serde_json::from_str(
            &fs::read_to_string(root.join("state").join("applied.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(record.applied_tag, "v0.1.2-alpha");
        assert_eq!(record.sha256, sha);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn preflight_failure_keeps_the_old_executable() {
        let root = scratch("preflight");
        let platform = crate::catalog::current_platform();
        if official_asset_name(platform, crate::catalog::current_arch()).is_none() {
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let binary_name = official_binary_name(platform);
        let target = root.join(binary_name);
        fs::write(&target, b"OLD").unwrap();
        let asset_name = official_asset_name(platform, "x86_64").unwrap();
        let archive = root.join(asset_name);
        let inner = format!("pkg/{binary_name}");
        match platform {
            Platform::Windows => write_zip(&archive, &[(&inner, b"not a real executable")]),
            _ => write_targz(&archive, &[(&inner, b"not a real executable")]),
        }
        let sha = checksum::sha256_file(&archive).unwrap();
        let err = apply_verified_archive(&ArchiveApply {
            archive: &archive,
            expected_sha256: &sha,
            target_exe: &target,
            staging_dir: &root.join("staging"),
            state_path: &root.join("applied.json"),
            tag: "v0.1.2-alpha",
            asset_name,
            platform,
            preflight: true,
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("not changed"),
            "unexpected error: {err}"
        );
        assert_eq!(fs::read(&target).unwrap(), b"OLD");
        assert!(!root.join("applied.json").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn zip_slip_does_not_overwrite_the_executable() {
        let root = scratch("slip");
        let target = root.join("nexus-launcher");
        fs::write(&target, b"OLD").unwrap();
        let archive = root.join("evil.zip");
        write_zip(&archive, &[("../nexus-launcher", b"PWNED")]);
        let sha = checksum::sha256_file(&archive).unwrap();
        let err = apply_verified_archive(&ArchiveApply {
            archive: &archive,
            expected_sha256: &sha,
            target_exe: &target,
            staging_dir: &root.join("staging"),
            state_path: &root.join("applied.json"),
            tag: "v0.1.2-alpha",
            asset_name: "evil.zip",
            platform: Platform::Linux,
            preflight: false,
        })
        .unwrap_err();
        assert!(err.to_string().contains("not changed") || err.to_string().contains("unsafe"));
        assert_eq!(fs::read(&target).unwrap(), b"OLD");
        assert_eq!(fs::read(root.join("nexus-launcher")).unwrap(), b"OLD");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_directory_leaves_the_executable() {
        if running_as_root() {
            return;
        }
        let root = scratch("locked");
        let dir = root.join("locked");
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("nexus-launcher");
        fs::write(&target, b"OLD").unwrap();
        let source = root.join("new");
        fs::write(&source, b"NEW").unwrap();
        let _reset = ModeReset(&dir);
        let mut permissions = fs::metadata(&dir).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o555);
        fs::set_permissions(&dir, permissions).unwrap();
        let err = replace_executable(&target, &source).unwrap_err();
        assert!(err.to_string().contains("not ask for administrator"));
        assert_eq!(fs::read(&target).unwrap(), b"OLD");
        assert!(!dir.join("nexus-launcher.new").exists());
    }

    #[cfg(unix)]
    struct ModeReset<'a>(&'a Path);

    #[cfg(unix)]
    impl Drop for ModeReset<'_> {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(self.0).unwrap().permissions();
            permissions.set_mode(0o755);
            let _ = fs::set_permissions(self.0, permissions);
        }
    }

    #[cfg(unix)]
    fn running_as_root() -> bool {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .is_some_and(|text| text.trim() == "0")
    }

    fn write_targz(path: &Path, files: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, *name, Cursor::new(*data))
                .unwrap();
        }
        builder.finish().unwrap();
    }

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        for (name, data) in files {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
}
