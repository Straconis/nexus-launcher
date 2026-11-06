//! Nexus Launcher library: catalog parsing, GitHub release lookup, and per-user installs.
//! The graphical shell lives behind the `gui` feature.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod central;
pub mod checksum;
pub mod cli;
pub mod github;
pub mod install;
pub mod model;

#[cfg(feature = "gui")]
mod theme;
#[cfg(feature = "gui")]
mod ui;
