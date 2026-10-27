//! Core logic of Resolve Linux Installer, kept free of GUI dependencies so it can be
//! unit-tested and reused by the background service.

pub mod distro;
pub mod gpu;
pub mod installer_file;
pub mod version;
