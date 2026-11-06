//! Desktop client for the central catalog broker.
//!
//! Network and credential storage live here, not in the egui frame. The module
//! does not decide who may download; the server does. A revoked grant does not
//! delete a build that is already on disk.

mod client;
mod config;
mod session;

pub use client::CentralClient;
pub use config::{config_path, load_config, save_config, validate_server_url, ServerConfig};
pub use session::{clear_session, load_session, save_session};
