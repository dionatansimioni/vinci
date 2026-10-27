//! Core logic of Vinci, kept free of GUI dependencies so it can be
//! unit-tested and reused by the background service.

pub mod blackmagic;
pub mod container;
pub mod diagnostics;
pub mod distro;
pub mod ffmpeg;
pub mod flow;
pub mod gpu;
pub mod hardware;
pub mod host;
pub mod installer;
pub mod installer_file;
pub mod integrate;
pub mod known_bad;
pub mod launcher;
pub mod recipe;
pub mod state;
pub mod tools;
pub mod version;
