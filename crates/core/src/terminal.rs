//! Opening a shell inside the Resolve container in the user's terminal emulator, for
//! debugging and installing extra plugins by hand.

use crate::host::{argv, Runner};
use std::path::Path;

/// Terminals we know how to start with a command, and the flag that precedes it.
/// An empty flag means the command follows the program name directly. Every one of them
/// takes the command as separate arguments (no single quoted string).
const KNOWN: &[(&str, &[&str])] = &[
    // The freedesktop way to run the user's chosen terminal (Ubuntu 25.10, Fedora 43).
    ("xdg-terminal-exec", &[]),
    ("ptyxis", &["--"]),
    ("kgx", &["--"]),
    ("gnome-terminal", &["--"]),
    // --separate: otherwise a running Konsole takes over the window (often as a tab of
    // the window Vinci was started from) and this process exits at once.
    ("konsole", &["--separate", "-e"]),
    ("xfce4-terminal", &["-x"]),
    ("mate-terminal", &["-x"]),
    ("cosmic-term", &["-e"]),
    ("io.elementary.terminal", &["-x"]),
    ("deepin-terminal", &["-e"]),
    ("qterminal", &["-e"]),
    ("lxterminal", &["-e"]),
    ("terminator", &["-x"]),
    ("tilix", &["-e"]),
    ("ghostty", &["-e"]),
    ("alacritty", &["-e"]),
    ("kitty", &[]),
    ("foot", &[]),
    ("wezterm", &["start", "--"]),
    ("x-terminal-emulator", &["-e"]),
    ("xterm", &["-e"]),
];

/// Candidates in the order to try them: the desktop's own terminal first, so KDE gets
/// Konsole even when GNOME Terminal is also installed.
pub fn candidates(desktop: &str) -> Vec<&'static str> {
    let desktop = desktop.to_ascii_lowercase();
    let preferred: &[&str] = if desktop.contains("kde") {
        &["konsole"]
    } else if desktop.contains("gnome") {
        &["ptyxis", "kgx", "gnome-terminal"]
    } else if desktop.contains("cinnamon") {
        &["gnome-terminal"]
    } else if desktop.contains("xfce") {
        &["xfce4-terminal"]
    } else if desktop.contains("mate") {
        &["mate-terminal"]
    } else if desktop.contains("cosmic") {
        &["cosmic-term"]
    } else if desktop.contains("pantheon") {
        &["io.elementary.terminal"]
    } else if desktop.contains("deepin") {
        &["deepin-terminal"]
    } else if desktop.contains("lxqt") {
        &["qterminal"]
    } else if desktop.contains("lxde") {
        &["lxterminal"]
    } else {
        &[]
    };
    let mut out: Vec<&str> = preferred.to_vec();
    out.extend(
        KNOWN
            .iter()
            .map(|(t, _)| *t)
            .filter(|t| !preferred.contains(t)),
    );
    out
}

/// The argv that opens `terminal` running `command`, or `None` for an unknown terminal.
pub fn command(terminal: &str, inner: &[String]) -> Option<Vec<String>> {
    let (_, flag) = KNOWN.iter().find(|(t, _)| *t == terminal)?;
    let mut a = vec![terminal.to_string()];
    a.extend(flag.iter().map(|s| s.to_string()));
    a.extend(inner.iter().cloned());
    Some(a)
}

/// The command a user types to get the same shell by hand. A distrobox on the system
/// path is just `distrobox`; Vinci's private copy needs its full path.
pub fn manual_command(distrobox: &str, container: &str) -> String {
    let on_path = ["/usr/bin/", "/usr/local/bin/", "/bin/"]
        .iter()
        .any(|dir| distrobox.strip_prefix(dir) == Some("distrobox"));
    let program = if on_path || !distrobox.contains('/') {
        "distrobox".to_string()
    } else if distrobox.contains(char::is_whitespace) {
        format!("'{}'", distrobox.replace('\'', r"'\''"))
    } else {
        distrobox.to_string()
    };
    format!("{program} enter --name {container}")
}

/// An interactive shell inside `container`. If it fails or ends within a few seconds,
/// the window stays open with `on_error` and the exit code until Enter is pressed,
/// instead of flashing and closing. `report` records whether the shell started and how
/// it ended (see [`read_report`]): the terminal's own exit says nothing about it.
pub fn shell_in(distrobox: &str, container: &str, on_error: &str, report: &Path) -> Vec<String> {
    argv(&[
        "sh",
        "-c",
        r#"msg=$1; report=$2; shift 2
echo started > "$report"
t=$(date +%s)
"$@"; st=$?
secs=$(( $(date +%s) - t ))
echo "ended $st $secs" > "$report"
if [ "$st" -ne 0 ] || [ "$secs" -lt 5 ]; then printf '\n%s (%s)\n' "$msg" "$st"; read -r _; fi"#,
        "sh",
        on_error,
        &report.to_string_lossy(),
        distrobox,
        "enter",
        "--name",
        container,
    ])
}

/// What the shell started by [`shell_in`] did, as far as its report file tells.
#[derive(Debug, PartialEq, Eq)]
pub enum ShellRun {
    /// The terminal never ran the command (or it couldn't write the report).
    NeverStarted,
    /// Still running: in a window the terminal handed to another process, say.
    Running,
    Ended {
        code: i32,
        secs: u64,
    },
}

