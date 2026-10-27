//! Executes an install plan with progress events and recovery from known failures.

use crate::container::{Step, StepKind};
use crate::host::{argv, Output, Runner};
use crate::recipe::Recipe;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Started {
        index: usize,
        total: usize,
        kind: StepKind,
    },
    Finished {
        kind: StepKind,
        elapsed: Duration,
    },
    /// The main registry failed; trying this mirror.
    TryingMirror {
        image: String,
    },
    /// distrobox init failed; removing and recreating the container once.
    RecreatingContainer,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("step {kind:?} failed")]
    StepFailed { kind: StepKind, output: Box<Output> },
    #[error("could not run step {kind:?}: {source}")]
    Spawn {
        kind: StepKind,
        source: std::io::Error,
    },
}

pub struct Context<'a> {
    pub recipe: &'a Recipe,
    pub distrobox: &'a str,
    pub container: &'a str,
}

fn run(runner: &dyn Runner, kind: StepKind, argv: &[String]) -> Result<Output, Error> {
    runner
        .run(argv)
        .map_err(|source| Error::Spawn { kind, source })
}

fn ok(runner: &dyn Runner, kind: StepKind, argv: &[String]) -> Result<bool, Error> {
    Ok(run(runner, kind, argv)?.success)
}

/// Pulls from each mirror in turn; on success tags it with the main image name so the
/// rest of the plan doesn't need to know which registry served it.
fn pull_from_mirrors(
    runner: &dyn Runner,
    ctx: &Context,
    on: &mut dyn FnMut(Event),
) -> Result<bool, Error> {
    for mirror in &ctx.recipe.container.mirrors {
        on(Event::TryingMirror {
            image: mirror.clone(),
        });
        let k = StepKind::PullImage;
        if ok(runner, k, &argv(&["podman", "pull", mirror]))?
            && ok(
                runner,
                k,
                &argv(&["podman", "tag", mirror, &ctx.recipe.container.image]),
            )?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn recreate_and_init(
    runner: &dyn Runner,
    ctx: &Context,
    steps: &[Step],
    init: &Step,
) -> Result<bool, Error> {
    let k = StepKind::Initialize;
    let _ = run(
        runner,
        k,
        &argv(&[ctx.distrobox, "rm", "--force", ctx.container]),
    )?;
    let Some(create) = steps.iter().find(|s| s.kind == StepKind::Create) else {
        return Ok(false);
    };
    Ok(ok(runner, StepKind::Create, &create.argv)? && ok(runner, k, &init.argv)?)
}

/// Runs every step in order, stopping at the first unrecoverable failure.
pub fn execute(
    runner: &dyn Runner,
    steps: &[Step],
    ctx: &Context,
    on: &mut dyn FnMut(Event),
) -> Result<(), Error> {
    let total = steps.len();
    for (index, step) in steps.iter().enumerate() {
        on(Event::Started {
            index,
            total,
            kind: step.kind,
        });
        let start = Instant::now();
        let out = run(runner, step.kind, &step.argv)?;
        let recovered = !out.success
            && match step.kind {
                StepKind::PullImage => pull_from_mirrors(runner, ctx, on)?,
                StepKind::Initialize => {
                    on(Event::RecreatingContainer);
                    recreate_and_init(runner, ctx, steps, step)?
                }
                _ => false,
            };
        if !out.success && !recovered {
            return Err(Error::StepFailed {
                kind: step.kind,
                output: Box::new(out),
            });
        }
        on(Event::Finished {
            kind: step.kind,
            elapsed: start.elapsed(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{install_plan, Target};
    use crate::gpu::Vendor;
    use crate::host::fake::FakeRunner;
    use crate::installer_file::Edition;
    use std::path::Path;

    fn plan(recipe: &Recipe) -> Vec<Step> {
        install_plan(
            recipe,
            &Target {
                edition: Edition::Free,
                vendor: Vendor::Nvidia,
                distrobox: "distrobox",
                installer: Path::new("/h/DaVinci_Resolve_21.1.2_Linux.zip"),
                work_dir: Path::new("/h/work"),
                snapshot: false,
            },
        )
    }

    fn ctx(recipe: &Recipe) -> Context<'_> {
        Context {
            recipe,
            distrobox: "distrobox",
            container: "vinci-free",
        }
    }

    fn collect(runner: &FakeRunner, recipe: &Recipe) -> (Result<(), Error>, Vec<Event>) {
        let mut events = Vec::new();
        let r = execute(runner, &plan(recipe), &ctx(recipe), &mut |e| events.push(e));
        (r, events)
    }

    #[test]
    fn runs_every_step_in_order() {
        let recipe = Recipe::bundled();
        let runner = FakeRunner::default()
            .respond(&["podman"], "", true)
            .respond(&["distrobox"], "", true);
        let (r, events) = collect(&runner, &recipe);
        r.unwrap();
        let started: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Started { index, total, .. } => Some((*index, *total)),
                _ => None,
            })
            .collect();
        assert_eq!(started.len(), plan(&recipe).len());
        assert_eq!(started[0], (0, started.len()));
        assert_eq!(runner.calls().len(), plan(&recipe).len());
    }

    #[test]
    fn stops_at_first_failure_and_reports_it() {
        let recipe = Recipe::bundled();
        let runner = FakeRunner::default()
            .respond(&["podman"], "", true)
            .respond(
                &[
                    "distrobox",
                    "enter",
                    "--name",
                    "vinci-free",
                    "--",
                    "sudo",
                    "sh",
                ],
                "",
                false,
            )
            .respond(&["distrobox"], "", true);
        let (r, _) = collect(&runner, &recipe);
        let Err(Error::StepFailed { kind, output }) = r else {
            panic!("expected failure")
        };
        assert_eq!(kind, StepKind::InstallPackages);
        assert_eq!(output.stderr, "failed");
        assert!(
            !runner
                .calls()
                .iter()
                .any(|c| c.get(5).map(String::as_str) == Some("unzip")),
            "nothing runs after the failed step"
        );
    }

    #[test]
    fn falls_back_to_mirror_and_tags_it() {
        let recipe = Recipe::bundled();
        let main = recipe.container.image.clone();
        let mirror = recipe.container.mirrors[0].clone();
        let runner = FakeRunner::default()
            .respond(&["podman", "pull", &main], "", false)
            .respond(&["podman"], "", true)
            .respond(&["distrobox"], "", true);
        let (r, events) = collect(&runner, &recipe);
        r.unwrap();
        assert!(events.contains(&Event::TryingMirror {
            image: mirror.clone()
        }));
        assert!(runner
            .calls()
            .contains(&argv(&["podman", "tag", &mirror, &main])));
    }

    #[test]
    fn fails_when_every_registry_fails() {
        let recipe = Recipe::bundled();
        let runner = FakeRunner::default()
            .respond(&["podman", "pull"], "", false)
            .respond(&["distrobox"], "", true);
        let (r, _) = collect(&runner, &recipe);
        assert!(matches!(
            r,
            Err(Error::StepFailed {
                kind: StepKind::PullImage,
                ..
            })
        ));
    }

    #[test]
    fn recreates_container_once_when_init_fails() {
        let recipe = Recipe::bundled();
        let init = [
            "distrobox",
            "enter",
            "--name",
            "vinci-free",
            "--",
            "sudo",
            "-n",
            "true",
        ];
        let runner = FakeRunner::default()
            .respond_once(&init, "", false)
            .respond(&["podman"], "", true)
            .respond(&["distrobox"], "", true);
        let (r, events) = collect(&runner, &recipe);
        r.unwrap();
        assert!(events.contains(&Event::RecreatingContainer));
        let calls = runner.calls();
        let rm = calls
            .iter()
            .position(|c| c == &argv(&["distrobox", "rm", "--force", "vinci-free"]))
            .expect("container removed");
        assert_eq!(calls[rm + 1][1], "create", "then created again");
    }

    #[test]
    fn gives_up_when_init_fails_twice() {
        let recipe = Recipe::bundled();
        let init = [
            "distrobox",
            "enter",
            "--name",
            "vinci-free",
            "--",
            "sudo",
            "-n",
            "true",
        ];
        let runner = FakeRunner::default()
            .respond(&init, "", false)
            .respond(&["podman"], "", true)
            .respond(&["distrobox"], "", true);
        let (r, _) = collect(&runner, &recipe);
        assert!(matches!(
            r,
            Err(Error::StepFailed {
                kind: StepKind::Initialize,
                ..
            })
        ));
    }

    #[test]
    fn missing_executable_is_a_spawn_error() {
        let recipe = Recipe::bundled();
        let (r, _) = collect(&FakeRunner::default(), &recipe);
        assert!(matches!(
            r,
            Err(Error::Spawn {
                kind: StepKind::PullImage,
                ..
            })
        ));
    }
}
