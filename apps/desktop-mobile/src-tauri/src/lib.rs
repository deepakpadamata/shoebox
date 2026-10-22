//! The Shoebox desktop app: [`AppCore`] (all the logic, no UI toolkit), the types shared with
//! the UI ([`types`]), blob serving ([`blob`]), the keystore ([`keystore`]) and two adapters:
//! a plain HTTP bridge (feature `http`, for browser-based UI review and end-to-end tests) and
//! the Tauri 2 shell (feature `tauri-app`).

pub mod blob;
pub mod commands;
mod core;
pub mod keystore;
pub mod types;

#[cfg(feature = "http")]
pub mod http;

pub use crate::core::{AppCore, AppOptions};
