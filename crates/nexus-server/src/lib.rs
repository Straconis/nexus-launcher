//! Access broker for Nexus Launcher.
//!
//! The desktop never receives the GitHub OAuth client secret, the device code,
//! or the release credential. Private bytes are proxied only after a session
//! and a grant (or admin) check on that request.

mod admin_tools;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod github;
pub mod policy;
pub mod routes;

pub use config::Config;
pub use routes::{router, AppState};

#[cfg(test)]
mod tests;
