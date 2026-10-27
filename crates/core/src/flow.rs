//! End-to-end install: plan → execute → desktop integration → state. The GUI calls this
//! from a worker thread and forwards the events to the progress view.

use crate::container::{self, Target};
use crate::gpu::Gpu;
use crate::host::Runner;
use crate::installer::{self, Context, Event};
use crate::installer_file::{Edition, InstallerFile};
use crate::integrate;
use crate::launcher;
use crate::recipe::Recipe;
use crate::state::{Installation, State};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Step(#[from] installer::Error),
    #[error("could not set up the menu entry: {0}")]
    Integrate(std::io::Error),
    #[error("could not save Vinci's state: {0}")]
    State(std::io::Error),
}

pub struct Request<'a> {
    pub installer: &'a InstallerFile,
    pub gpus: &'a [Gpu],
    pub gpu: &'a Gpu,
    pub distrobox: &'a str,
    pub home: &'a Path,
    /// Usually [`integrate::data_home`]; explicit so callers and tests stay in control.
    pub data_home: &'a Path,
    pub state_path: &'a Path,
    /// Keep a copy of the installer so Resolve can be reinstalled offline (SPEC §12.5).
    pub keep_installer: bool,
}

#[derive(Debug)]
pub struct Outcome {
    pub installation: Installation,
    /// The install that was active before, if it lived in another container
    /// (Free ↔ Studio). The GUI offers to remove it.
    pub replaced: Option<Installation>,
}

pub fn work_dir(home: &Path) -> PathBuf {
    home.join(".cache/vinci/work")
}

pub fn installers_dir(home: &Path) -> PathBuf {
    home.join(".local/share/vinci/installers")
}

fn display_name(edition: Edition) -> &'static str {
    match edition {
        Edition::Free => "DaVinci Resolve",
        Edition::Studio => "DaVinci Resolve Studio",
    }
}

/// Keeps only `file` in `dir`: hard link when possible (same filesystem, no extra space),
/// copy otherwise. Older kept installers are deleted.
fn keep_installer(file: &Path, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let name = file.file_name().unwrap_or_default();
    let dest = dir.join(name);
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p != dest {
            std::fs::remove_file(p)?;
        }
    }
    if !dest.exists() && std::fs::hard_link(file, &dest).is_err() {
        std::fs::copy(file, &dest)?;
    }
    Ok(())
}

/// Writes the menu entry, icon and file types for an existing install. Used by the
/// install itself and by the "Repair" action when the entry was deleted.
pub fn write_launcher(
    runner: &dyn Runner,
    recipe: &Recipe,
    distrobox: &str,
    installation: &Installation,
    gpus: &[Gpu],
    gpu: &Gpu,
    data_home: &Path,
) -> std::io::Result<()> {
    let env = launcher::environment(recipe, gpus, gpu);
    let entry = launcher::desktop_file(
        recipe,
        &launcher::Entry {
            distrobox,
            container: &installation.container,
            name: display_name(installation.edition),
            env: &env,
        },
    );
    integrate::install(
        runner,
        recipe,
        distrobox,
        &installation.container,
        data_home,
        &entry,
    )
    .map(drop)
}

