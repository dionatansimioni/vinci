//! Developer tool: exercises the real restore point on the installed container.
//!
//! cargo run -p rli-core --example restore_point -- snapshot|restore

use rli_core::backup;
use rli_core::host::{HostRunner, Runner};
use rli_core::recipe::Recipe;
use rli_core::state::{self, State};
use rli_core::{gpu, tools};
use std::path::Path;

fn main() {
    let cmd = std::env::args().nth(1).expect("snapshot|restore");
    let runner = HostRunner::new();
    let recipe = Recipe::bundled();
    let path = State::default_path();
    let mut st = State::load(&path).unwrap();
    let inst = st.installation.clone().expect("nothing installed");
    match cmd.as_str() {
        "snapshot" => {
            let step = backup::snapshot_step(&inst.container);
            let out = runner.run(&step.argv).unwrap();
            println!("snapshot ok={} {}", out.success, out.stderr.trim());
            if out.success {
                st.backup = Some(inst);
                st.save(&path).unwrap();
            }
        }
        "restore" => {
            let dbx = tools::detect(&runner, &recipe, &state::home())
                .distrobox
                .executable()
                .unwrap();
            let vendor = gpu::preferred(&gpu::detect(Path::new("/"))).unwrap().vendor;
            let r = backup::restore(&runner, &recipe, &dbx, vendor, &path, &mut |e| {
                println!("{e:?}")
            });
            println!("restore: {r:?}");
        }
        _ => panic!("snapshot|restore"),
    }
}
