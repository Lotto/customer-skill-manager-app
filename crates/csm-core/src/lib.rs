//! Core logic for the Customer Skill Manager (CSM).
//!
//! This crate is deliberately GUI-free so it can be unit-tested with a plain
//! `cargo test`, without building the Tauri/WebView layer. It owns:
//!
//! - [`config`]: the on-disk TOML configuration (license, backend, targets…).
//! - [`state`]: the record of what the app has installed on this machine.
//! - [`manifest`]: the shape of what the backend advertises.
//! - [`diff`]: computing what to install/remove from (manifest, state).
//! - [`hash`]: content hashing used to detect changes.
//! - [`paths`]: resolving well-known directories such as `~/.claude/skills`.

pub mod backoff;
pub mod cleanup;
pub mod config;
pub mod desktop;
pub mod diff;
pub mod error;
pub mod hash;
#[cfg(feature = "net")]
pub mod http;
pub mod manifest;
pub mod paths;
pub mod state;
pub mod sync;

pub use error::{CoreError, Result};
