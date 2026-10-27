//! Core logic of Vinci, kept free of GUI dependencies so it can be
//! unit-tested and reused by the background service.

pub mod container;
pub mod distro;
pub mod ffmpeg;
pub mod gpu;
pub mod hardware;
pub mod host;
pub mod installer_file;
pub mod known_bad;
pub mod recipe;
pub mod state;
pub mod version;
