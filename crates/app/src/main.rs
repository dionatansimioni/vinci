mod app;
mod i18n;
mod media;
mod messages;
mod preferences;

use relm4::RelmApp;

pub const APP_ID: &str = "io.github.dionatansimioni.Vinci";

fn main() {
    let settings =
        rli_core::settings::Settings::load(&rli_core::settings::Settings::default_path());
    i18n::init(settings.language.as_deref());
    RelmApp::new(APP_ID).run::<app::App>(());
}
