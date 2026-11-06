//! Per-user installs: portable files, archives, and OS handoff for installers.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::checksum::{self, is_checksum_name, parse_digest};
use crate::github::{self, ChecksumAsset};
use crate::model::PackageKind;

pub const RECORD_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRecord {
    #[serde(default = "default_schema")]
    pub schema: u32,
    pub app_id: String,
    pub version: String,
    pub asset_name: String,
    #[serde(default)]
    pub sha256: Option<String>,
    pub kind: PackageKind,
    pub install_dir: PathBuf,
    #[serde(default)]
    pub launch_path: Option<PathBuf>,
    #[serde(default)]
    pub installer_path: Option<PathBuf>,
    pub installed_at: String,
}

fn default_schema() -> u32 {
    RECORD_SCHEMA
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    NotInstalled,
    Ready(InstallRecord),
    /// The installer was downloaded and handed to the OS. We do not know if it finished.
    InstallerStaged(InstallRecord),
    Broken {
        record: InstallRecord,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub app_id: String,
    pub version: String,
    pub executable_hint: Option<String>,
    pub asset_name: String,
    pub url: String,
    pub kind: PackageKind,
    pub digest: Option<String>,
    pub checksum_asset: Option<ChecksumAsset>,
    pub allow_unverified: bool,
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    #[error("download failed: {0}")]
    Download(String),
    #[error("checksum mismatch: expected {expected}, got {got}")]
    Checksum { expected: String, got: String },
    #[error("checksum file did not list {0}")]
    ChecksumMissingEntry(String),
    #[error("refusing to install without a SHA-256 checksum")]
    ChecksumRequired,
    #[error("checksum file is not valid text")]
    ChecksumFormat,
    #[error("unsafe archive path: {0}")]
    UnsafePath(String),
    #[error("unsupported archive '{0}' (v1 extracts zip and tar.gz)")]
    UnsupportedArchive(String),
    #[error("{0}")]
    NoExecutable(String),
    #[error("could not open the installer with the operating system: {0}")]
    Handoff(String),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("{0}")]
    Message(String),
}

/// `directories` data dir for Straconis / Nexus Launcher, plus nothing else.
pub fn default_data_root() -> Result<PathBuf, InstallError> {
    directories::ProjectDirs::from("org", "Straconis", "nexus-launcher")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .ok_or_else(|| InstallError::Message("no per-user data directory on this platform".into()))
}

pub fn detect(root: &Path, app_id: &str) -> InstallState {
    let path = root.join("apps").join(app_id).join("install.json");
    let Ok(text) = fs::read_to_string(path) else {
        return InstallState::NotInstalled;
    };
    let Ok(record) = serde_json::from_str::<InstallRecord>(&text) else {
        return InstallState::NotInstalled;
    };
    match record.kind {
        PackageKind::Handoff => match &record.installer_path {
            Some(installer) if installer.is_file() => InstallState::InstallerStaged(record),
            _ => InstallState::Broken {
                reason: "Installer file is missing.".into(),
                record,
            },
        },
        PackageKind::Portable | PackageKind::Archive => match &record.launch_path {
            Some(launch) if launch.is_file() || launch.is_dir() => InstallState::Ready(record),
            Some(_) => InstallState::Broken {
                reason: "Launch file is missing.".into(),
                record,
            },
            None => InstallState::Broken {
                reason: "No launch path was recorded.".into(),
                record,
            },
        },
    }
}

pub fn install_package<F>(
    root: &Path,
    request: &InstallRequest,
    client: &reqwest::blocking::Client,
    mut progress: F,
) -> Result<InstallRecord, InstallError>
where
    F: FnMut(&str, Option<f32>),
{
    if request.app_id.contains('/')
        || request.app_id.contains('\\')
        || request.app_id.contains("..")
    {
        return Err(InstallError::Message("invalid app id".into()));
    }
    let safe_name = sanitize_filename(&request.asset_name)?;
    github::is_allowed_download_host_url(&request.url)
        .then_some(())
        .ok_or_else(|| InstallError::Download(format!("refusing download URL {}", request.url)))?;

    let app_dir = root.join("apps").join(&request.app_id);
    fs::create_dir_all(&app_dir)?;
    let incoming = app_dir.join("incoming");
    if incoming.exists() {
        fs::remove_dir_all(&incoming)?;
    }
    fs::create_dir_all(&incoming)?;

    let mut swapped = false;
    let result = install_into(
        &app_dir,
        &incoming,
        &safe_name,
        request,
        client,
        &mut progress,
        &mut swapped,
    );
    if result.is_err() && !swapped && incoming.exists() {
        let _ = fs::remove_dir_all(&incoming);
    }
    result
}

