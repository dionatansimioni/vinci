//! Weekly "did the world change?" check, run by CI. Verifies that everything the recipe
//! depends on still exists. Exits non-zero (and prints what broke) if anything failed.
//!
//! cargo run -p rli-core --example watchdog [-- path/to/recipe.toml]
//!
//! Needs curl and podman (or docker via CONTAINER_ENGINE=docker).

use rli_core::blackmagic::parse_latest;
use rli_core::recipe::Recipe;
use std::process::Command;

struct Report {
    failures: Vec<String>,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: &str) {
        println!(
            "{} {name}{}",
            if ok { "✓" } else { "✗" },
            if detail.is_empty() {
                String::new()
            } else {
                format!(" — {detail}")
            }
        );
        if !ok {
            self.failures.push(format!("{name}: {detail}"));
        }
    }
}

fn curl(args: &[&str]) -> Option<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--retry",
            "3",
            "--max-time",
            "60",
            "-A",
            "Mozilla/5.0 (vinci-watchdog)",
        ])
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn main() {
    let recipe = match std::env::args().nth(1) {
        Some(path) => Recipe::parse(&std::fs::read_to_string(path).expect("read recipe"))
            .expect("valid recipe"),
        None => Recipe::bundled(),
    };
    let engine = std::env::var("CONTAINER_ENGINE").unwrap_or_else(|_| "podman".into());
    let mut r = Report {
        failures: Vec::new(),
    };

    for (name, url) in [
        ("Blackmagic API (Free)", &recipe.blackmagic.latest_free),
        ("Blackmagic API (Studio)", &recipe.blackmagic.latest_studio),
    ] {
        let latest = curl(&[url]).and_then(|j| parse_latest(&j));
        r.check(
            name,
            latest.is_some(),
            &latest.map_or_else(
                || format!("unexpected response from {url}"),
                |v| format!("latest {v}"),
            ),
        );
    }
    r.check(
        "Blackmagic download page",
        curl(&["-o", "/dev/null", &recipe.blackmagic.download_page]).is_some(),
        "",
    );
    r.check(
        "distrobox install script",
        curl(&["-o", "/dev/null", &recipe.distrobox.install_url]).is_some(),
        &recipe.distrobox.install_url,
    );

    let vendors = [
        ("nvidia", &recipe.gpu.nvidia),
        ("amd", &recipe.gpu.amd),
        ("intel", &recipe.gpu.intel),
    ];
    for (name, v) in &vendors {
        if let Some(repo) = &v.apt_repo {
            r.check(
                &format!("{name} repo key"),
                curl(&["-o", "/dev/null", &repo.key_url]).is_some(),
                &repo.key_url,
            );
            let release = format!("{}/dists/{}/Release", repo.url, repo.suite);
            r.check(
                &format!("{name} repo index"),
                curl(&["-o", "/dev/null", &release]).is_some(),
                &release,
            );
        }
    }

    // Simulate the full package install on the real base image, with vendor repos added.
    let mut script = String::from("set -e; export DEBIAN_FRONTEND=noninteractive; apt-get update -qq --error-on=any; apt-get install -y -qq --no-install-recommends ca-certificates curl >/dev/null; install -d /etc/apt/keyrings; ");
    let mut packages: Vec<&str> = recipe.packages.base.iter().map(String::as_str).collect();
    for (_, v) in &vendors {
        if let Some(repo) = &v.apt_repo {
            // Recipe fields are validated (https URLs, plain tokens), safe to inline here.
            script.push_str(&format!(
                "curl -fsSL {} -o {}; echo '{}' > /etc/apt/sources.list.d/{}.list; ",
                repo.key_url,
                repo.keyring_path(),
                repo.source_line(),
                repo.name
            ));
        }
        packages.extend(v.packages.iter().map(String::as_str));
    }
    script.push_str(&format!("apt-get update -qq --error-on=any; apt-get install -s --no-install-recommends {} >/dev/null && echo SIMULATION_OK", packages.join(" ")));
    let out = Command::new(&engine)
        // Host networking, like distrobox, so results match what users get.
        .args([
            "run",
            "--rm",
            "--network",
            "host",
            &recipe.container.image,
            "sh",
            "-c",
            &script,
        ])
        .output();
    let (ok, detail) = match out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            let last = stderr
                .lines()
                .filter(|l| l.starts_with("E:"))
                .collect::<Vec<_>>()
                .join("; ");
            (
                stdout.contains("SIMULATION_OK"),
                if last.is_empty() {
                    format!("{} packages", packages.len())
                } else {
                    last
                },
            )
        }
        Err(e) => (false, format!("{engine}: {e}")),
    };
    r.check(
        &format!("packages on {}", recipe.container.image),
        ok,
        &detail,
    );

    for mirror in &recipe.container.mirrors {
        let ok = Command::new(&engine)
            .args(["pull", "-q", mirror])
            .output()
            .is_ok_and(|o| o.status.success());
        r.check(&format!("mirror {mirror}"), ok, "");
    }

    if r.failures.is_empty() {
        println!("\nAll checks passed.");
    } else {
        println!("\n{} check(s) failed:", r.failures.len());
        for f in &r.failures {
            println!("- {f}");
        }
        std::process::exit(1);
    }
}
