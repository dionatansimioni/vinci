//! Developer tool: runs the real install plan step by step with live output.
//!
//! cargo run -p rli-core --example run_plan -- <distrobox> <nvidia|amd|intel> <installer> <work_dir>

use rli_core::container::{install_plan, Target};
use rli_core::gpu::Vendor;
use rli_core::installer_file::{parse_filename, Edition};
use rli_core::recipe::Recipe;
use std::path::Path;
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [distrobox, vendor, installer, work_dir] = args.as_slice() else {
        eprintln!("usage: run_plan <distrobox> <nvidia|amd|intel> <installer> <work_dir>");
        std::process::exit(2);
    };
    let vendor = match vendor.as_str() {
        "nvidia" => Vendor::Nvidia,
        "amd" => Vendor::Amd,
        "intel" => Vendor::Intel,
        other => panic!("unknown vendor {other}"),
    };
    let installer = Path::new(installer);
    let name = installer.file_name().unwrap().to_string_lossy();
    let edition = parse_filename(&name).map_or(Edition::Free, |(e, _, _)| e);
    let skip = std::env::var("SKIP_STEPS").unwrap_or_default();
    let recipe = Recipe::bundled();
    let target = Target {
        edition,
        vendor,
        distrobox,
        installer,
        work_dir: Path::new(work_dir),
    };
    for step in install_plan(&recipe, &target) {
        if skip.split(',').any(|s| s == format!("{:?}", step.kind)) {
            eprintln!("\n=== skip {:?}", step.kind);
            continue;
        }
        eprintln!("\n=== {:?}: {:?}", step.kind, step.argv);
        let start = std::time::Instant::now();
        let status = Command::new(&step.argv[0])
            .args(&step.argv[1..])
            .status()
            .unwrap();
        eprintln!("=== {:?} -> {status} in {:.0?}", step.kind, start.elapsed());
        if !status.success() {
            std::process::exit(1);
        }
    }
}
