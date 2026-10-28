//! ffmpeg command lines for the codec workarounds.
//!
//! On Linux, Resolve Free can't decode H.264/H.265, no edition handles AAC, and only
//! Studio on NVIDIA encodes H.264/H.265. Import presets turn camera/phone media into
//! something Resolve reads; export presets turn a Resolve render into a web-ready MP4.

use crate::gpu::Vendor;
use crate::installer_file::Edition;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportPreset {
    /// DNxHR HQ video + PCM audio in .mov. Readable by every edition.
    DnxhrHq,
    /// ProRes 422 video + PCM audio in .mov.
    Prores422,
    /// Keep the video stream, only convert the audio to PCM (Studio on NVIDIA).
    AudioOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSettings {
    pub codec: VideoCodec,
    /// Constant-quality level on the x264/x265 CRF scale (lower = better).
    pub quality: u8,
    pub audio_kbps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPreset {
    /// "YouTube/Web": high-quality H.264 + AAC 320k.
    Web,
    /// "Arquivo H.265": smaller files for archiving.
    Archive,
    Custom(ExportSettings),
}

impl ExportPreset {
    pub fn settings(self) -> ExportSettings {
        match self {
            Self::Web => ExportSettings {
                codec: VideoCodec::H264,
                quality: 18,
                audio_kbps: 320,
            },
            Self::Archive => ExportSettings {
                codec: VideoCodec::H265,
                quality: 22,
                audio_kbps: 256,
            },
            Self::Custom(s) => s,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Archive => "h265",
            Self::Custom(_) => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Encoder {
    Nvenc,
    /// VAAPI needs the render node of the GPU, e.g. `/dev/dri/renderD128`.
    Vaapi {
        device: PathBuf,
    },
    Qsv,
    Cpu,
}

/// Preset chosen by default for the import tab and watch folder.
pub fn default_import(edition: Edition, vendor: Option<Vendor>) -> ImportPreset {
    match (edition, vendor) {
        (Edition::Studio, Some(Vendor::Nvidia)) => ImportPreset::AudioOnly,
        _ => ImportPreset::DnxhrHq,
    }
}

/// Parses `ffmpeg -hide_banner -encoders` into encoder names.
pub fn parse_encoders(output: &str) -> BTreeSet<String> {
    output
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("------"))
        .skip(1)
        .filter_map(|l| {
            let mut cols = l.split_whitespace();
            let flags = cols.next()?;
            (flags.len() == 6).then(|| cols.next().map(str::to_string))?
        })
        .collect()
}

/// Encoders to try for `codec` on this GPU, best first. The CPU is always last, so there
/// is something to fall back to when a hardware encoder turns out not to work.
pub fn encoder_candidates(
    codec: VideoCodec,
    vendor: Option<Vendor>,
    render_node: Option<&Path>,
    available: &BTreeSet<String>,
) -> Vec<Encoder> {
    let has = |kind: &str| available.contains(&format!("{}_{kind}", codec_prefix(codec)));
    let vaapi = render_node
        .filter(|_| has("vaapi"))
        .map(|d| Encoder::Vaapi {
            device: d.to_path_buf(),
        });
    let mut out: Vec<Encoder> = match vendor {
        Some(Vendor::Nvidia) => has("nvenc").then_some(Encoder::Nvenc).into_iter().collect(),
        Some(Vendor::Intel) => has("qsv")
            .then_some(Encoder::Qsv)
            .into_iter()
            .chain(vaapi)
            .collect(),
        Some(Vendor::Amd) => vaapi.into_iter().collect(),
        _ => Vec::new(),
    };
    out.push(Encoder::Cpu);
    out
}

/// A short test encode of a generated clip: proves `encoder` really works here (the
/// driver is installed and the device opens) before a real file is handed to it.
pub fn probe_command(encoder: &Encoder, codec: VideoCodec) -> Vec<String> {
    let settings = ExportSettings {
        codec,
        quality: 23,
        audio_kbps: 128,
    };
    let (pre, video) = video_args(encoder, settings);
    let mut a = args(&["ffmpeg", "-hide_banner", "-nostdin", "-v", "error"]);
    a.extend(pre);
    a.extend(args(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=320x240:r=30:d=0.5",
    ]));
    a.extend(video);
    a.extend(args(&["-f", "null", "-"]));
    a
}

fn codec_prefix(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "h264",
        VideoCodec::H265 => "hevc",
    }
}

/// `<stem>_<suffix>.<ext>` in `out_dir`, or `<stem>_<suffix> (2).<ext>` and so on when
/// taken. ffmpeg's `-n` exits 0 when it refuses to overwrite, so we never hand it an
/// existing path in the first place; `-n` stays as a second guard against races.
fn output_path(input: &Path, out_dir: &Path, suffix: &str, ext: &str) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let base = format!("{stem}_{suffix}");
    (1..)
        .map(|n| match n {
            1 => out_dir.join(format!("{base}.{ext}")),
            n => out_dir.join(format!("{base} ({n}).{ext}")),
        })
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Common head: never prompt, never overwrite (`-n`), machine-readable progress.
fn head(pre_input: &[String], input: &Path) -> Vec<String> {
    let mut a = args(&[
        "ffmpeg",
        "-hide_banner",
        "-nostdin",
        "-n",
        "-progress",
        "pipe:1",
    ]);
    a.extend(pre_input.iter().cloned());
    a.push("-i".into());
    a.push(input.to_string_lossy().into_owned());
    a
}

/// Command and output file for converting `input` so Resolve can import it.
pub fn import_command(
    input: &Path,
    out_dir: &Path,
    preset: ImportPreset,
) -> (Vec<String>, PathBuf) {
    let (suffix, body): (&str, &[&str]) = match preset {
        ImportPreset::DnxhrHq => (
            "dnxhr",
            &[
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-fps_mode",
                "cfr",
                "-c:v",
                "dnxhd",
                "-profile:v",
                "dnxhr_hq",
                "-pix_fmt",
                "yuv422p",
                "-c:a",
                "pcm_s16le",
            ],
        ),
        ImportPreset::Prores422 => (
            "prores",
            &[
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-fps_mode",
                "cfr",
                "-c:v",
                "prores_ks",
                "-profile:v",
                "2",
                "-vendor",
                "apl0",
                "-pix_fmt",
                "yuv422p10le",
                "-c:a",
                "pcm_s16le",
            ],
        ),
        ImportPreset::AudioOnly => (
            "pcm",
            &[
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-c:v",
                "copy",
                "-c:a",
                "pcm_s16le",
            ],
        ),
    };
    let out = output_path(input, out_dir, suffix, "mov");
    let mut a = head(&[], input);
    a.extend(args(body));
    a.push(out.to_string_lossy().into_owned());
    (a, out)
}

/// Options before the input (device setup) and the video encoding options.
fn video_args(encoder: &Encoder, s: ExportSettings) -> (Vec<String>, Vec<String>) {
    let q = s.quality.to_string();
    let prefix = codec_prefix(s.codec);
    let mut pre = Vec::new();
    let mut video = match encoder {
        Encoder::Nvenc => args(&[
            "-c:v",
            &format!("{prefix}_nvenc"),
            "-preset",
            "p6",
            "-rc",
            "vbr",
            "-cq",
            &q,
            "-b:v",
            "0",
        ]),
        Encoder::Vaapi { device } => {
            pre = args(&["-vaapi_device", &device.to_string_lossy()]);
            args(&[
                "-vf",
                "format=nv12,hwupload",
                "-c:v",
                &format!("{prefix}_vaapi"),
                "-qp",
                &q,
            ])
        }
        Encoder::Qsv => args(&["-c:v", &format!("{prefix}_qsv"), "-global_quality", &q]),
        Encoder::Cpu => {
            let lib = match s.codec {
                VideoCodec::H264 => "libx264",
                VideoCodec::H265 => "libx265",
            };
            args(&[
                "-c:v", lib, "-preset", "slow", "-crf", &q, "-pix_fmt", "yuv420p",
            ])
        }
    };
    if s.codec == VideoCodec::H265 {
        // Lets Apple devices and browsers recognize HEVC in MP4.
        video.extend(args(&["-tag:v", "hvc1"]));
    }
    (pre, video)
}

/// Command and output file for turning a Resolve render into a distributable MP4.
pub fn export_command(
    input: &Path,
    out_dir: &Path,
    preset: ExportPreset,
    encoder: &Encoder,
) -> (Vec<String>, PathBuf) {
    let s = preset.settings();
    let (pre, video) = video_args(encoder, s);
    let out = output_path(input, out_dir, preset.suffix(), "mp4");
    let mut a = head(&pre, input);
    a.extend(args(&["-map", "0:v:0", "-map", "0:a?"]));
    a.extend(video);
    a.extend(args(&[
        "-c:a",
        "aac",
        "-b:a",
        &format!("{}k", s.audio_kbps),
        "-movflags",
        "+faststart",
    ]));
    a.push(out.to_string_lossy().into_owned());
    (a, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENCODERS: &str = "Encoders:
 V..... = Video
 ------
 V....D libx264              libx264 H.264 / AVC / MPEG-4 AVC
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 V....D h264_vaapi           H.264/AVC (VAAPI) (codec h264)
 V....D hevc_qsv             HEVC (Intel Quick Sync Video acceleration)
 A....D aac                  AAC (Advanced Audio Coding)
";

    fn has(a: &[String], pair: [&str; 2]) -> bool {
        a.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1])
    }

    #[test]
    fn parses_encoder_list() {
        let e = parse_encoders(ENCODERS);
        assert!(e.contains("h264_nvenc"));
        assert!(e.contains("aac"));
        assert!(!e.contains("Video"), "legend lines are skipped");
        assert_eq!(e.len(), 5);
    }

    #[test]
    fn default_import_follows_edition_and_gpu() {
        assert_eq!(
            default_import(Edition::Studio, Some(Vendor::Nvidia)),
            ImportPreset::AudioOnly
        );
        assert_eq!(
            default_import(Edition::Studio, Some(Vendor::Amd)),
            ImportPreset::DnxhrHq
        );
        assert_eq!(
            default_import(Edition::Free, Some(Vendor::Nvidia)),
            ImportPreset::DnxhrHq
        );
        assert_eq!(default_import(Edition::Free, None), ImportPreset::DnxhrHq);
    }

    #[test]
    fn hardware_encoders_first_and_the_cpu_always_last() {
        let e = parse_encoders(ENCODERS);
        let node = Path::new("/dev/dri/renderD128");
        let vaapi = Encoder::Vaapi {
            device: node.into(),
        };
        use VideoCodec::*;
        let c = |codec, vendor, node| encoder_candidates(codec, vendor, node, &e);
        assert_eq!(
            c(H264, Some(Vendor::Nvidia), None),
            [Encoder::Nvenc, Encoder::Cpu]
        );
        assert_eq!(
            c(H264, Some(Vendor::Amd), Some(node)),
            [vaapi, Encoder::Cpu]
        );
        assert_eq!(c(H264, Some(Vendor::Amd), None), [Encoder::Cpu]);
        assert_eq!(
            c(H265, Some(Vendor::Intel), Some(node)),
            [Encoder::Qsv, Encoder::Cpu]
        );
        assert_eq!(
            c(H265, Some(Vendor::Nvidia), None),
            [Encoder::Cpu],
            "no hevc_nvenc"
        );
        assert_eq!(c(H264, None, Some(node)), [Encoder::Cpu]);
    }

    #[test]
    fn probe_encodes_a_generated_clip_with_the_same_options() {
        let vaapi = Encoder::Vaapi {
            device: "/dev/dri/renderD128".into(),
        };
        let a = probe_command(&vaapi, VideoCodec::H264);
        assert!(has(&a, ["-vaapi_device", "/dev/dri/renderD128"]));
        assert!(has(&a, ["-c:v", "h264_vaapi"]));
        assert!(has(&a, ["-f", "lavfi"]));
        assert_eq!(a[a.len() - 3..], ["-f", "null", "-"], "writes nothing");
    }

    #[test]
    fn import_commands() {
        let (a, out) = import_command(
            Path::new("/v/clip.MP4"),
            Path::new("/out"),
            ImportPreset::DnxhrHq,
        );
        assert_eq!(out, Path::new("/out/clip_dnxhr.mov"));
        assert!(has(&a, ["-profile:v", "dnxhr_hq"]));
        assert!(has(&a, ["-c:a", "pcm_s16le"]));
        assert!(has(&a, ["-fps_mode", "cfr"]), "phone footage is often VFR");
        assert!(a.contains(&"-n".to_string()), "never overwrite");
        assert_eq!(a.last().unwrap(), "/out/clip_dnxhr.mov");

        let (a, out) = import_command(
            Path::new("/v/clip.mov"),
            Path::new("/out"),
            ImportPreset::AudioOnly,
        );
        assert_eq!(out, Path::new("/out/clip_pcm.mov"));
        assert!(has(&a, ["-c:v", "copy"]));
        assert!(
            !a.contains(&"-fps_mode".to_string()),
            "can't retime a copied stream"
        );

        let (a, _) = import_command(
            Path::new("/v/c.mp4"),
            Path::new("/o"),
            ImportPreset::Prores422,
        );
        assert!(has(&a, ["-c:v", "prores_ks"]));
    }

    #[test]
    fn never_reuses_an_existing_output_name() {
        let dir = std::env::temp_dir().join(format!("rli-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip_dnxhr.mov"), "").unwrap();
        std::fs::write(dir.join("clip_dnxhr (2).mov"), "").unwrap();
        let (a, out) = import_command(Path::new("/v/clip.mp4"), &dir, ImportPreset::DnxhrHq);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(out, dir.join("clip_dnxhr (3).mov"));
        assert_eq!(a.last().unwrap(), &out.to_string_lossy());
    }

    #[test]
    fn export_commands() {
        let input = Path::new("/r/render.mov");
        let (a, out) = export_command(input, Path::new("/o"), ExportPreset::Web, &Encoder::Nvenc);
        assert_eq!(out, Path::new("/o/render_web.mp4"));
        assert!(has(&a, ["-c:v", "h264_nvenc"]));
        assert!(has(&a, ["-cq", "18"]));
        assert!(has(&a, ["-b:a", "320k"]));
        assert!(has(&a, ["-movflags", "+faststart"]));

        let vaapi = Encoder::Vaapi {
            device: "/dev/dri/renderD129".into(),
        };
        let (a, _) = export_command(input, Path::new("/o"), ExportPreset::Archive, &vaapi);
        let dev = a.iter().position(|x| x == "-vaapi_device").unwrap();
        assert!(
            dev < a.iter().position(|x| x == "-i").unwrap(),
            "device is an input option"
        );
        assert!(has(&a, ["-c:v", "hevc_vaapi"]));
        assert!(has(&a, ["-tag:v", "hvc1"]));

        let custom = ExportPreset::Custom(ExportSettings {
            codec: VideoCodec::H264,
            quality: 23,
            audio_kbps: 192,
        });
        let (a, out) = export_command(input, Path::new("/o"), custom, &Encoder::Cpu);
        assert_eq!(out, Path::new("/o/render_custom.mp4"));
        assert!(has(&a, ["-c:v", "libx264"]));
        assert!(has(&a, ["-crf", "23"]));
        assert!(has(&a, ["-b:a", "192k"]));
    }

    /// End-to-end with a real ffmpeg: builds an H.264/AAC clip (what phones produce),
    /// runs every import preset and the CPU export presets, and checks the codecs.
    /// Needs ffmpeg/ffprobe on PATH; run inside the container with `--ignored`.
    #[test]
    #[ignore]
    fn real_ffmpeg_roundtrip() {
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!("rli-ffmpeg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("phone.mp4");
        let ok = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=30:duration=2",
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
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
        assert!(ok, "could not create the source clip");
        let probe = |p: &Path, stream: &str| {
            let out = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    stream,
                    "-show_entries",
                    "stream=codec_name",
                    "-of",
                    "csv=p=0",
                ])
                .arg(p)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let run = |argv: &[String]| {
            let out = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        for (preset, vcodec) in [
            (ImportPreset::DnxhrHq, "dnxhd"),
            (ImportPreset::Prores422, "prores"),
            (ImportPreset::AudioOnly, "h264"),
        ] {
            let (argv, out) = import_command(&src, &dir, preset);
            run(&argv);
            assert_eq!(probe(&out, "v:0"), vcodec, "{preset:?}");
            assert_eq!(probe(&out, "a:0"), "pcm_s16le", "{preset:?}");
        }
        let render = dir.join("phone_dnxhr.mov");
        for (preset, vcodec) in [(ExportPreset::Web, "h264"), (ExportPreset::Archive, "hevc")] {
            let (argv, out) = export_command(&render, &dir, preset, &Encoder::Cpu);
            run(&argv);
            assert_eq!(probe(&out, "v:0"), vcodec, "{preset:?}");
            assert_eq!(probe(&out, "a:0"), "aac", "{preset:?}");
        }
        // A second run must not touch the first output: it picks a new name.
        let first = dir.join("phone_dnxhr_web.mp4");
        let before = std::fs::metadata(&first).unwrap().modified().unwrap();
        let (argv, out) = export_command(&render, &dir, ExportPreset::Web, &Encoder::Cpu);
        assert_eq!(out, dir.join("phone_dnxhr_web (2).mp4"));
        run(&argv);
        assert_eq!(
            std::fs::metadata(&first).unwrap().modified().unwrap(),
            before
        );
        assert_eq!(probe(&out, "v:0"), "h264");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