fn install_into<F>(
    app_dir: &Path,
    incoming: &Path,
    safe_name: &str,
    request: &InstallRequest,
    client: &reqwest::blocking::Client,
    progress: &mut F,
    swapped: &mut bool,
) -> Result<InstallRecord, InstallError>
where
    F: FnMut(&str, Option<f32>),
{
    let sha = resolve_sha256(client, request, progress)?;
    if sha.is_none() && !request.allow_unverified {
        return Err(InstallError::ChecksumRequired);
    }

    let archive_path = incoming.join(safe_name);
    progress("Downloading", Some(0.05));
    download_file(
        client,
        &request.url,
        &archive_path,
        u64::MAX,
        |got, total| {
            let fraction = if total == 0 {
                0.4
            } else {
                0.05 + 0.75 * (got as f32 / total as f32)
            };
            progress(
                &format!("Downloading {}", format_bytes(got)),
                Some(fraction.min(0.85)),
            );
        },
    )?;
    if let Some(expected) = &sha {
        progress("Verifying SHA-256", Some(0.88));
        verify_sha256(&archive_path, expected)?;
    }

    let relative_launch = match request.kind {
        PackageKind::Portable => {
            make_executable(&archive_path)?;
            Some(PathBuf::from(safe_name))
        }
        PackageKind::Archive => {
            progress("Extracting", Some(0.92));
            let payload = incoming.join("payload");
            extract_archive(&archive_path, &payload)?;
            let launch = find_launchable(
                &payload,
                request.executable_hint.as_deref(),
                &request.app_id,
            )?;
            make_executable(&launch)?;
            let relative = launch
                .strip_prefix(&payload)
                .map_err(|_| InstallError::Message("launch path escaped the archive".into()))?;
            let _ = fs::remove_file(&archive_path);
            Some(Path::new("payload").join(relative))
        }
        PackageKind::Handoff => None,
    };

    progress("Finishing install", Some(0.97));
    promote_incoming(app_dir)?;
    *swapped = true;
    let current = app_dir.join("current");
    let launch_path = relative_launch.map(|relative| current.join(relative));
    let installer_path = if request.kind == PackageKind::Handoff {
        Some(current.join(safe_name))
    } else {
        None
    };
    let record = InstallRecord {
        schema: RECORD_SCHEMA,
        app_id: request.app_id.clone(),
        version: request.version.clone(),
        asset_name: safe_name.to_string(),
        sha256: sha,
        kind: request.kind,
        install_dir: app_dir.to_path_buf(),
        launch_path,
        installer_path: installer_path.clone(),
        installed_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    };
    write_record(&app_dir.join("install.json"), &record)?;
    if let Some(path) = &installer_path {
        hand_to_os(path).map_err(|err| InstallError::Handoff(err.to_string()))?;
    }
    progress("Installed", Some(1.0));
    Ok(record)
}

pub fn spawn_launch(program: &Path, cwd: &Path) -> std::io::Result<Option<std::process::Child>> {
    if program.is_dir() {
        open::that(program)?;
        return Ok(None);
    }
    let child = std::process::Command::new(program)
        .current_dir(cwd)
        .spawn()?;
    Ok(Some(child))
}

pub fn hand_to_os(path: &Path) -> std::io::Result<()> {
    open::that(path)
}

