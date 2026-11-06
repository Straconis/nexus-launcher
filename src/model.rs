//! Shared package classification used by release selection and the installer.

use serde::{Deserialize, Serialize};

/// How Nexus Launcher treats a published file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    /// A single file we can chmod (when needed) and execute. Includes AppImage.
    Portable,
    /// A zip or tar.gz we extract under the per-user data directory.
    Archive,
    /// MSI, DMG, deb, or a setup executable. Downloaded, then handed to the OS.
    Handoff,
}
