//! Optional activity log (off by default): every command Vinci runs, its output and exit
//! code, in a local file the user can open or attach to a bug report. Nothing is ever
//! sent anywhere, and the home folder (which carries the user name) is written as `~`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Past this size the log moves to `vinci.log.1` (replacing an older one) and starts over.
const MAX_BYTES: u64 = 5 * 1024 * 1024;

struct Log {
    path: PathBuf,
    file: std::fs::File,
    written: u64,
    home: PathBuf,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// `$XDG_STATE_HOME/vinci/logs` on the host.
pub fn dir() -> PathBuf {
    crate::state::host_xdg_dir("XDG_STATE_HOME", ".local/state").join("vinci/logs")
}

pub fn default_path() -> PathBuf {
    dir().join("vinci.log")
}

/// Starts writing to `path` (or stops, with `None`). Called at startup and whenever the
/// setting changes.
pub fn enable(path: Option<&Path>, home: &Path) {
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    *log = path.and_then(|p| open(p, home.to_path_buf()).ok());
    drop(log);
    if path.is_some() {
        info(&format!("Vinci {} started", env!("CARGO_PKG_VERSION")));
    }
}

pub fn is_enabled() -> bool {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

fn open(path: &Path, home: PathBuf) -> std::io::Result<Log> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let written = file.metadata()?.len();
    Ok(Log {
        path: path.to_path_buf(),
        file,
        written,
        home,
    })
}

fn write(text: &str) {
    let mut guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(log) = guard.as_mut() else {
        return;
    };
    if log.written > MAX_BYTES {
        let _ = std::fs::rename(&log.path, log.path.with_extension("log.1"));
        match open(&log.path, log.home.clone()) {
            Ok(fresh) => *log = fresh,
            Err(_) => {
                *guard = None;
                return;
            }
        }
    }
    let line = format!(
        "{} {}\n",
        timestamp(std::time::SystemTime::now()),
        crate::diagnostics::anonymize(text, &log.home)
    );
    if log.file.write_all(line.as_bytes()).is_ok() {
        log.written += line.len() as u64;
    }
}

pub fn info(text: &str) {
    write(text);
}

/// A command about to run.
pub fn command(argv: &[String]) {
    if is_enabled() {
        write(&format!("$ {}", argv.join(" ")));
    }
}

/// Output printed by the running command (one or more lines).
pub fn output(text: &str) {
    if !is_enabled() {
        return;
    }
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        write(&format!("  {line}"));
    }
}

pub fn finished(code: Option<i32>) {
    if is_enabled() {
        match code {
            Some(c) => write(&format!("  → exit {c}")),
            None => write("  → stopped by a signal"),
        }
    }
}

/// `2026-10-28 14:03:07` in UTC (no time-zone database needed, and logs from different
/// machines compare directly).
fn timestamp(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01 00:00:00");
        let t = UNIX_EPOCH + Duration::from_secs(1_793_196_187);
        assert_eq!(timestamp(t), "2026-10-28 14:03:07");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(timestamp(leap), "2000-02-29 00:00:00");
    }

    /// One test drives the global log end to end, so parallel tests never race on it.
    #[test]
    fn writes_only_when_enabled_hides_home_and_rotates() {
        let dir = std::env::temp_dir().join(format!("rli-log-{}", std::process::id()));
        let path = dir.join("vinci.log");
        let home = Path::new("/home/maria");

        command(&["before".into()]);
        enable(Some(&path), home);
        assert!(is_enabled());
        command(&["ls".into(), "/home/maria/Videos".into()]);
        output("line one\n\nline two");
        finished(Some(3));
        enable(None, home);
        command(&["after".into()]);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("$ ls ~/Videos"), "{text}");
        assert!(!text.contains("maria"));
        assert!(text.contains("  line one\n") && text.contains("  line two\n"));
        assert!(!text.contains("before") && !text.contains("after"));
        assert!(text.contains("→ exit 3"));

        // Over the limit: the next line starts a fresh file and keeps one old one.
        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        enable(Some(&path), home);
        info("fresh");
        enable(None, home);
        let fresh = std::fs::read_to_string(&path).unwrap();
        let old = std::fs::metadata(path.with_extension("log.1"))
            .unwrap()
            .len();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(fresh.contains("fresh") && fresh.len() < 1000);
        assert!(old > MAX_BYTES);
    }
}