fn resolve_sha256<F>(
    client: &reqwest::blocking::Client,
    request: &InstallRequest,
    progress: &mut F,
) -> Result<Option<String>, InstallError>
where
    F: FnMut(&str, Option<f32>),
{
    if let Some(hex) = parse_digest(request.digest.as_deref()) {
        return Ok(Some(hex));
    }
    let Some(checksum) = &request.checksum_asset else {
        return Ok(None);
    };
    download_checksum(client, checksum, &request.asset_name, progress)
}

fn download_checksum<F>(
    client: &reqwest::blocking::Client,
    checksum: &ChecksumAsset,
    asset_name: &str,
    progress: &mut F,
) -> Result<Option<String>, InstallError>
where
    F: FnMut(&str, Option<f32>),
{
    if !github::is_allowed_download_host_url(&checksum.url) {
        return Err(InstallError::Download(format!(
            "refusing checksum URL {}",
            checksum.url
        )));
    }
    progress("Downloading checksum", Some(0.02));
    let text = download_limited(client, &checksum.url, 2 * 1024 * 1024)?;
    checksum::parse_checksum_file(&text, asset_name)
        .map(Some)
        .ok_or_else(|| InstallError::ChecksumMissingEntry(asset_name.to_string()))
}

pub(crate) fn download_limited(
    client: &reqwest::blocking::Client,
    url: &str,
    limit: usize,
) -> Result<String, InstallError> {
    let response = client.get(url).send()?;
    if !response.status().is_success() {
        return Err(InstallError::Download(format!(
            "checksum HTTP {}",
            response.status()
        )));
    }
    let mut buf = Vec::new();
    response.take(limit as u64 + 1).read_to_end(&mut buf)?;
    if buf.len() > limit {
        return Err(InstallError::Message("checksum file is too large".into()));
    }
    String::from_utf8(buf).map_err(|_| InstallError::ChecksumFormat)
}

pub(crate) fn download_file<F>(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    max_bytes: u64,
    mut progress: F,
) -> Result<(), InstallError>
where
    F: FnMut(u64, u64),
{
    let mut response = client.get(url).send()?;
    if !response.status().is_success() {
        return Err(InstallError::Download(format!(
            "HTTP {}",
            response.status()
        )));
    }
    let total = response.content_length().unwrap_or(0);
    if total > max_bytes {
        return Err(InstallError::Message(format!(
            "download is larger than {max_bytes} bytes"
        )));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = dest.with_extension("partial");
    let mut file = File::create(&temporary)?;
    let mut buf = [0_u8; 64 * 1024];
    let mut got = 0_u64;
    loop {
        let read = response.read(&mut buf)?;
        if read == 0 {
            break;
        }
        file.write_all(&buf[..read])?;
        got += read as u64;
        if got > max_bytes {
            drop(file);
            let _ = fs::remove_file(&temporary);
            return Err(InstallError::Message(format!(
                "download is larger than {max_bytes} bytes"
            )));
        }
        progress(got, total);
    }
    file.sync_all()?;
    if dest.exists() {
        fs::remove_file(dest)?;
    }
    fs::rename(&temporary, dest)?;
    Ok(())
}

pub(crate) fn verify_sha256(path: &Path, expected: &str) -> Result<(), InstallError> {
    let got = checksum::sha256_file(path)?;
    if got.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        let _ = fs::remove_file(path);
        Err(InstallError::Checksum {
            expected: expected.trim().to_ascii_lowercase(),
            got,
        })
    }
}

pub(crate) fn extract_archive(archive: &Path, dest: &Path) -> Result<(), InstallError> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;
    let name = archive
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let result = if name.ends_with(".zip") {
        extract_zip(archive, dest)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(archive, dest)
    } else {
        Err(InstallError::UnsupportedArchive(name))
    };
    if result.is_err() {
        let _ = fs::remove_dir_all(dest);
    }
    result
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), InstallError> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_string();
        if name.is_empty() {
            continue;
        }
        let relative = safe_relative(Path::new(&name))?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&relative);
        if entry.is_dir() || name.ends_with('/') {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = File::create(&out)?;
        std::io::copy(&mut entry, &mut outfile)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            if mode & 0o111 != 0 {
                fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o777))?;
            }
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), InstallError> {
    let file = File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let relative = safe_relative(&path)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&relative);
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(InstallError::UnsafePath(format!(
                "links are not extracted ({})",
                relative.display()
            )));
        }
        if kind.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = File::create(&out)?;
        std::io::copy(&mut entry, &mut outfile)?;
        #[cfg(unix)]
        if let Ok(mode) = entry.header().mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(())
}

