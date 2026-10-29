//! Developer tool: stages the udev rules from the installed container and reports which
//! pass the allow-list. Does not install anything on the host.

use rli_core::host::HostRunner;
use rli_core::recipe::Recipe;
use rli_core::state::{self, State};
use rli_core::{tools, udev};

fn main() {
    let runner = HostRunner::new();
    let recipe = Recipe::bundled();
    let home = state::home();
    let inst = State::load(&State::default_path())
        .unwrap()
        .installations
        .into_iter()
        .next()
        .expect("nothing installed");
    let dbx = tools::detect(&runner, &recipe, &home)
        .distrobox
        .executable()
        .unwrap();
    let staging = home.join(".cache/vinci/udev");
    let (ok, rejected) = udev::stage(&runner, &recipe, &dbx, &inst.container, &staging).unwrap();
    println!(
        "accepted: {ok:?}\nrejected: {rejected:?}\nwould install: {:?}",
        udev::host_paths(&ok)
    );
}
