//! Small per-user appearance settings, separate from central credentials and app data.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub charcoal_frame: bool,
    pub open_maximized: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            charcoal_frame: true,
            open_maximized: true,
        }
    }
}

fn path() -> Result<PathBuf, String> {
    directories::ProjectDirs::from("org", "Straconis", "nexus-launcher")
        .map(|dirs| dirs.config_dir().join("preferences.toml"))
        .ok_or_else(|| "No preferences directory available".into())
}

pub fn load() -> Result<Preferences, String> {
    let path = path()?;
    if !path.exists() {
        return Ok(Preferences::default());
    }
    toml::from_str(&std::fs::read_to_string(path).map_err(|err| err.to_string())?)
        .map_err(|err| err.to_string())
}

fn save_at(path: &Path, prefs: &Preferences) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let temporary = path.with_extension("toml.tmp");
    std::fs::write(
        &temporary,
        toml::to_string(prefs).map_err(|err| err.to_string())?,
    )
    .map_err(|err| err.to_string())?;
    std::fs::rename(temporary, path).map_err(|err| err.to_string())
}

pub fn save(prefs: &Preferences) -> Result<(), String> {
    save_at(&path()?, prefs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appearance_preferences_survive_saving_and_allow_missing_fields() {
        let prefs = Preferences {
            charcoal_frame: false,
            open_maximized: false,
        };
        let path =
            std::env::temp_dir().join(format!("nexus-preferences-{}.toml", std::process::id()));
        save_at(&path, &prefs).unwrap();
        let restored: Preferences =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(restored, prefs);
        let partial: Preferences = toml::from_str("charcoal_frame = false").unwrap();
        assert!(!partial.charcoal_frame);
        assert!(partial.open_maximized);
        std::fs::remove_file(path).unwrap();
    }
}
