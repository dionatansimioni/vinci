mod about;
mod app;
mod desktop;
mod i18n;
mod media;
mod messages;
mod preferences;
mod service;

use relm4::RelmApp;

pub const APP_ID: &str = "io.github.dionatansimioni.Vinci";

/// Starts or stops the local activity log to match the setting (off by default).
pub fn apply_log_setting(settings: &rli_core::settings::Settings) {
    let path = rli_core::log::default_path();
    rli_core::log::enable(
        settings.save_logs.then_some(path.as_path()),
        &rli_core::state::home(),
    );
}

fn main() {
    let settings =
        rli_core::settings::Settings::load(&rli_core::settings::Settings::default_path());
    i18n::init(settings.language.as_deref());
    apply_log_setting(&settings);
    if std::env::args().any(|a| a == "--watch") {
        std::process::exit(service::run());
    }
    RelmApp::new(APP_ID).run::<app::App>(());
}