pub fn find_launchable(
    root: &Path,
    hint: Option<&str>,
    app_id: &str,
) -> Result<PathBuf, InstallError> {
    if let Some(hint) = hint {
        let relative = safe_relative(Path::new(hint))?;
        if relative.as_os_str().is_empty() {
            return Err(InstallError::NoExecutable(
                "executable hint is empty".into(),
            ));
        }
        let path = root.join(relative);
        if path.is_file() || (path.is_dir() && is_app_bundle(&path)) {
            return Ok(path);
        }
        return Err(InstallError::NoExecutable(format!(
            "catalog executable `{hint}` was not in the archive"
        )));
    }

    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    let candidates: Vec<PathBuf> = files
        .into_iter()
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            !is_checksum_name(name) && !is_archive_filename(name)
        })
        .collect();
    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    let mut scored: Vec<(i32, PathBuf)> = candidates
        .into_iter()
        .map(|path| (score_candidate(&path, root, app_id), path))
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    if let Some((_, path)) = scored.into_iter().next() {
        return Ok(path);
    }
    Err(InstallError::NoExecutable(
        "no executable found in the archive; set `executable` in the catalog".into(),
    ))
}

fn score_candidate(path: &Path, root: &Path, app_id: &str) -> i32 {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let id = app_id.to_ascii_lowercase();
    let mut score = 0;
    if name.contains(&id) || name.replace('-', "").contains(&id.replace('-', "")) {
        score += 50;
    }
    if name.ends_with(".appimage") || name.ends_with(".exe") || name.ends_with(".app") {
        score += 30;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.is_file() {
            if let Ok(meta) = path.metadata() {
                if meta.permissions().mode() & 0o111 != 0 {
                    score += 20;
                }
            }
        }
    }
    let depth = path
        .strip_prefix(root)
        .map(|relative| relative.components().count())
        .unwrap_or(8);
    score - depth as i32
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if is_app_bundle(&path) {
                out.push(path);
            } else {
                collect_files(&path, out)?;
            }
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn is_app_bundle(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("app")
}

fn is_archive_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar.bz2")
        || lower.ends_with(".tar.xz")
}

fn promote_incoming(app_dir: &Path) -> std::io::Result<()> {
    let incoming = app_dir.join("incoming");
    let current = app_dir.join("current");
    let previous = app_dir.join("previous");
    if previous.exists() {
        fs::remove_dir_all(&previous)?;
    }
    if current.exists() {
        fs::rename(&current, &previous)?;
    }
    fs::rename(&incoming, &current)?;
    if previous.exists() {
        let _ = fs::remove_dir_all(&previous);
    }
    Ok(())
}

fn write_record(path: &Path, record: &InstallRecord) -> Result<(), InstallError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(record)
        .map_err(|err| InstallError::Message(err.to_string()))?;
    fs::write(&temporary, body)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn sanitize_filename(name: &str) -> Result<String, InstallError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(InstallError::UnsafePath(format!(
            "asset name '{name}' is not a single file name"
        )));
    }
    Ok(name.to_string())
}

fn safe_relative(path: &Path) -> Result<PathBuf, InstallError> {
    if path.is_absolute() {
        return Err(InstallError::UnsafePath(path.display().to_string()));
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => {
                return Err(InstallError::UnsafePath(path.display().to_string()));
            }
        }
    }
    Ok(clean)
}

