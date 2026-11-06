//! Optional server URL. The file holds no tokens and no GitHub client secret.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub server_url: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct FileConfig {
    server_url: String,
}

pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "Straconis", "nexus-launcher")
        .map(|dirs| dirs.config_dir().join("central.toml"))
}

pub fn load_config(path: &Path) -> Result<Option<ServerConfig>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let file: FileConfig = toml::from_str(&text).map_err(|err| err.to_string())?;
    let server_url = validate_server_url(&file.server_url)?;
    Ok(Some(ServerConfig { server_url }))
}

/// Saves only the service address. Credentials are never written to this file.
pub fn save_config(path: &Path, value: &str) -> Result<(), String> {
    let server_url = validate_server_url(value)?;
    let text = toml::to_string(&FileConfig { server_url }).map_err(|err| err.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, text).map_err(|err| err.to_string())
}

pub fn validate_server_url(value: &str) -> Result<String, String> {
    let url =
        reqwest::Url::parse(value.trim()).map_err(|_| "server URL is not a URL".to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("server URL must be http or https".into());
    }
    if url.host_str().is_none() {
        return Err("server URL needs a host".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("server URL must not embed a username or password".into());
    }
    let mut base = url.clone();
    base.set_query(None);
    base.set_fragment(None);
    Ok(base.to_string().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_credentials_and_non_http() {
        assert!(validate_server_url("http://127.0.0.1:8787").is_ok());
        assert!(validate_server_url("https://user:secret@example.com").is_err());
        assert!(validate_server_url("file:///etc/passwd").is_err());
        assert!(validate_server_url("ftp://example.com").is_err());
    }

    #[test]
    fn service_address_round_trips_without_credentials() {
        let path = std::env::temp_dir().join(format!("nexus-address-{}.toml", std::process::id()));
        save_config(&path, "https://nexus.example.com/").unwrap();
        assert_eq!(
            load_config(&path).unwrap().unwrap().server_url,
            "https://nexus.example.com"
        );
        assert!(save_config(&path, "https://user:secret@example.com").is_err());
        assert_eq!(
            load_config(&path).unwrap().unwrap().server_url,
            "https://nexus.example.com"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_file_means_central_catalog_is_off() {
        let path = std::env::temp_dir().join(format!(
            "nexus-missing-central-{}-{}.toml",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);
        assert!(load_config(&path).unwrap().is_none());
    }
}
