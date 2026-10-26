//! `shoebox-keeper`: the always-on backstop device. This crate is the daemon, its local control
//! socket and the CLI that talks to it.

pub mod alerts;
pub mod control;
mod daemon;
pub mod render;
pub mod service;
pub mod status;

pub use daemon::{default_storage, Daemon, KeeperOptions};
