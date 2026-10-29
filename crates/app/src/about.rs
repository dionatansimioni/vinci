//! "About Vinci" dialog.

use crate::i18n::tr;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const WEBSITE: &str = env!("CARGO_PKG_HOMEPAGE");

/// Shows the dialog. `debug_info` is the diagnostic report, offered under
/// "Troubleshooting" so it can be copied or saved from here too.
pub fn present(parent: &impl IsA<gtk::Widget>, debug_info: String) {
    let dialog = adw::AboutDialog::builder()
        .application_name("Vinci")
        .application_icon(crate::APP_ID)
        .developer_name("Dionatan Simioni")
        .developers(["Dionatan Simioni"])
        .version(env!("CARGO_PKG_VERSION"))
        .comments(tr(
            "Installs and runs DaVinci Resolve on any Linux distribution, in an isolated container, and converts media Resolve can't open on Linux.",
        ))
        .website(WEBSITE)
        .issue_url(format!("{REPOSITORY}/issues"))
        .license_type(gtk::License::Gpl30)
        .copyright("© 2026 Dionatan Simioni")
        .debug_info(debug_info)
        .debug_info_filename("vinci-diagnostic-report.txt")
        .build();
    dialog.add_legal_section(
        &tr("Unofficial"),
        None,
        gtk::License::Custom,
        Some(&tr("Vinci is an independent project, not affiliated with Blackmagic Design. DaVinci Resolve is a trademark of Blackmagic Design Pty. Ltd. and is downloaded from its website under its own license.")),
    );
    // The AppImage bundles GTK, libadwaita and other LGPL libraries; their licenses and
    // sources are listed in THIRD-PARTY.md.
    dialog.add_legal_section(
        &tr("Third-party software"),
        None,
        gtk::License::Custom,
        Some(&tr("Vinci uses GTK, libadwaita and other free software. The licenses of every component, and where to get their source code, are listed at <a href=\"{url}\">THIRD-PARTY.md</a>.").replace("{url}", &format!("{REPOSITORY}/blob/main/THIRD-PARTY.md"))),
    );
    dialog.present(Some(parent));
}
