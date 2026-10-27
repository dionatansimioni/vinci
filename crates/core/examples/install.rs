//! Developer tool: the full install exactly as the app runs it, with real commands.
//!
//! cargo run -p rli-core --example install -- <installer.zip|.run>

use rli_core::flow::{self, Request};
use rli_core::host::HostRunner;
use rli_core::installer_file::parse_filename;
use rli_core::installer_file::InstallerFile;
use rli_core::recipe::Recipe;
use rli_core::{gpu, integrate, state, tools};
use std::path::{Path, PathBuf};

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("usage: install <installer>"));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let (edition, version, kind) = parse_filename(&name).expect("not a Resolve installer");
    let installer = InstallerFile {
        path,
        edition,
        version,
        kind,
    };

    let recipe = Recipe::bundled();
    let runner = HostRunner::new();
    let home = state::home();
    let host = tools::detect(&runner, &recipe, &home);
    let distrobox = host
        .distrobox
        .executable()
        .expect("distrobox missing: run the host setup first");
    let gpus = gpu::detect(Path::new("/"));
    let chosen = gpu::preferred(&gpus).expect("no usable GPU").clone();
    println!(
        "podman {:?}, distrobox {distrobox}, GPU {}",
        host.podman,
        chosen.display_name()
    );

    let out = flow::install_resolve(
        &runner,
        &recipe,
        &Request {
            installer: &installer,
            gpus: &gpus,
            gpu: &chosen,
            distrobox: &distrobox,
            home: &home,
            data_home: &integrate::data_home(&home),
            state_path: &state::State::default_path(),
            keep_installer: true,
            keep_restore_point: true,
        },
        &mut |e| println!("{e:?}"),
    );
    match out {
        Ok(o) => println!("installed {:?}, replaced {:?}", o.installation, o.replaced),
        Err(e) => {
            eprintln!("FAILED: {e}");
            if let flow::Error::Step(rli_core::installer::Error::StepFailed { output, .. }) = &e {
                eprintln!(
                    "{}",
                    output
                        .stderr
                        .lines()
                        .rev()
                        .take(15)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
            std::process::exit(1);
        }
    }
}
