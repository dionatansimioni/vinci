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

    /// Like `run`, but also hands every output line (stdout and stderr) to `on_line` as
    /// it appears, so long steps can show signs of life.
    fn run_streaming(
        &self,
        argv: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<Output> {
        let out = self.run(argv)?;
        out.stdout
            .lines()
            .chain(out.stderr.lines())
            .for_each(&mut *on_line);
        Ok(out)
    }
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

    /// Runs `argv` on the host as root, asking for the password through polkit.
    pub fn run_root(&self, argv: &[String]) -> std::io::Result<Output> {
        exec(&self.mode.wrap_root(argv))
    }

    /// [`Self::run_root`] with every output line handed to `on_line` as it appears.
    pub fn run_root_streaming(
        &self,
        argv: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<Output> {
        exec_streaming(&self.mode.wrap_root(argv), on_line)
    }

    /// Runs an argv that is already complete (no Flatpak wrapping) and returns stdout
    /// when it succeeds.
    pub fn run_plain(&self, argv: &[String]) -> std::io::Result<String> {
        let out = exec(&self.mode.wrap(argv))?;
        if out.success {
            Ok(out.stdout)
        } else {
            Err(std::io::Error::other(out.stderr))
        }
    }

    /// Starts a long-lived program (e.g. Resolve) without waiting for it.
    pub fn spawn_detached(&self, argv: &[String]) -> std::io::Result<()> {
        let argv = self.mode.wrap(argv);
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| std::io::Error::other("empty command"))?;
        Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(drop)
    }
}

impl Default for HostRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl Runner for HostRunner {
    fn run(&self, argv: &[String]) -> std::io::Result<Output> {
        exec(&self.mode.wrap(argv))
    }

    fn run_streaming(
        &self,
        argv: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<Output> {
        exec_streaming(&self.mode.wrap(argv), on_line)
    }
}

/// Runs an already-wrapped argv, forwarding each stdout/stderr line as it arrives while
/// still collecting both for the result.
fn exec_streaming(argv: &[String], on_line: &mut dyn FnMut(&str)) -> std::io::Result<Output> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::sync::mpsc;

    let (program, args) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::other("empty command"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (tx, rx) = mpsc::channel::<(bool, String)>();
    let readers: Vec<_> = [
        (
            false,
            Box::new(child.stdout.take().expect("piped")) as Box<dyn std::io::Read + Send>,
        ),
        (true, Box::new(child.stderr.take().expect("piped"))),
    ]
    .into_iter()
    .map(|(is_err, pipe)| {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                let _ = tx.send((is_err, line));
            }
        })
    })
    .collect();
    drop(tx);
    let (mut stdout, mut stderr) = (String::new(), String::new());
    for (is_err, line) in rx {
        on_line(&line);
        let buf = if is_err { &mut stderr } else { &mut stdout };
        buf.push_str(&line);
        buf.push('\n');
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait()?;
    Ok(Output {
        success: status.success(),
        code: status.code(),
        stdout,
        stderr,
    })
}

/// Runs an already-wrapped argv and captures its output.
fn exec(argv: &[String]) -> std::io::Result<Output> {
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

/// Builds an owned argv from string literals.
pub fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::cell::RefCell;

    /// Records every command and answers from (argv prefix, output) rules. The first
    /// matching rule wins; `once` rules are consumed when used.
    #[derive(Default)]
    pub struct FakeRunner {
        pub calls: RefCell<Vec<Vec<String>>>,
        rules: RefCell<Vec<(Vec<String>, Output, bool)>>,
    }

    impl FakeRunner {
        fn add(self, prefix: &[&str], stdout: &str, success: bool, once: bool) -> Self {
            let out = Output {
                success,
                code: Some(if success { 0 } else { 1 }),
                stdout: stdout.into(),
                stderr: if success {
                    String::new()
                } else {
                    "failed".into()
                },
            };
            self.rules.borrow_mut().push((argv(prefix), out, once));
            self
        }

        pub fn respond(self, prefix: &[&str], stdout: &str, success: bool) -> Self {
            self.add(prefix, stdout, success, false)
        }

        /// Answers only the next matching call, then falls through to later rules.
        pub fn respond_once(self, prefix: &[&str], stdout: &str, success: bool) -> Self {
            self.add(prefix, stdout, success, true)
        }

        pub fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, argv: &[String]) -> std::io::Result<Output> {
            self.calls.borrow_mut().push(argv.to_vec());
            let mut rules = self.rules.borrow_mut();
            let Some(i) = rules.iter().position(|(p, _, _)| argv.starts_with(p)) else {
                return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
            };
            let out = rules[i].1.clone();
            if rules[i].2 {
                rules.remove(i);
            }
            Ok(out)
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
    fn streams_lines_from_both_pipes() {
        let r = HostRunner {
            mode: ExecMode::Direct,
        };
        let mut seen = Vec::new();
        let out = r
            .run_streaming(
                &argv(&["sh", "-c", "echo out; echo err >&2; exit 4"]),
                &mut |l| seen.push(l.to_string()),
            )
            .unwrap();
        seen.sort();
        assert_eq!(seen, ["err", "out"]);
        assert!(!out.success);
        assert_eq!(out.code, Some(4));
        assert_eq!(
            (out.stdout.as_str(), out.stderr.as_str()),
            ("out\n", "err\n")
        );
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
