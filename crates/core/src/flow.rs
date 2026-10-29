//! End-to-end install: plan → execute → desktop integration → state. The GUI calls this
//! from a worker thread and forwards the events to the progress view.

use crate::cancel::Cancel;
use crate::container::{self, StepKind, Target};
use crate::gpu::Gpu;
use crate::host::argv;
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

impl Error {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Step(installer::Error::Cancelled { .. }))
    }
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
    /// Save the container being replaced as a restore point (SPEC §12.4).
    pub keep_restore_point: bool,
    /// Lets the user stop the install; what it created so far is then removed.
    pub cancel: Option<&'a Cancel>,
}

#[derive(Debug)]
pub struct Outcome {
    pub installation: Installation,
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
            edition: installation.edition,
            name: display_name(installation.edition),
            env: &env,
        },
    );
    integrate::install(
        runner,
        recipe,
        distrobox,
        &installation.container,
        installation.edition,
        data_home,
        &entry,
    )
    .map(drop)
}

fn exists(runner: &dyn Runner, cmd: &[&str]) -> bool {
    runner.run(&argv(cmd)).is_ok_and(|o| o.success)
}

/// What a cancelled install got through, for [`undo`].
struct Undo<'a> {
    container: &'a str,
    /// Steps that finished before the cancel.
    done: &'a [StepKind],
    image_existed: bool,
    /// The install living in the same container before this run, if any.
    previous: Option<&'a Installation>,
}

