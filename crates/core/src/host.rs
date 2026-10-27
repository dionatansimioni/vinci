//! Running commands on the host, whether the app is a Flatpak or a native binary.

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecMode {
    /// AppImage or native build: run commands as-is.
    Direct,
    /// Inside the Flatpak sandbox: escape with `flatpak-spawn --host`.
    FlatpakSpawn,
}

impl ExecMode {
    pub fn detect() -> Self {
        Self::detect_in(Path::new("/"))
    }

    fn detect_in(root: &Path) -> Self {
        if root.join(".flatpak-info").exists() {
            Self::FlatpakSpawn
        } else {
            Self::Direct
        }
    }

    /// Wraps `argv` so it runs on the host.
    pub fn wrap(self, argv: &[String]) -> Vec<String> {
        match self {
            Self::Direct => argv.to_vec(),
            Self::FlatpakSpawn => ["flatpak-spawn", "--host"]
                .iter()
                .map(|s| s.to_string())
                .chain(argv.iter().cloned())
                .collect(),
        }
    }

    /// Wraps `argv` so it runs on the host as root, asking via polkit.
    pub fn wrap_root(self, argv: &[String]) -> Vec<String> {
        let with_pkexec: Vec<String> = std::iter::once("pkexec".to_string())
            .chain(argv.iter().cloned())
            .collect();
        self.wrap(&with_pkexec)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Executes commands. The real implementation shells out; tests use a fake.
pub trait Runner {
    fn run(&self, argv: &[String]) -> std::io::Result<Output>;
}

pub struct HostRunner {
    pub mode: ExecMode,
}

impl HostRunner {
    pub fn new() -> Self {
        Self {
            mode: ExecMode::detect(),
        }
    }
}

impl Default for HostRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl Runner for HostRunner {
    fn run(&self, argv: &[String]) -> std::io::Result<Output> {
        let argv = self.mode.wrap(argv);
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| std::io::Error::other("empty command"))?;
        let out = Command::new(program).args(args).output()?;
        Ok(Output {
            success: out.status.success(),
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Builds an owned argv from string literals.
pub fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::cell::RefCell;

    /// Records every command and answers from a list of (argv prefix, output) rules.
    #[derive(Default)]
    pub struct FakeRunner {
        pub calls: RefCell<Vec<Vec<String>>>,
        pub rules: Vec<(Vec<String>, Output)>,
    }

    impl FakeRunner {
        pub fn respond(mut self, prefix: &[&str], stdout: &str, success: bool) -> Self {
            self.rules.push((
                argv(prefix),
                Output {
                    success,
                    code: Some(if success { 0 } else { 1 }),
                    stdout: stdout.into(),
                    stderr: String::new(),
                },
            ));
            self
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, argv: &[String]) -> std::io::Result<Output> {
            self.calls.borrow_mut().push(argv.to_vec());
            self.rules
                .iter()
                .find(|(prefix, _)| argv.starts_with(prefix))
                .map(|(_, out)| Ok(out.clone()))
                .unwrap_or_else(|| Err(std::io::Error::from(std::io::ErrorKind::NotFound)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_mode_passes_through() {
        assert_eq!(
            ExecMode::Direct.wrap(&argv(&["podman", "ps"])),
            ["podman", "ps"]
        );
    }

    #[test]
    fn flatpak_mode_escapes_sandbox() {
        assert_eq!(
            ExecMode::FlatpakSpawn.wrap_root(&argv(&["apt-get", "install", "-y", "podman"])),
            [
                "flatpak-spawn",
                "--host",
                "pkexec",
                "apt-get",
                "install",
                "-y",
                "podman"
            ]
        );
    }

    #[test]
    fn detects_flatpak_by_info_file() {
        let root = std::env::temp_dir().join(format!("rli-host-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(ExecMode::detect_in(&root), ExecMode::Direct);
        std::fs::write(root.join(".flatpak-info"), "").unwrap();
        assert_eq!(ExecMode::detect_in(&root), ExecMode::FlatpakSpawn);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn host_runner_runs_real_commands() {
        let r = HostRunner {
            mode: ExecMode::Direct,
        };
        let out = r.run(&argv(&["echo", "ok"])).unwrap();
        assert!(out.success);
        assert_eq!(out.stdout.trim(), "ok");
        assert!(r.run(&[]).is_err());
    }
}
