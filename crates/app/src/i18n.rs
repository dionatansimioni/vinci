//! gettext setup. English is the source language; translations live in `po/`.

use gettextrs::{bind_textdomain_codeset, bindtextdomain, setlocale, textdomain, LocaleCategory};
use std::path::PathBuf;

pub const DOMAIN: &str = "vinci";

/// Where compiled `.mo` files are, in order: explicit override, Flatpak, next to the binary.
fn locale_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("VINCI_LOCALEDIR") {
        return dir.into();
    }
    if std::path::Path::new("/.flatpak-info").exists() {
        return "/app/share/locale".into();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("share/locale")))
        .unwrap_or_else(|| "/usr/share/locale".into())
}

/// Initializes translations. `language` overrides the system locale (setting in the app).
pub fn init(language: Option<&str>) {
    // SAFETY: called once at startup, before GTK or any other thread exists.
    unsafe {
        if let Some(lang) = language {
            std::env::set_var("LANGUAGE", lang);
        }
        setlocale(LocaleCategory::LcAll, "");
    }
    let _ = bindtextdomain(DOMAIN, locale_dir());
    let _ = bind_textdomain_codeset(DOMAIN, "UTF-8");
    let _ = textdomain(DOMAIN);
}

pub fn tr(msgid: &str) -> String {
    gettextrs::gettext(msgid)
}