/// Removes what a cancelled install created: the half-built container and an image it
/// downloaded. When it had already replaced an earlier install of the same edition, that
/// one comes back from the restore point taken at the start of this run. Best effort,
/// except that a failed restore is reported.
fn undo(runner: &dyn Runner, recipe: &Recipe, req: &Request, u: &Undo) -> Result<(), Error> {
    if let Some(c) = req.cancel {
        c.begin_cleanup();
    }
    let name = u.container;
    if u.done.contains(&StepKind::RemoveExisting) {
        let _ = runner.run(&argv(&["podman", "rm", "--force", "--ignore", name]));
    }
    let lost = u.previous.is_some() && !exists(runner, &["podman", "container", "exists", name]);
    if lost && u.done.contains(&StepKind::Snapshot) {
        let steps = crate::backup::restore_plan(req.distrobox, name, req.gpu.vendor);
        let ctx = Context {
            recipe,
            distrobox: req.distrobox,
            container: name,
            cancel: None,
        };
        installer::execute(runner, &steps, &ctx, &mut |_| {})?;
        // The restore point now backs the active container again.
        let mut state = State::load(req.state_path).map_err(Error::State)?;
        state.backup = None;
        state.save(req.state_path).map_err(Error::State)?;
    }
    if !u.image_existed {
        // Without --force: an image still used by some container stays.
        let images = std::iter::once(&recipe.container.image).chain(&recipe.container.mirrors);
        for image in images {
            let _ = runner.run(&argv(&["podman", "image", "rm", "--ignore", image]));
        }
    }
    Ok(())
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
    let previous = State::load(req.state_path)
        .map_err(Error::State)?
        .installation(&name)
        .cloned();
    let snapshot = req.keep_restore_point && previous.is_some();
    let steps = container::install_plan(
        recipe,
        &Target {
            edition,
            vendor: req.gpu.vendor,
            distrobox: req.distrobox,
            installer: &req.installer.path,
            work_dir: &work,
            snapshot,
        },
    );
    let ctx = Context {
        recipe,
        distrobox: req.distrobox,
        container: &name,
        cancel: req.cancel,
    };
    let image = &recipe.container.image;
    let image_existed = exists(runner, &["podman", "image", "exists", image]);
    // Only a container present now is captured by this run's snapshot.
    let had_container = exists(runner, &["podman", "container", "exists", &name]);
    let mut done = Vec::new();
    let result = installer::execute(runner, &steps, &ctx, &mut |e| {
        if let Event::Finished { kind, .. } = &e {
            done.push(*kind);
        }
        on(e)
    });
    // The extracted .run is several GB; never leave it behind, success or not.
    let _ = std::fs::remove_dir_all(&work);
    if let Err(installer::Error::Cancelled { .. }) = &result {
        undo(
            runner,
            recipe,
            req,
            &Undo {
                container: &name,
                done: &done,
                image_existed,
                previous: previous.as_ref().filter(|_| had_container),
            },
        )?;
    }
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
    if snapshot {
        state.backup = previous;
    }
    // Another edition, in its own container, stays installed next to this one.
    state.set_installation(installation.clone());
    state.save(req.state_path).map_err(Error::State)?;
    Ok(Outcome { installation })
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

        fn run(&self, runner: &dyn Runner) -> Result<Outcome, Error> {
            self.run_with(runner, None)
        }

        fn run_with(&self, runner: &dyn Runner, cancel: Option<&Cancel>) -> Result<Outcome, Error> {
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
                keep_restore_point: true,
                cancel,
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

        let desktop = launcher::paths(&env.home.join(".local/share"), Edition::Free).desktop;
        let text = std::fs::read_to_string(desktop).unwrap();
        assert!(text.contains("Name=DaVinci Resolve\n"));
        assert!(text.contains("--name vinci-free"));

        let state = State::load(&env.home.join("state.json")).unwrap();
        assert_eq!(state.installations[0].version.to_string(), "21.1.2");
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
    fn studio_after_free_keeps_both() {
        let free = Env::new("switch", Edition::Free);
        free.run(&ok_runner()).unwrap();
        let mut studio = Env::new("switch", Edition::Studio);
        studio.home = free.home.clone();
        let runner = ok_runner();
        let out = studio.run(&runner).unwrap();
        assert_eq!(out.installation.container, "vinci-studio");
        let st = State::load(&free.home.join("state.json")).unwrap();
        let names: Vec<&str> = st
            .installations
            .iter()
            .map(|i| i.container.as_str())
            .collect();
        assert_eq!(names, ["vinci-free", "vinci-studio"]);
        assert!(
            !runner
                .calls()
                .iter()
                .any(|c| c.contains(&"vinci-free".to_string())),
            "the Free container is not touched"
        );
        let data = free.home.join(".local/share");
        for e in [Edition::Free, Edition::Studio] {
            assert!(
                launcher::paths(&data, e).desktop.exists(),
                "{e:?} in the menu"
            );
        }
        assert!(
            std::fs::read_to_string(launcher::paths(&data, Edition::Studio).desktop)
                .unwrap()
                .contains("Name=DaVinci Resolve Studio\n")
        );
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
        let desktop = launcher::paths(&data, Edition::Free).desktop;
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
    fn update_keeps_a_restore_point_of_the_previous_version() {
        let env = Env::new("restore", Edition::Free);
        env.run(&ok_runner()).unwrap();
        let first = State::load(&env.home.join("state.json")).unwrap();
        assert!(first.backup.is_none(), "nothing to keep on a first install");

        let runner = ok_runner();
        env.run(&runner).unwrap();
        let st = State::load(&env.home.join("state.json")).unwrap();
        assert_eq!(st.backup.unwrap().container, "vinci-free");
        let calls = runner.calls();
        let snap = calls
            .iter()
            .position(|c| c.iter().any(|a| a.contains("container commit")))
            .unwrap();
        let rm = calls
            .iter()
            .position(|c| c.starts_with(&["podman".into(), "rm".into()]))
            .unwrap();
        assert!(
            snap < rm,
            "snapshot is taken before the old container is removed"
        );
    }

    #[test]
    fn reinstalling_same_edition_keeps_one_entry() {
        let env = Env::new("again", Edition::Free);
        env.run(&ok_runner()).unwrap();
        env.run(&ok_runner()).unwrap();
        let st = State::load(&env.home.join("state.json")).unwrap();
        assert_eq!(st.installations.len(), 1);
    }

    /// Cancels (like the user's button) when a command starting with `at` begins.
    struct CancelAt<'a> {
        inner: FakeRunner,
        at: Vec<String>,
        cancel: &'a Cancel,
    }

    impl Runner for CancelAt<'_> {
        fn run(&self, a: &[String]) -> std::io::Result<crate::host::Output> {
            if a.starts_with(&self.at) {
                self.cancel.cancel();
            }
            self.inner.run(a)
        }
    }

    /// Everything succeeds, except that the image is not there yet (unless `cached`) and
    /// the container exists only when `container` says so.
    fn world(cached: bool, container: bool) -> FakeRunner {
        FakeRunner::default()
            .respond(&["podman", "image", "exists"], "", cached)
            .respond(&["podman", "container", "exists"], "", container)
            .respond(&[], "", true)
    }

    fn position(calls: &[Vec<String>], prefix: &[&str]) -> Option<usize> {
        calls.iter().position(|c| c.starts_with(&argv(prefix)))
    }

    #[test]
    fn cancelling_a_first_install_removes_the_container_and_the_new_image() {
        let env = Env::new("cancel-new", Edition::Free);
        let cancel = Cancel::default();
        let image = Recipe::bundled().container.image;
        let r = CancelAt {
            inner: world(false, false),
            at: argv(&[
                "distrobox",
                "enter",
                "--name",
                "vinci-free",
                "--",
                "sudo",
                "sh",
            ]),
            cancel: &cancel,
        };
        let err = env.run_with(&r, Some(&cancel)).unwrap_err();
        assert!(err.is_cancelled());
        let calls = r.inner.calls();
        let packages = position(
            &calls,
            &[
                "distrobox",
                "enter",
                "--name",
                "vinci-free",
                "--",
                "sudo",
                "sh",
            ],
        )
        .unwrap();
        assert!(
            !calls[packages + 1..]
                .iter()
                .any(|c| c.iter().any(|a| a == "unzip")),
            "nothing runs after the cancel"
        );
        let rm = calls
            .iter()
            .rposition(|c| c == &argv(&["podman", "rm", "--force", "--ignore", "vinci-free"]));
        assert!(
            rm.unwrap() > packages,
            "the half-built container is removed"
        );
        assert!(calls.contains(&argv(&["podman", "image", "rm", "--ignore", &image])));
        assert!(!env.home.join("state.json").exists(), "nothing recorded");
        assert!(!work_dir(&env.home).exists());
        assert!(
            !launcher::paths(&env.home.join(".local/share"), Edition::Free)
                .desktop
                .exists()
        );
    }

    #[test]
    fn cancelling_a_reinstall_brings_back_the_previous_version() {
        let env = Env::new("cancel-again", Edition::Free);
        env.run(&ok_runner()).unwrap();
        let cancel = Cancel::default();
        // The container is there when the run starts and gone after the cancel.
        let inner = FakeRunner::default()
            .respond_once(&["podman", "container", "exists"], "", true)
            .respond(&["podman", "image", "exists"], "", true)
            .respond(&["podman", "container", "exists"], "", false)
            .respond(&[], "", true);
        let r = CancelAt {
            inner,
            at: argv(&[
                "distrobox",
                "enter",
                "--name",
                "vinci-free",
                "--",
                "sudo",
                "env",
            ]),
            cancel: &cancel,
        };
        assert!(env.run_with(&r, Some(&cancel)).unwrap_err().is_cancelled());
        let calls = r.inner.calls();
        let restore = calls
            .iter()
            .position(|c| c.ends_with(&argv(&["--image", crate::backup::IMAGE, "--nvidia"])))
            .expect("container recreated from the restore point");
        let rm = calls
            .iter()
            .rposition(|c| c == &argv(&["podman", "rm", "--force", "--ignore", "vinci-free"]))
            .unwrap();
        assert!(rm < restore);
        assert!(
            position(&calls, &["podman", "image", "rm"]).is_none(),
            "the image was already there"
        );
        let st = State::load(&env.home.join("state.json")).unwrap();
        assert_eq!(st.installations[0].container, "vinci-free");
        assert!(
            st.backup.is_none(),
            "the restore point is the active install again"
        );
    }

    #[test]
    fn cancelling_while_downloading_leaves_the_installed_version_alone() {
        let env = Env::new("cancel-pull", Edition::Free);
        env.run(&ok_runner()).unwrap();
        let cancel = Cancel::default();
        let r = CancelAt {
            inner: world(true, true),
            at: argv(&["podman", "pull"]),
            cancel: &cancel,
        };
        assert!(env.run_with(&r, Some(&cancel)).unwrap_err().is_cancelled());
        let calls = r.inner.calls();
        assert!(position(&calls, &["podman", "rm"]).is_none());
        assert!(position(&calls, &["distrobox"]).is_none());
        assert_eq!(
            State::load(&env.home.join("state.json"))
                .unwrap()
                .installations
                .len(),
            1
        );
    }

    /// No container at the start (e.g. removed by a system cleanup): the restore point
    /// holds something older than this run, so it is not brought back on a cancel.
    #[test]
    fn cancelling_without_a_container_restores_nothing() {
        let env = Env::new("cancel-gone", Edition::Free);
        env.run(&ok_runner()).unwrap();
        let cancel = Cancel::default();
        let r = CancelAt {
            inner: world(true, false),
            at: argv(&[
                "distrobox",
                "enter",
                "--name",
                "vinci-free",
                "--",
                "sudo",
                "env",
            ]),
            cancel: &cancel,
        };
        assert!(env.run_with(&r, Some(&cancel)).unwrap_err().is_cancelled());
        let calls = r.inner.calls();
        assert!(
            !calls.iter().any(|c| c.get(1).is_some_and(|a| a == "create")
                && c.contains(&crate::backup::IMAGE.to_string())),
            "not recreated from the restore point"
        );
    }
}
