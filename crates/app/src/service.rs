//! `vinci --watch`: the background service that converts files dropped in the watch
//! folders. No window; a tray icon when the desktop has one (KDE, GNOME with the
//! AppIndicator extension) and notifications either way (SPEC §9).

use crate::i18n::tr;
use rli_core::convert::{self, Engine};
use rli_core::ffmpeg::{self, Encoder, ImportPreset, VideoCodec};
use rli_core::host::HostRunner;
use rli_core::recipe::Recipe;
use rli_core::settings::Settings;
use rli_core::state::{self, State};
use rli_core::watch::{self, Done, Kind};
use rli_core::{gpu, tools};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const POLL: Duration = Duration::from_secs(10);

struct Tray {
    status: String,
    paused: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        crate::APP_ID.into()
    }
    fn title(&self) -> String {
        "Vinci".into()
    }
    fn icon_name(&self) -> String {
        crate::APP_ID.into()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Vinci".into(),
            description: self.status.clone(),
            ..Default::default()
        }
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let paused = self.paused.load(Ordering::Relaxed);
        vec![
            StandardItem {
                label: self.status.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: tr("Open Vinci"),
                activate: Box::new(|_| {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).spawn();
                    }
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: if paused {
                    tr("Resume conversions")
                } else {
                    tr("Pause conversions")
                },
                activate: Box::new(|t: &mut Self| {
                    t.paused.fetch_xor(true, Ordering::Relaxed);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: tr("Quit background service"),
                activate: Box::new(|t: &mut Self| t.quit.store(true, Ordering::Relaxed)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn notify(summary: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .appname("Vinci")
        .icon(crate::APP_ID)
        .summary(summary)
        .body(body)
        .show();
}

/// Keeps a single service per user: the lock lives as long as the process.
fn single_instance() -> Option<std::fs::File> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let file = std::fs::File::create(dir.join("vinci-watch.lock")).ok()?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).ok()?;
    Some(file)
}

/// The container to convert in, plus the default import preset for this machine.
fn engine() -> Option<(Engine, ImportPreset, Option<gpu::Gpu>)> {
    let inst = State::load(&State::default_path()).ok()?.installation?;
    let dbx = tools::detect(&HostRunner::new(), &Recipe::bundled(), &state::home())
        .distrobox
        .executable()?;
    let gpus = gpu::detect(std::path::Path::new("/"));
    let chosen = Settings::load(&Settings::default_path())
        .choose_gpu(&gpus)
        .cloned();
    let preset = ffmpeg::default_import(inst.edition, chosen.as_ref().map(|g| g.vendor));
    Some((
        Engine::Container {
            distrobox: dbx,
            container: inst.container,
        },
        preset,
        chosen,
    ))
}

pub fn run() -> i32 {
    let Some(_lock) = single_instance() else {
        return 0; // already running
    };
    let paused = Arc::new(AtomicBool::new(false));
    let quit = Arc::new(AtomicBool::new(false));
    // No StatusNotifierWatcher (stock GNOME): keep working with notifications only.
    let tray = {
        use ksni::blocking::TrayMethods;
        Tray {
            status: tr("Watching your folders"),
            paused: paused.clone(),
            quit: quit.clone(),
        }
        .spawn()
        .ok()
    };
    let set_status = |s: String| {
        if let Some(t) = &tray {
            t.update(move |t| t.status = s);
        }
    };

    let done_path = Done::default_path();
    let mut done = Done::load(&done_path);
    // Probing encoders takes a few seconds; do it once per codec.
    let mut encoders: Option<(VideoCodec, Vec<Encoder>)> = None;
    while !quit.load(Ordering::Relaxed) {
        let settings = Settings::load(&Settings::default_path());
        if !settings.background_service
            || (settings.watch_import.is_none() && settings.watch_export.is_none())
        {
            break;
        }
        if paused.load(Ordering::Relaxed) {
            set_status(tr("Paused"));
            std::thread::sleep(POLL);
            continue;
        }
        let Some((engine, preset, gpu)) = engine() else {
            set_status(tr("Waiting for DaVinci Resolve to be installed"));
            std::thread::sleep(POLL);
            continue;
        };
        let jobs = watch::jobs(&settings, preset, &done, SystemTime::now());
        for job in jobs {
            if quit.load(Ordering::Relaxed) || paused.load(Ordering::Relaxed) {
                break;
            }
            let name = job
                .input
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let _ = std::fs::create_dir_all(&job.out_dir);
            set_status(format!("{} {name}", tr("Converting")));
            let progress = |f: Option<f64>| {
                if let Some(f) = f {
                    set_status(format!("{} {name} ({:.0}%)", tr("Converting"), f * 100.0));
                }
            };
            let result = match job.kind {
                Kind::Import(p) => convert::convert_file(
                    &engine,
                    ffmpeg::import_command(&job.input, &job.out_dir, p),
                    &job.input,
                    progress,
                ),
                Kind::Export(p) => {
                    let codec = p.settings().codec;
                    if encoders.as_ref().is_none_or(|(c, _)| *c != codec) {
                        let found = crate::media::export_encoders(&engine, codec, gpu.as_ref());
                        encoders = Some((codec, found));
                    }
                    let list = encoders.as_ref().map(|(_, e)| e.as_slice()).unwrap_or(&[]);
                    convert::convert_export(&engine, &job.input, &job.out_dir, p, list, progress)
                }
            };
            match result {
                Ok(out) => notify(
                    &tr("Conversion finished"),
                    &format!(
                        "{name} → {}",
                        out.file_name().unwrap_or_default().to_string_lossy()
                    ),
                ),
                Err(e) => notify(&tr("Conversion failed"), &format!("{name}: {e}")),
            }
            // Failed files are marked too, so a broken file isn't retried forever.
            done.mark(&job.input);
            done.prune();
            let _ = done.save(&done_path);
        }
        set_status(tr("Watching your folders"));
        std::thread::sleep(POLL);
    }
    if let Some(t) = tray {
        t.shutdown().wait();
    }
    0
}