fn make_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.is_file() {
            let mut permissions = fs::metadata(path)?.permissions();
            permissions.set_mode(permissions.mode() | 0o755);
            fs::set_permissions(path, permissions)?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
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
    use std::io::Cursor;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nexus-launcher-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn zip_and_targz_round_trip_and_reject_zip_slip() {
        let root = scratch("zip");
        let zip_path = root.join("tool.zip");
        {
            let file = File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("bin/stat-tracker", options).unwrap();
            writer.write_all(b"echo stat\n").unwrap();
            writer.finish().unwrap();
        }
        let dest = root.join("out");
        extract_archive(&zip_path, &dest).unwrap();
        let launch = find_launchable(&dest, None, "stat-tracker").unwrap();
        assert_eq!(fs::read(&launch).unwrap(), b"echo stat\n");
        let hinted = find_launchable(&dest, Some("bin/stat-tracker"), "stat-tracker").unwrap();
        assert_eq!(hinted, launch);

        let evil_zip = root.join("evil.zip");
        {
            let file = File::create(&evil_zip).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("../evil.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"nope").unwrap();
            writer.finish().unwrap();
        }
        let evil_dest = root.join("nested").join("dest");
        let err = extract_archive(&evil_zip, &evil_dest).unwrap_err();
        assert!(err.to_string().contains("evil"));
        assert!(!root.join("evil.txt").exists());
        assert!(!evil_dest.exists());

        let tar_path = root.join("tool.tar.gz");
        {
            let file = File::create(&tar_path).unwrap();
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let data = b"frame";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "frameweaver", Cursor::new(data))
                .unwrap();
            builder.finish().unwrap();
        }
        let tar_dest = root.join("tar");
        extract_archive(&tar_path, &tar_dest).unwrap();
        assert_eq!(fs::read(tar_dest.join("frameweaver")).unwrap(), b"frame");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn detect_ready_broken_and_staged() {
        let root = scratch("detect");
        assert!(matches!(
            detect(&root, "stat-tracker"),
            InstallState::NotInstalled
        ));
        let app = root.join("apps").join("stat-tracker");
        let current = app.join("current");
        fs::create_dir_all(&current).unwrap();
        let binary = current.join("stat-tracker");
        fs::write(&binary, b"bin").unwrap();
        let record = InstallRecord {
            schema: 1,
            app_id: "stat-tracker".into(),
            version: "v1.0.0".into(),
            asset_name: "stat-tracker".into(),
            sha256: None,
            kind: PackageKind::Portable,
            install_dir: app.clone(),
            launch_path: Some(binary.clone()),
            installer_path: None,
            installed_at: "2026-01-01T00:00:00Z".into(),
        };
        write_record(&app.join("install.json"), &record).unwrap();
        assert!(matches!(
            detect(&root, "stat-tracker"),
            InstallState::Ready(_)
        ));
        fs::remove_file(&binary).unwrap();
        assert!(matches!(
            detect(&root, "stat-tracker"),
            InstallState::Broken { .. }
        ));

        let staged = root.join("apps").join("snaplapse");
        let staged_current = staged.join("current");
        fs::create_dir_all(&staged_current).unwrap();
        let msi = staged_current.join("SnapLapse.msi");
        fs::write(&msi, b"msi").unwrap();
        write_record(
            &staged.join("install.json"),
            &InstallRecord {
                schema: 1,
                app_id: "snaplapse".into(),
                version: "v0.1.0".into(),
                asset_name: "SnapLapse.msi".into(),
                sha256: None,
                kind: PackageKind::Handoff,
                install_dir: staged.clone(),
                launch_path: None,
                installer_path: Some(msi),
                installed_at: "2026-01-01T00:00:00Z".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            detect(&root, "snaplapse"),
            InstallState::InstallerStaged(_)
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn data_root_is_user_scoped() {
        let root = default_data_root().unwrap();
        assert!(root.is_absolute());
        let rendered = root.to_string_lossy();
        assert!(
            rendered.contains("nexus-launcher"),
            "unexpected data dir {root:?}"
        );
    }

    #[test]
    fn checksum_mismatch_removes_the_file() {
        let root = scratch("sum");
        let file = root.join("payload.bin");
        fs::write(&file, b"abc").unwrap();
        let err = verify_sha256(
            &file,
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"));
        assert!(!file.exists());
        let _ = fs::remove_dir_all(&root);
    }
}
