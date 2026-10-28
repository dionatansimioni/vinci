//! Running conversions: ffmpeg inside the Resolve container, live progress, and the
//! bookkeeping for watch folders.

use crate::ffmpeg::{self, Encoder, ExportPreset, VideoCodec};
use crate::gpu::Vendor;
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
    pub fn wrap(&self, argv: &[String]) -> Vec<String> {
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

/// Encoders that pass a test encode in this engine, best first, always ending with the
/// CPU. A GPU without its video driver (e.g. AMD without mesa-va-drivers) is skipped here
/// instead of failing every file.
pub fn working_encoders(
    engine: &Engine,
    codec: VideoCodec,
    vendor: Option<Vendor>,
    render_node: Option<&Path>,
) -> Vec<Encoder> {
    let runner = crate::host::HostRunner::new();
    let list = ["ffmpeg", "-hide_banner", "-encoders"].map(String::from);
    let available = runner
        .run_plain(&engine.wrap(&list))
        .map(|o| ffmpeg::parse_encoders(&o))
        .unwrap_or_default();
    ffmpeg::encoder_candidates(codec, vendor, render_node, &available)
        .into_iter()
        .filter(|e| {
            *e == Encoder::Cpu
                || runner
                    .run_plain(&engine.wrap(&ffmpeg::probe_command(e, codec)))
                    .is_ok()
        })
        .collect()
}

/// Exports `input` with the first encoder that works: when a hardware encoder fails on
/// this file, its partial output is deleted and the next one (finally the CPU) is tried.
pub fn convert_export(
    engine: &Engine,
    input: &Path,
    out_dir: &Path,
    preset: ExportPreset,
    encoders: &[Encoder],
    mut on_progress: impl FnMut(Option<f64>),
) -> Result<PathBuf, ConvertError> {
    let mut last = ConvertError::NoOutput;
    for encoder in encoders {
        let command = ffmpeg::export_command(input, out_dir, preset, encoder);
        let out = command.1.clone();
        match convert_file(engine, command, input, &mut on_progress) {
            Ok(p) => return Ok(p),
            Err(e @ ConvertError::Failed { .. }) => {
                // The name was free before this attempt, so whatever is there is ours.
                let _ = std::fs::remove_file(&out);
                crate::log::info(&format!(
                    "{encoder:?} failed for this file, trying the next encoder"
                ));
                last = e;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
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

    /// The first encoder fails on the file: its partial output goes, the next one runs.
    /// Uses a fake ffmpeg (a shell script) so it runs anywhere.
    #[test]
    fn falls_back_to_the_next_encoder_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("rli-fallback-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let input = dir.join("render.mov");
        std::fs::write(&input, "x").unwrap();
        // Writes the output (last argument), then fails when asked for NVENC.
        let fake = bin.join("ffmpeg");
        std::fs::write(
            &fake,
            "#!/bin/sh\nfor a; do out=$a; done\necho partial > \"$out\"\ncase \"$*\" in *nvenc*) echo 'nvenc broke' >&2; exit 1;; esac\necho progress=end\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        // convert_file looks ffmpeg up on PATH; the fake only lives for this process's
        // children, which inherit the variable set here.
        std::env::set_var("PATH", &path);
        let out = convert_export(
            &Engine::Direct,
            &input,
            &dir,
            ExportPreset::Web,
            &[Encoder::Nvenc, Encoder::Cpu],
            |_| {},
        )
        .unwrap();
        let files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".mp4"))
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(out.file_name().unwrap(), "render_web.mp4");
        assert_eq!(
            files,
            ["render_web.mp4"],
            "the failed attempt left nothing behind"
        );
    }

    /// Against a real Resolve container: a VAAPI device without its driver (the error a
    /// Ryzen user hit) is filtered out by the probe, and a forced VAAPI attempt falls back
    /// to the CPU. Run with VINCI_DBX=<distrobox> VINCI_CONTAINER=<name> and `--ignored`.
    #[test]
    #[ignore]
    fn real_broken_vaapi_falls_back_to_cpu() {
        let (Ok(distrobox), Ok(container)) =
            (std::env::var("VINCI_DBX"), std::env::var("VINCI_CONTAINER"))
        else {
            return;
        };
        let engine = Engine::Container {
            distrobox,
            container,
        };
        // Under $HOME: the only host folder the container is sure to see.
        let dir = crate::state::home().join(".cache/vinci/vaapi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("render.mov");
        let made = crate::host::HostRunner::new().run_plain(
            &engine.wrap(
                &[
                    "ffmpeg",
                    "-y",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=640x360:rate=30:duration=2",
                    "-c:v",
                    "mpeg4",
                ]
                .iter()
                .map(|s| s.to_string())
                .chain(std::iter::once(input.to_string_lossy().into_owned()))
                .collect::<Vec<_>>(),
            ),
        );
        assert!(made.is_ok(), "{made:?}");
        let node = Path::new("/dev/dri/renderD128");
        let usable = working_encoders(&engine, VideoCodec::H264, Some(Vendor::Amd), Some(node));
        assert_eq!(usable, [Encoder::Cpu], "VAAPI without a driver is skipped");
        let forced = [
            Encoder::Vaapi {
                device: node.into(),
            },
            Encoder::Cpu,
        ];
        let out = convert_export(&engine, &input, &dir, ExportPreset::Web, &forced, |_| {});
        let ok = out.as_ref().is_ok_and(|p| p.exists());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(ok, "{out:?}");
    }
}
