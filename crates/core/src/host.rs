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

/// Variables the AppImage's GTK hook points at the files bundled inside it. Programs we
/// start on the host must not see them: GNOME Terminal, Nemo and other GTK apps abort
/// when GSETTINGS_SCHEMA_DIR hides their own settings schemas.
const APPIMAGE_VARS: [&str; 12] = [
    "GTK_DATA_PREFIX",
    "GTK_THEME",
    "GDK_BACKEND",
    "GSETTINGS_SCHEMA_DIR",
    "GI_TYPELIB_PATH",
    "GTK_EXE_PREFIX",
    "GTK_PATH",
    "GDK_PIXBUF_MODULE_FILE",
    "APPDIR",
    "APPIMAGE",
    "ARGV0",
    "OWD",
];

/// How the environment of a program started on the host differs from ours: each
/// variable to set (`Some`) or remove (`None`). Empty outside an AppImage.
pub fn host_env_changes() -> Vec<(String, Option<String>)> {
    env_changes(|k| std::env::var(k).ok())
}

fn env_changes(get: impl Fn(&str) -> Option<String>) -> Vec<(String, Option<String>)> {
    let Some(appdir) = get("APPDIR").filter(|d| !d.is_empty()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Option<String>)> = APPIMAGE_VARS
        .iter()
        .filter(|k| get(k).is_some())
        .map(|k| (k.to_string(), None))
        .collect();
    if let Some(dirs) = get("XDG_DATA_DIRS") {
        let kept: Vec<&str> = dirs
            .split(':')
            .filter(|d| !d.is_empty() && !d.starts_with(appdir.as_str()))
            .collect();
        out.push((
            "XDG_DATA_DIRS".into(),
            (!kept.is_empty()).then(|| kept.join(":")),
        ));
    }
    out
}

/// A `Command` for a program on the host, with the AppImage's own variables taken out.
pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    for (k, v) in host_env_changes() {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    cmd
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
    /// When set, streamed commands run in their own process group so a cancel can stop
    /// them with everything they started.
    pub cancel: Option<crate::cancel::Cancel>,
}

impl HostRunner {
    pub fn new() -> Self {
        Self {
            mode: ExecMode::detect(),
            cancel: None,
        }
    }

    /// A runner whose streamed commands stop when `cancel` fires.
    pub fn cancellable(cancel: crate::cancel::Cancel) -> Self {
        Self {
            cancel: Some(cancel),
            ..Self::new()
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
        // Never interrupted: killing a package manager halfway can break the system.
        exec_streaming(&self.mode.wrap_root(argv), on_line, None)
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
        crate::log::command(&argv);
        command(program)
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
        exec_streaming(&self.mode.wrap(argv), on_line, self.cancel.as_ref())
    }
}

/// Runs an already-wrapped argv, forwarding each stdout/stderr line as it arrives while
/// still collecting both for the result.
fn exec_streaming(
    argv: &[String],
    on_line: &mut dyn FnMut(&str),
    cancel: Option<&crate::cancel::Cancel>,
) -> std::io::Result<Output> {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    use std::sync::mpsc;

    let (program, args) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::other("empty command"))?;
    let mut cmd = command(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cancel.is_some() {
        cmd.process_group(0);
    }
    crate::log::command(argv);
    let mut child = cmd.spawn()?;
    if let Some(c) = cancel {
        c.set_running(Some(child.id()));
    }
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
        crate::log::output(&line);
        on_line(&line);
        let buf = if is_err { &mut stderr } else { &mut stdout };
        buf.push_str(&line);
        buf.push('\n');
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait();
    if let Some(c) = cancel {
        c.set_running(None);
    }
    let status = status?;
    crate::log::finished(status.code());
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
    crate::log::command(argv);
    let out = command(program).args(args).output()?;
    crate::log::finished(out.status.code());
    if !out.status.success() {
        crate::log::output(&String::from_utf8_lossy(&out.stderr));
    }
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
mod env_tests {
    use super::*;

    #[test]
    fn programs_on_the_host_do_not_see_the_appimage_environment() {
        let env = |k: &str| -> Option<String> {
            match k {
                "APPDIR" => Some("/tmp/.mount_VinciX".into()),
                "GSETTINGS_SCHEMA_DIR" => {
                    Some("/tmp/.mount_VinciX//usr/share/glib-2.0/schemas".into())
                }
                "GTK_PATH" => Some("/tmp/.mount_VinciX//usr/lib/gtk-4.0".into()),
                "XDG_DATA_DIRS" => Some(
                    "/tmp/.mount_VinciX/usr/share:/usr/share:/usr/local/share:/usr/share".into(),
                ),
                _ => None,
            }
        };
        let changes = env_changes(env);
        let removed: Vec<&str> = changes
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(removed, ["GSETTINGS_SCHEMA_DIR", "GTK_PATH", "APPDIR"]);
        assert!(changes.contains(&(
            "XDG_DATA_DIRS".into(),
            Some("/usr/share:/usr/local/share:/usr/share".into())
        )));

        // Outside an AppImage nothing changes, even with the same variables set.
        assert!(env_changes(|k| (k != "APPDIR").then(|| "x".into())).is_empty());
    }
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
            cancel: None,
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
            cancel: None,
        };
        let out = r.run(&argv(&["echo", "ok"])).unwrap();
        assert!(out.success);
        assert_eq!(out.stdout.trim(), "ok");
        assert!(r.run(&[]).is_err());
    }
}
