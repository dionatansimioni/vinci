mod app;
mod i18n;
mod messages;

use relm4::RelmApp;

pub const APP_ID: &str = "io.github.dionatansimioni.Vinci";

fn main() {
    i18n::init(None);
    RelmApp::new(APP_ID).run::<app::App>(());
}