pub fn install_resolve(
    runner: &dyn Runner,
    recipe: &Recipe,
    req: &Request,
    on: &mut dyn FnMut(Event),
) -> Result<Outcome, Error> {
    let edition = req.installer.edition;
    let name = container::container_name(recipe, edition);
    let work = work_dir(req.home);
    let steps = container::install_plan(
        recipe,
        &Target {
            edition,
            vendor: req.gpu.vendor,
            distrobox: req.distrobox,
            installer: &req.installer.path,
            work_dir: &work,
        },
    );
    let ctx = Context {
        recipe,
        distrobox: req.distrobox,
        container: &name,
    };
    let result = installer::execute(runner, &steps, &ctx, on);
    // The extracted .run is several GB; never leave it behind, success or not.
    let _ = std::fs::remove_dir_all(&work);
    result?;

    let installation = Installation {
        container: name,
        edition,
        version: req.installer.version.clone(),
    };
    write_launcher(
        runner,
        recipe,
        req.distrobox,
        &installation,
        req.gpus,
        req.gpu,
        req.data_home,
    )
    .map_err(Error::Integrate)?;

    if req.keep_installer {
        // Best effort: a full disk here must not fail an install that already worked.
        let _ = keep_installer(&req.installer.path, &installers_dir(req.home));
    }

    let mut state = State::load(req.state_path).map_err(Error::State)?;
    let replaced = state
        .installation
        .replace(installation.clone())
        .filter(|old| old.container != installation.container);
    state.save(req.state_path).map_err(Error::State)?;
    Ok(Outcome {
        installation,
        replaced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Vendor;
    use crate::host::fake::FakeRunner;
    use crate::installer_file::FileKind;

    fn nvidia() -> Gpu {
        Gpu {
            vendor: Vendor::Nvidia,
            device_id: 0x2571,
            driver: Some("nvidia".into()),
            pci_slot: None,
            render_node: None,
            amd_gfx_target: None,
        }
    }

    struct Env {
        home: PathBuf,
        installer: InstallerFile,
    }

    impl Env {
        fn new(name: &str, edition: Edition) -> Self {
            let home = std::env::temp_dir().join(format!("rli-flow-{}-{name}", std::process::id()));
            let downloads = home.join("Downloads");
            std::fs::create_dir_all(&downloads).unwrap();
            let file = match edition {
                Edition::Free => "DaVinci_Resolve_21.1.2_Linux.zip",
                Edition::Studio => "DaVinci_Resolve_Studio_21.1.2_Linux.zip",
            };
            let path = downloads.join(file);
            std::fs::write(&path, b"zip").unwrap();
            // Simulate a leftover extraction from the run.
            std::fs::create_dir_all(work_dir(&home)).unwrap();
            std::fs::write(work_dir(&home).join("big.run"), b"x").unwrap();
            Self {
                installer: InstallerFile {
                    path,
                    edition,
                    version: "21.1.2".parse().unwrap(),
                    kind: FileKind::Zip,
                },
                home,
            }
        }

        fn run(&self, runner: &FakeRunner) -> Result<Outcome, Error> {
            let gpus = [nvidia()];
            let req = Request {
                installer: &self.installer,
                gpus: &gpus,
                gpu: &gpus[0],
                distrobox: "distrobox",
                home: &self.home,
                data_home: &self.home.join(".local/share"),
                state_path: &self.home.join("state.json"),
                keep_installer: true,
            };
            install_resolve(runner, &Recipe::bundled(), &req, &mut |_| {})
        }
    }

    impl Drop for Env {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    fn ok_runner() -> FakeRunner {
        FakeRunner::default().respond(&[], "", true)
    }

    #[test]
    fn full_install_writes_launcher_state_and_keeps_installer() {
        let env = Env::new("full", Edition::Free);
        let out = env.run(&ok_runner()).unwrap();
        assert_eq!(out.installation.container, "vinci-free");
        assert!(out.replaced.is_none());

        let desktop = launcher::paths(&env.home.join(".local/share")).desktop;
        let text = std::fs::read_to_string(desktop).unwrap();
        assert!(text.contains("Name=DaVinci Resolve\n"));
        assert!(text.contains("--name vinci-free"));

        let state = State::load(&env.home.join("state.json")).unwrap();
        assert_eq!(state.installation.unwrap().version.to_string(), "21.1.2");
        assert!(installers_dir(&env.home)
            .join("DaVinci_Resolve_21.1.2_Linux.zip")
            .exists());
        assert!(
            !work_dir(&env.home).exists(),
            "extracted installer cleaned up"
        );
    }

    #[test]
    fn failure_cleans_work_dir_and_leaves_state_alone() {
        let env = Env::new("fail", Edition::Free);
        let runner = FakeRunner::default().respond(&["podman"], "", false);
        assert!(matches!(env.run(&runner), Err(Error::Step(_))));
        assert!(!work_dir(&env.home).exists());
        assert!(!env.home.join("state.json").exists());
    }

    #[test]
    fn studio_after_free_reports_the_replaced_install() {
        let free = Env::new("switch", Edition::Free);
        free.run(&ok_runner()).unwrap();
        let mut studio = Env::new("switch", Edition::Studio);
        studio.home = free.home.clone();
        let out = studio.run(&ok_runner()).unwrap();
        assert_eq!(out.installation.container, "vinci-studio");
        assert_eq!(out.replaced.unwrap().container, "vinci-free");
        let kept: Vec<_> = std::fs::read_dir(installers_dir(&free.home))
            .unwrap()
            .collect();
        assert_eq!(kept.len(), 1, "only the latest installer is kept");
        std::mem::forget(studio); // same home as `free`, which cleans it up
    }

    #[test]
    fn repair_rewrites_a_deleted_launcher() {
        let env = Env::new("repair", Edition::Free);
        let out = env.run(&ok_runner()).unwrap();
        let data = env.home.join(".local/share");
        let desktop = launcher::paths(&data).desktop;
        std::fs::remove_file(&desktop).unwrap();
        let gpus = [nvidia()];
        write_launcher(
            &ok_runner(),
            &Recipe::bundled(),
            "distrobox",
            &out.installation,
            &gpus,
            &gpus[0],
            &data,
        )
        .unwrap();
        assert!(std::fs::read_to_string(desktop)
            .unwrap()
            .contains("--name vinci-free"));
    }

    #[test]
    fn reinstalling_same_edition_is_not_a_replacement() {
        let env = Env::new("again", Edition::Free);
        env.run(&ok_runner()).unwrap();
        assert!(env.run(&ok_runner()).unwrap().replaced.is_none());
    }
}
