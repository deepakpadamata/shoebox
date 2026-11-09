//! The Shoebox desktop app: [`AppCore`] (all the logic, no UI toolkit), the types shared with
//! the UI ([`types`]), blob serving ([`blob`]), the keystore ([`keystore`]) and two adapters:
//! a plain HTTP bridge (feature `http`, for browser-based UI review and end-to-end tests) and
//! the Tauri 2 shell (feature `tauri-app`).

mod backup;
pub mod blob;
mod burst;
pub mod commands;
mod core;
pub mod diagnostics;
mod export;
// ImageIO is on iPhone and iPad too.
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod heic_macos;
pub mod keystore;
pub mod notify;
mod people;
pub mod photos_source;
mod safety;
mod search_models;
mod shared;
pub mod types;
mod video;
mod web_links;

#[cfg(feature = "http")]
pub mod http;

#[cfg(all(feature = "tauri-app", any(target_os = "macos", target_os = "ios")))]
mod macos_photos;
#[cfg(all(feature = "tauri-app", any(target_os = "macos", target_os = "ios")))]
mod ocr_macos;
#[cfg(feature = "tauri-app")]
mod share;
#[cfg(feature = "tauri-app")]
pub mod shell;

#[cfg(all(feature = "tauri-app", target_os = "android"))]
pub mod android_bridge;
#[cfg(all(feature = "tauri-app", target_os = "android"))]
pub mod android_photos;
#[cfg(all(feature = "tauri-app", target_os = "android"))]
mod heic_android;

pub use crate::backup::pause_reason;
pub use crate::core::{AppCore, AppOptions};
