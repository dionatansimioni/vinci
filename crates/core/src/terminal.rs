//! Opening a shell inside the Resolve container in the user's terminal emulator, for
//! debugging and installing extra plugins by hand.

use crate::host::{argv, Runner};

/// Terminals we know how to start with a command, and the flag that precedes it.
/// An empty flag means the command follows the program name directly.
const KNOWN: [(&str, &[&str]); 14] = [
    ("ptyxis", &["--"]),
    ("kgx", &["--"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("mate-terminal", &["-x"]),
    ("terminator", &["-x"]),
    ("tilix", &["-e"]),
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
    } else if desktop.contains("xfce") {
        &["xfce4-terminal"]
    } else if desktop.contains("mate") {
        &["mate-terminal"]
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

/// An interactive shell inside `container`.
pub fn shell_in(distrobox: &str, container: &str) -> Vec<String> {
    argv(&[distrobox, "enter", "--name", container])
}

/// The first installed terminal (checked on the host in one call), ready to run `inner`.
pub fn find(runner: &dyn Runner, desktop: &str, inner: &[String]) -> Option<Vec<String>> {
    let list = candidates(desktop);
    let mut probe = argv(&[
        "sh",
        "-c",
        r#"for t in "$@"; do command -v "$t" >/dev/null 2>&1 && echo "$t"; done"#,
        "sh",
    ]);
    probe.extend(list.iter().map(|s| s.to_string()));
    let out = runner.run(&probe).ok()?;
    let found = out.stdout.lines().next()?.trim().to_string();
    command(&found, inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fake::FakeRunner;

    #[test]
    fn prefers_the_desktops_own_terminal() {
        assert_eq!(candidates("KDE")[0], "konsole");
        assert_eq!(candidates("ubuntu:GNOME")[0], "ptyxis");
        assert_eq!(candidates("")[0], "ptyxis");
        let all = candidates("KDE");
        assert_eq!(all.len(), KNOWN.len(), "no duplicates, nothing lost");
    }

    #[test]
    fn builds_the_command_per_terminal() {
        let inner = shell_in("/usr/bin/distrobox", "vinci-free");
        assert_eq!(
            command("konsole", &inner).unwrap(),
            [
                "konsole",
                "-e",
                "/usr/bin/distrobox",
                "enter",
                "--name",
                "vinci-free"
            ]
        );
        assert_eq!(
            command("gnome-terminal", &inner).unwrap()[..2],
            ["gnome-terminal", "--"]
        );
        assert_eq!(command("kitty", &inner).unwrap()[1], "/usr/bin/distrobox");
        assert!(command("rm", &inner).is_none());
    }

    #[test]
    fn finds_the_first_installed_terminal() {
        let r = FakeRunner::default().respond(&["sh", "-c"], "konsole\nxterm\n", true);
        let a = find(&r, "KDE", &shell_in("distrobox", "vinci-free")).unwrap();
        assert_eq!(a[..2], ["konsole", "-e"]);
        // Every candidate is passed as an argument, never spliced into the script.
        let call = &r.calls()[0];
        assert!(call.contains(&"xterm".to_string()));

        let none = FakeRunner::default().respond(&["sh", "-c"], "", true);
        assert!(find(&none, "KDE", &[]).is_none());
    }
}
