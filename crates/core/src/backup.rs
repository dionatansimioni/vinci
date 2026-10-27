//! Restore point (SPEC §12.4): before an update or reinstall replaces the container, it is
//! committed to an image; "Go back to the previous version" recreates the container from
//! it. Only the latest restore point is kept. Projects live in $HOME and are untouched.

use crate::container::{Step, StepKind};
use crate::gpu::Vendor;
use crate::host::{argv, Runner};
use crate::installer::{self, Context, Event};
use crate::recipe::Recipe;
use crate::state::{Installation, State};
use std::path::Path;

pub const IMAGE: &str = "localhost/vinci-backup:latest";

/// Commits `container` to [`IMAGE`] if it exists, then drops the previous restore point.
/// Succeeds (doing nothing) when there is no container yet.
pub fn snapshot_step(container: &str) -> Step {
    let script = r#"set -e
podman container exists "$1" || exit 0
old=$(podman image inspect --format '{{.Id}}' "$2" 2>/dev/null || true)
podman container commit "$1" "$2" >/dev/null
new=$(podman image inspect --format '{{.Id}}' "$2")
# The old image may still back a container; then it simply stays until the next cleanup.
if [ -n "$old" ] && [ "$old" != "$new" ]; then podman image rm "$old" >/dev/null 2>&1 || true; fi"#;
    Step {
        kind: StepKind::Snapshot,
        argv: argv(&["sh", "-c", script, "sh", container, IMAGE]),
    }
}

/// Steps that bring the restore point back as the active container.
pub fn restore_plan(distrobox: &str, container: &str, vendor: Vendor) -> Vec<Step> {
    let mut create = argv(&[
        distrobox, "create", "--yes", "--name", container, "--image", IMAGE,
    ]);
    if vendor == Vendor::Nvidia {
        create.push("--nvidia".into());
    }
    vec![
        Step {
            kind: StepKind::RemoveExisting,
            argv: argv(&["podman", "rm", "--force", "--ignore", container]),
        },
        Step {
            kind: StepKind::Create,
            argv: create,
        },
        Step {
            kind: StepKind::Initialize,
            argv: argv(&[
                distrobox, "enter", "--name", container, "--", "sudo", "-n", "true",
            ]),
        },
    ]
}

pub fn image_exists(runner: &dyn Runner) -> bool {
    runner
        .run(&argv(&["podman", "image", "exists", IMAGE]))
        .is_ok_and(|o| o.success)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("there is no restore point")]
    NoRestorePoint,
    #[error(transparent)]
    Step(#[from] installer::Error),
    #[error("could not save Vinci's state: {0}")]
    State(std::io::Error),
}

/// Recreates the container from the restore point and makes it the active install.
/// The caller rewrites the menu entry afterwards (`flow::write_launcher`).
pub fn restore(
    runner: &dyn Runner,
    recipe: &Recipe,
    distrobox: &str,
    vendor: Vendor,
    state_path: &Path,
    on: &mut dyn FnMut(Event),
) -> Result<Installation, Error> {
    let mut state = State::load(state_path).map_err(Error::State)?;
    let previous = state.backup.clone().ok_or(Error::NoRestorePoint)?;
    if !image_exists(runner) {
        return Err(Error::NoRestorePoint);
    }
    let steps = restore_plan(distrobox, &previous.container, vendor);
    let ctx = Context {
        recipe,
        distrobox,
        container: &previous.container,
    };
    installer::execute(runner, &steps, &ctx, on)?;
    state.installation = Some(previous.clone());
    // The image now backs the active container; it stops being a separate restore point.
    state.backup = None;
    state.save(state_path).map_err(Error::State)?;
    Ok(previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fake::FakeRunner;
    use crate::installer_file::Edition;

    fn inst(v: &str) -> Installation {
        Installation {
            container: "vinci-free".into(),
            edition: Edition::Free,
            version: v.parse().unwrap(),
        }
    }

    fn state_with_backup(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "rli-backup-{}-{name}/state.json",
            std::process::id()
        ));
        let st = State {
            installation: Some(inst("21.2")),
            backup: Some(inst("21.1.2")),
            ..State::default()
        };
        st.save(&p).unwrap();
        p
    }

    #[test]
    fn restore_recreates_container_from_backup_image() {
        let path = state_with_backup("ok");
        let r = FakeRunner::default().respond(&[], "", true);
        let back = restore(
            &r,
            &Recipe::bundled(),
            "distrobox",
            Vendor::Nvidia,
            &path,
            &mut |_| {},
        )
        .unwrap();
        let st = State::load(&path).unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert_eq!(back.version.to_string(), "21.1.2");
        assert_eq!(st.installation.unwrap().version.to_string(), "21.1.2");
        assert!(st.backup.is_none());
        let calls = r.calls();
        let create = calls
            .iter()
            .find(|c| c.get(1).map(String::as_str) == Some("create"))
            .unwrap();
        assert!(create.contains(&IMAGE.to_string()) && create.contains(&"--nvidia".to_string()));
    }

    #[test]
    fn no_backup_means_no_restore() {
        let path =
            std::env::temp_dir().join(format!("rli-backup-{}-none/state.json", std::process::id()));
        let r = FakeRunner::default().respond(&[], "", true);
        let e = restore(&r, &Recipe::bundled(), "d", Vendor::Amd, &path, &mut |_| {});
        assert!(matches!(e, Err(Error::NoRestorePoint)));
        assert!(r.calls().is_empty(), "nothing touched");

        let path = state_with_backup("noimage");
        let r = FakeRunner::default()
            .respond(&["podman", "image", "exists"], "", false)
            .respond(&[], "", true);
        let e = restore(&r, &Recipe::bundled(), "d", Vendor::Amd, &path, &mut |_| {});
        let st = State::load(&path).unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert!(matches!(e, Err(Error::NoRestorePoint)));
        assert_eq!(
            st.installation.unwrap().version.to_string(),
            "21.2",
            "active install untouched"
        );
    }

    #[test]
    fn snapshot_step_is_safe_without_a_container() {
        // Run the real script with a fake `podman` that says the container doesn't exist.
        let dir = std::env::temp_dir().join(format!("rli-snap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log");
        std::fs::write(
            dir.join("podman"),
            format!("#!/bin/sh\necho \"$@\" >> {}\n[ \"$1 $2\" = \"container exists\" ] && exit 1\nexit 0\n", log.display()),
        )
        .unwrap();
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(dir.join("podman"))
            .status()
            .unwrap();
        let step = snapshot_step("vinci-free");
        let path = format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let ok = std::process::Command::new(&step.argv[0])
            .args(&step.argv[1..])
            .env("PATH", path)
            .status()
            .unwrap()
            .success();
        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(ok);
        assert_eq!(
            calls.trim(),
            "container exists vinci-free",
            "no commit without a container"
        );
    }
}
