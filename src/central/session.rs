//! Session token storage. The OS keyring is preferred. A mode-0600 file is the fallback.
//!
//! The token is an opaque broker session. It is not a GitHub token and it is not
//! a permanent download URL.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const KEYRING_SERVICE: &str = "nexus-launcher";
const KEYRING_USER: &str = "central-session";

pub fn load_session() -> Option<String> {
    if let Some(token) = keyring_get() {
        return Some(token);
    }
    default_file().and_then(|path| load_file(&path).ok().flatten())
}

pub fn save_session(token: &str) -> Result<(), String> {
    if keyring_set(token).is_ok() {
        if let Some(path) = default_file() {
            let _ = clear_file(&path);
        }
        return Ok(());
    }
    let path =
        default_file().ok_or_else(|| "no config directory for the session file".to_string())?;
    save_file(&path, token).map_err(|err| err.to_string())
}

pub fn clear_session() {
    let _ = keyring_delete();
    if let Some(path) = default_file() {
        let _ = clear_file(&path);
    }
}

fn default_file() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "Straconis", "nexus-launcher")
        .map(|dirs| dirs.config_dir().join("session.token"))
}

pub fn load_file(path: &Path) -> std::io::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    let token = text.trim();
    if token.is_empty() {
        Ok(None)
    } else {
        Ok(Some(token.to_string()))
    }
}

pub fn save_file(path: &Path, token: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(token.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

pub fn clear_file(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn keyring_get() -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
    entry.get_password().ok()
}

fn keyring_set(token: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    entry.set_password(token)
}

fn keyring_delete() -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    entry.delete_credential()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trip_does_not_use_the_token_as_a_path() {
        let dir = std::env::temp_dir().join(format!("nexus-session-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.token");
        let token = "session-token-not-a-secret-in-the-name";
        save_file(&path, token).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(load_file(&path).unwrap().as_deref(), Some(token));
        clear_file(&path).unwrap();
        assert!(load_file(&path).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
