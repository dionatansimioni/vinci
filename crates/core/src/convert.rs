//! Running conversions: ffmpeg inside the Resolve container, live progress, and the
//! bookkeeping for watch folders.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

/// Media we offer to convert. Lower-case extensions.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "mkv", "m4v", "mts", "m2ts", "avi", "webm", "3gp", "m4a", "aac", "mp3",
];

/// Runs `argv` inside the container (ffmpeg and ffprobe live there, from Ubuntu's repos).
pub fn in_container(distrobox: &str, container: &str, argv: &[String]) -> Vec<String> {
    let mut v: Vec<String> = [distrobox, "enter", "--name", container, "--"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    v.extend(argv.iter().cloned());
    v
}

pub fn duration_command(input: &Path) -> Vec<String> {
    [
        "ffprobe",
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain(std::iter::once(input.to_string_lossy().into_owned()))
    .collect()
}

pub fn parse_duration(out: &str) -> Option<Duration> {
    let secs: f64 = out.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Media time encoded so far.
    At(Duration),
    End,
}

/// Parses one line of `ffmpeg -progress pipe:1` output.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let (key, value) = line.trim().split_once('=')?;
    match key {
        // ffmpeg names this "_ms" but it is in microseconds; "_us" is the newer spelling.
        "out_time_us" | "out_time_ms" => value
            .parse::<u64>()
            .ok()
            .map(|us| Progress::At(Duration::from_micros(us))),
        "progress" if value == "end" => Some(Progress::End),
        _ => None,
    }
}

/// Fraction done in 0..=1, when the total duration is known.
pub fn fraction(at: Duration, total: Option<Duration>) -> Option<f64> {
    let total = total.filter(|t| !t.is_zero())?;
    Some((at.as_secs_f64() / total.as_secs_f64()).clamp(0.0, 1.0))
}

/// Runs `argv`, calling `on_line` for each stdout line as it arrives. Returns success and
/// the tail of stderr for error reports.
pub fn run_streaming(
    argv: &[String],
    mut on_line: impl FnMut(&str),
) -> std::io::Result<(bool, String)> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::other("empty command"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = child.stderr.take().expect("piped");
    // Drain stderr on its own thread so a chatty ffmpeg can't block on a full pipe.
    let err_thread = std::thread::spawn(move || {
        let mut tail: Vec<String> = Vec::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            tail.push(line);
            if tail.len() > 30 {
                tail.remove(0);
            }
        }
        tail.join("\n")
    });
    for line in BufReader::new(child.stdout.take().expect("piped")).lines() {
        on_line(&line?);
    }
    let ok = child.wait()?.success();
    Ok((ok, err_thread.join().unwrap_or_default()))
}

/// Where and how to run ffmpeg: inside a container, or directly (tests, CI).
#[derive(Debug, Clone)]
pub enum Engine {
    Container {
        distrobox: String,
        container: String,
    },
    Direct,
}

