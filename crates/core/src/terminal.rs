//! Opening a shell inside the Resolve container in the user's terminal emulator, for
//! debugging and installing extra plugins by hand.

use crate::host::{argv, Runner};

/// Terminals we know how to start with a command, and the flag that precedes it.
/// An empty flag means the command follows the program name directly. Every one of them
/// takes the command as separate arguments (no single quoted string).
const KNOWN: &[(&str, &[&str])] = &[
    // The freedesktop way to run the user's chosen terminal (Ubuntu 25.10, Fedora 43).
    ("xdg-terminal-exec", &[]),
    ("ptyxis", &["--"]),
    ("kgx", &["--"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
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

/// An interactive shell inside `container`. If it ends with an error (the container
/// would not start, say), the window stays open with `on_error` until Enter is pressed,
/// instead of flashing and closing.
pub fn shell_in(distrobox: &str, container: &str, on_error: &str) -> Vec<String> {
    argv(&[
        "sh",
        "-c",
        r#"msg=$1; shift; "$@" || { printf '\n%s\n' "$msg"; read -r _; }"#,
        "sh",
        on_error,
        distrobox,
        "enter",
        "--name",
        container,
    ])
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
        let inner = shell_in("/usr/bin/distrobox", "vinci-free", "Failed");
        let konsole = command("konsole", &inner).unwrap();
        assert_eq!(konsole[..3], ["konsole", "-e", "sh"]);
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

    /// The wrapper runs the shell, and only on failure waits with the message.
    #[test]
    fn a_failing_shell_keeps_the_window_open_with_a_message() {
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
        // `false` stands in for a distrobox that fails; `true` for a clean exit.
        let fails = shell_in("false", "c", "It failed. Press Enter.");
        assert_eq!(run(&fails, "\n"), "\nIt failed. Press Enter.\n");
        let works = shell_in("true", "c", "It failed. Press Enter.");
        assert_eq!(run(&works, ""), "");
    }

    #[test]
    fn finds_the_first_installed_terminal() {
        let r = FakeRunner::default().respond(&["sh", "-c"], "konsole\nxterm\n", true);
        let a = find(&r, "KDE", &shell_in("distrobox", "vinci-free", "x")).unwrap();
        assert_eq!(a[..2], ["konsole", "-e"]);
        // Every candidate is passed as an argument, never spliced into the script.
        let call = &r.calls()[0];
        assert!(call.contains(&"xterm".to_string()));

        let none = FakeRunner::default().respond(&["sh", "-c"], "", true);
        assert!(find(&none, "KDE", &[]).is_none());
    }
}