pub fn read_report(path: &Path) -> ShellRun {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut words = text.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some("started"), ..) => ShellRun::Running,
        (Some("ended"), Some(code), Some(secs)) => ShellRun::Ended {
            code: code.parse().unwrap_or(-1),
            secs: secs.parse().unwrap_or(0),
        },
        _ => ShellRun::NeverStarted,
    }
}

/// The first installed terminal (checked on the host in one call), ready to run `inner`.
pub fn find(runner: &dyn Runner, desktop: &str, inner: &[String]) -> Option<Vec<String>> {
    let list = candidates(desktop);
    let mut probe = argv(&[
        "sh",
        "-c",
        r#"for t in "$@"; do command -v "$t" >/dev/null 2>&1 && echo "$t"; done; true"#,
        "sh",
    ]);
    probe.extend(list.iter().map(|s| s.to_string()));
    let out = runner.run(&probe).ok()?;
    let found = out.stdout.lines().next().map(|l| l.trim().to_string());
    crate::log::info(&format!(
        "terminal: {}",
        found
            .as_deref()
            .unwrap_or("none of the known ones is installed")
    ));
    command(&found?, inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fake::FakeRunner;

    #[test]
    fn prefers_the_desktops_own_terminal() {
        assert_eq!(candidates("KDE")[0], "konsole");
        assert_eq!(candidates("ubuntu:GNOME")[0], "ptyxis");
        assert_eq!(candidates("")[0], "xdg-terminal-exec");
        assert_eq!(candidates("X-Cinnamon")[0], "gnome-terminal");
        assert_eq!(candidates("COSMIC")[0], "cosmic-term");
        let all = candidates("KDE");
        assert_eq!(all.len(), KNOWN.len(), "no duplicates, nothing lost");
    }

    #[test]
    fn builds_the_command_per_terminal() {
        let inner = shell_in(
            "/usr/bin/distrobox",
            "vinci-free",
            "Failed",
            Path::new("/r"),
        );
        let konsole = command("konsole", &inner).unwrap();
        assert_eq!(konsole[..4], ["konsole", "--separate", "-e", "sh"]);
        assert_eq!(
            konsole[konsole.len() - 4..],
            ["/usr/bin/distrobox", "enter", "--name", "vinci-free"]
        );
        assert_eq!(
            command("gnome-terminal", &inner).unwrap()[..2],
            ["gnome-terminal", "--"]
        );
        assert_eq!(command("kitty", &inner).unwrap()[1], "sh");
        assert_eq!(command("xdg-terminal-exec", &inner).unwrap()[1], "sh");
        assert!(command("rm", &inner).is_none());
        assert_eq!(
            manual_command("/usr/bin/distrobox", "vinci-free"),
            "distrobox enter --name vinci-free"
        );
        assert_eq!(
            manual_command(
                "/home/u/.local/share/vinci/distrobox/bin/distrobox",
                "vinci-studio"
            ),
            "/home/u/.local/share/vinci/distrobox/bin/distrobox enter --name vinci-studio"
        );
        assert_eq!(
            manual_command("/home/my user/bin/distrobox", "c"),
            "'/home/my user/bin/distrobox' enter --name c"
        );
    }

    /// The wrapper runs the shell, records how it went, and waits with the message when
    /// it failed or ended at once (a window that only flashes tells nobody anything).
    #[test]
    fn a_shell_that_fails_or_ends_at_once_keeps_the_window_open() {
        let report = std::env::temp_dir().join(format!("rli-term-{}", std::process::id()));
        let run = |inner: &[String], input: &str| {
            use std::io::Write;
            let mut child = std::process::Command::new(&inner[0])
                .args(&inner[1..])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
        };
        assert_eq!(read_report(&report), ShellRun::NeverStarted);
        // `false` stands in for a distrobox that fails; `true` for one that ends at once.
        let fails = shell_in("false", "c", "It ended. Press Enter.", &report);
        assert_eq!(run(&fails, "\n"), "\nIt ended. Press Enter. (1)\n");
        assert_eq!(read_report(&report), ShellRun::Ended { code: 1, secs: 0 });
        let quick = shell_in("true", "c", "It ended. Press Enter.", &report);
        assert_eq!(run(&quick, "\n"), "\nIt ended. Press Enter. (0)\n");
        assert_eq!(read_report(&report), ShellRun::Ended { code: 0, secs: 0 });

        std::fs::write(&report, "started\n").unwrap();
        assert_eq!(read_report(&report), ShellRun::Running);
        let _ = std::fs::remove_file(&report);
    }

    #[test]
    fn finds_the_first_installed_terminal() {
        let r = FakeRunner::default().respond(&["sh", "-c"], "konsole\nxterm\n", true);
        let a = find(
            &r,
            "KDE",
            &shell_in("distrobox", "vinci-free", "x", Path::new("/r")),
        )
        .unwrap();
        assert_eq!(a[..3], ["konsole", "--separate", "-e"]);
        // Every candidate is passed as an argument, never spliced into the script.
        let call = &r.calls()[0];
        assert!(call.contains(&"xterm".to_string()));

        let none = FakeRunner::default().respond(&["sh", "-c"], "", true);
        assert!(find(&none, "KDE", &[]).is_none());
    }
}