impl Engine {
    fn wrap(&self, argv: &[String]) -> Vec<String> {
        match self {
            Self::Container {
                distrobox,
                container,
            } => in_container(distrobox, container, argv),
            Self::Direct => argv.to_vec(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("could not start ffmpeg: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("ffmpeg failed")]
    Failed { stderr: String },
    #[error("ffmpeg reported success but produced no file")]
    NoOutput,
}

/// Converts one file: probes its length, runs ffmpeg with live progress (0..=1 when the
/// length is known), and checks the output really exists before reporting success.
pub fn convert_file(
    engine: &Engine,
    command: (Vec<String>, PathBuf),
    input: &Path,
    mut on_progress: impl FnMut(Option<f64>),
) -> Result<PathBuf, ConvertError> {
    let total = crate::host::HostRunner::new()
        .run_plain(&engine.wrap(&duration_command(input)))
        .ok()
        .and_then(|o| parse_duration(&o));
    let (argv, out) = command;
    let mode = crate::host::ExecMode::detect();
    let (ok, stderr) = run_streaming(
        &mode.wrap(&engine.wrap(&argv)),
        |line| match parse_progress(line) {
            Some(Progress::At(t)) => on_progress(fraction(t, total)),
            Some(Progress::End) => on_progress(Some(1.0)),
            None => {}
        },
    )?;
    if !ok {
        return Err(ConvertError::Failed { stderr });
    }
    // ffmpeg can exit 0 without writing (e.g. -n on an existing file); trust the disk.
    if !out.exists() {
        return Err(ConvertError::NoOutput);
    }
    Ok(out)
}

/// Files in a watch folder that are ready to convert: media, not already handled, and
/// unchanged for `settle` (so we don't grab a file that is still being copied).
pub fn pending(
    dir: &Path,
    done: &BTreeSet<PathBuf>,
    settle: Duration,
    now: SystemTime,
) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| MEDIA_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .filter(|p| !done.contains(p))
        .filter(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .is_ok_and(|m| now.duration_since(m).is_ok_and(|age| age >= settle))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_commands_for_the_container() {
        let argv = in_container(
            "distrobox",
            "vinci-free",
            &["ffmpeg".into(), "-version".into()],
        );
        assert_eq!(
            argv,
            [
                "distrobox",
                "enter",
                "--name",
                "vinci-free",
                "--",
                "ffmpeg",
                "-version"
            ]
        );
    }

    #[test]
    fn parses_ffprobe_duration() {
        assert_eq!(
            parse_duration("12.500000\n"),
            Some(Duration::from_millis(12500))
        );
        assert_eq!(parse_duration("N/A"), None);
        assert_eq!(parse_duration("-1"), None);
    }

    #[test]
    fn parses_progress_lines() {
        assert_eq!(
            parse_progress("out_time_us=2500000"),
            Some(Progress::At(Duration::from_millis(2500)))
        );
        assert_eq!(
            parse_progress("out_time_ms=1000000"),
            Some(Progress::At(Duration::from_secs(1)))
        );
        assert_eq!(parse_progress("progress=end"), Some(Progress::End));
        assert_eq!(parse_progress("progress=continue"), None);
        assert_eq!(parse_progress("out_time_us=N/A"), None);
        assert_eq!(parse_progress("fps=30"), None);
    }

    #[test]
    fn computes_fraction() {
        let s = Duration::from_secs;
        assert_eq!(fraction(s(5), Some(s(10))), Some(0.5));
        assert_eq!(fraction(s(15), Some(s(10))), Some(1.0));
        assert_eq!(fraction(s(5), None), None);
        assert_eq!(fraction(s(5), Some(s(0))), None);
    }

    #[test]
    fn streams_lines_and_captures_stderr() {
        let argv: Vec<String> = ["sh", "-c", "echo a; echo b; echo oops >&2; exit 3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut lines = Vec::new();
        let (ok, err) = run_streaming(&argv, |l| lines.push(l.to_string())).unwrap();
        assert!(!ok);
        assert_eq!(lines, ["a", "b"]);
        assert_eq!(err, "oops");
    }

    /// Real conversion with progress, using ffmpeg on PATH (run inside the container or
    /// CI image with `--ignored`).
    #[test]
    #[ignore]
    fn real_convert_with_progress() {
        use crate::ffmpeg::{import_command, ImportPreset};
        let dir = std::env::temp_dir().join(format!("rli-conv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("clip.mp4");
        let made = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=30:duration=4",
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=duration=4",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(made);
        let mut seen = Vec::new();
        let out = convert_file(
            &Engine::Direct,
            import_command(&src, &dir, ImportPreset::DnxhrHq),
            &src,
            |f| seen.push(f),
        )
        .unwrap();
        let exists = out.exists();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(exists);
        assert_eq!(seen.last(), Some(&Some(1.0)));
        assert!(
            seen.iter().flatten().any(|f| *f > 0.0 && *f < 1.0),
            "intermediate progress: {seen:?}"
        );
    }

    #[test]
    fn picks_settled_media_files_only() {
        let dir = std::env::temp_dir().join(format!("rli-watch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["a.MP4", "b.mov", "notes.txt", "c.mkv"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let done: BTreeSet<PathBuf> = [dir.join("c.mkv")].into();
        let later = SystemTime::now() + Duration::from_secs(60);
        let found = pending(&dir, &done, Duration::from_secs(10), later);
        assert_eq!(found, [dir.join("a.MP4"), dir.join("b.mov")]);
        let too_soon = pending(
            &dir,
            &BTreeSet::new(),
            Duration::from_secs(10),
            SystemTime::now(),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(too_soon.is_empty(), "files still being written are skipped");
    }
}
