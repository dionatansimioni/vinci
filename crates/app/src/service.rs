//! `vinci --watch`: the background service that converts files dropped in the watch
//! folders. No window; a tray icon when the desktop has one (KDE, GNOME with the
//! AppIndicator extension) and notifications either way (SPEC §9).

use crate::i18n::tr;
use relm4::gtk::{gio, glib, prelude::*};
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
    /// The app icon drawn at a few sizes. Sent as pixels: an AppImage's icon isn't in the
    /// desktop's icon theme, and a tray that can't find the named icon shows nothing.
    pixmaps: Vec<ksni::Icon>,
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
        if self.pixmaps.is_empty() {
            crate::APP_ID.into()
        } else {
            String::new()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        open_window();
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmaps.clone()
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
                activate: Box::new(|_| open_window()),
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
                label: tr("Quit Vinci"),
                activate: Box::new(|t: &mut Self| {
                    // The window may be hidden in the tray: it goes too.
                    app_action(
                        "org.gtk.Actions",
                        "Activate",
                        (
                            "quit",
                            Vec::<glib::Variant>::new(),
                            glib::VariantDict::new(None).end(),
                        )
                            .to_variant(),
                    );
                    t.quit.store(true, Ordering::Relaxed);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

const APP_PATH: &str = "/io/github/dionatansimioni/Vinci";

/// Calls `method` on the running Vinci window's D-Bus interface (GApplication exports
/// it). False when no window process is running.
fn app_action(interface: &str, method: &str, args: glib::Variant) -> bool {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .and_then(|bus| {
            bus.call_sync(
                Some(crate::APP_ID),
                APP_PATH,
                interface,
                method,
                Some(&args),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                5000,
                gio::Cancellable::NONE,
            )
        })
        .is_ok()
}

/// Shows the window: the one hidden in the tray if Vinci is running, else a new one.
fn open_window() {
    let shown = app_action(
        "org.freedesktop.Application",
        "Activate",
        (glib::VariantDict::new(None).end(),).to_variant(),
    );
    if !shown {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
    }
}

/// Whether closing the window can leave Vinci in the tray: this service is running and
/// the desktop has a tray to show its icon (stock GNOME doesn't).
pub fn tray_available() -> bool {
    let service_running = single_instance().is_none();
    let watcher = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .and_then(|bus| {
            bus.call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                Some(&("org.kde.StatusNotifierWatcher",).to_variant()),
                Some(glib::VariantTy::new("(b)").unwrap()),
                gio::DBusCallFlags::NONE,
                2000,
                gio::Cancellable::NONE,
            )
        })
        .ok()
        .and_then(|v| v.get::<(bool,)>())
        .is_some_and(|(b,)| b);
    service_running && watcher
}

/// What closing the window does: hide it to the tray, or quit.
pub fn hide_on_close(settings: &Settings, tray: bool) -> bool {
    settings.close_to_tray && tray
}

const ICON_SVG: &[u8] = include_bytes!("../../../data/icons/io.github.dionatansimioni.Vinci.svg");

/// The app icon as ARGB32 pixels (what StatusNotifierItem wants) at common tray sizes.
/// Empty when the image loader can't read SVG; the tray then asks for it by name.
fn icon_pixmaps() -> Vec<ksni::Icon> {
    use relm4::gtk::{gdk_pixbuf::Pixbuf, gio, glib};
    [16, 22, 24, 32, 48, 64]
        .into_iter()
        .filter_map(|size| {
            let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(ICON_SVG));
            let pb =
                Pixbuf::from_stream_at_scale(&stream, size, size, true, gio::Cancellable::NONE)
                    .ok()?
                    .add_alpha(false, 0, 0, 0)
                    .ok()?;
            let (w, h, stride) = (pb.width(), pb.height(), pb.rowstride() as usize);
            let bytes = pb.read_pixel_bytes();
            let data = (0..h as usize)
                .flat_map(|y| bytes[y * stride..][..w as usize * 4].as_chunks::<4>().0)
                .flat_map(|&[r, g, b, a]| [a, r, g, b])
                .collect();
            Some(ksni::Icon {
                width: w,
                height: h,
                data,
            })
        })
        .collect()
}

fn notify(summary: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .appname("Vinci")
        .icon(crate::APP_ID)
        .summary(summary)
        .body(body)
        .show();
}

/// An exclusive lock on a file in the runtime dir, held as long as the file stays open.
fn runtime_lock(name: &str) -> Option<std::fs::File> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let file = std::fs::File::create(dir.join(name)).ok()?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).ok()?;
    Some(file)
}

/// Keeps a single service per user: the lock lives as long as the process.
fn single_instance() -> Option<std::fs::File> {
    runtime_lock("vinci-watch.lock")
}

static APP_LOCK: std::sync::OnceLock<Option<std::fs::File>> = std::sync::OnceLock::new();

/// Marks the window as open for the service, which keeps the tray icon while it is.
pub fn hold_app_lock() {
    APP_LOCK.get_or_init(|| runtime_lock("vinci-app.lock"));
}

/// Whether a Vinci window is open: it holds the lock, so taking it fails.
fn app_open() -> bool {
    runtime_lock("vinci-app.lock").is_none()
}

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    /// Nothing to do and no window: the service (and its icon) goes away.
    Stop,
    /// Only the tray icon, while the window is open.
    Idle,
    Convert,
}

fn mode(settings: &Settings, app_open: bool) -> Mode {
    let folders = settings.watch_import.is_some() || settings.watch_export.is_some();
    if settings.background_service && folders {
        Mode::Convert
    } else if app_open {
        Mode::Idle
    } else {
        Mode::Stop
    }
}

/// The container to convert in, plus the default import preset for this machine.
fn engine() -> Option<(Engine, ImportPreset, Option<gpu::Gpu>)> {
    // Free first: its import preset suits both editions.
    let inst = State::load(&State::default_path())
        .ok()?
        .installations
        .into_iter()
        .next()?;
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
    // At login this can start before the desktop's tray (Cinnamon's xapp-sn-watcher, KDE's
    // kded): the icon then shows up as soon as the tray does. Stock GNOME never has one,
    // and the service keeps working with notifications only.
    let tray = {
        use ksni::blocking::TrayMethods;
        let spawned = Tray {
            status: tr("Watching your folders"),
            pixmaps: icon_pixmaps(),
            paused: paused.clone(),
            quit: quit.clone(),
        }
        .assume_sni_available(true)
        .spawn();
        if let Err(e) = &spawned {
            rli_core::log::info(&format!("tray icon unavailable: {e}"));
        }
        spawned.ok()
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
        match mode(&settings, app_open()) {
            Mode::Stop => break,
            Mode::Idle => {
                set_status(if settings.background_service {
                    tr("No folder to watch: choose one in Preferences")
                } else {
                    tr("Background conversion is off")
                });
                std::thread::sleep(POLL);
                continue;
            }
            Mode::Convert => {}
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
                Kind::Import(p) => {
                    convert::convert_import(&engine, &job.input, &job.out_dir, p, progress)
                }
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

#[cfg(test)]
mod tests {
    use super::{hide_on_close, mode, Mode};
    use rli_core::settings::Settings;

    /// The icon stays while the window is open; conversions need folders and the switch.
    #[test]
    fn runs_while_the_window_is_open_or_there_is_work() {
        let mut s = Settings::default();
        assert_eq!(mode(&s, false), Mode::Stop);
        assert_eq!(mode(&s, true), Mode::Idle);
        s.watch_import = Some("/v".into());
        assert_eq!(mode(&s, false), Mode::Convert);
        s.background_service = false;
        assert_eq!(mode(&s, false), Mode::Stop);
        assert_eq!(mode(&s, true), Mode::Idle);
    }

    /// Closing hides to the tray only when the user wants it and there is a tray.
    #[test]
    fn closing_hides_to_the_tray_only_when_there_is_one() {
        let mut s = Settings::default();
        assert!(s.close_to_tray, "on by default");
        assert!(hide_on_close(&s, true));
        assert!(
            !hide_on_close(&s, false),
            "no tray: quitting is the only way out"
        );
        s.close_to_tray = false;
        assert!(!hide_on_close(&s, true));
    }

    /// The tray gets the icon as pixels it can always show, at every size, fully opaque
    /// somewhere (not a blank square).
    #[test]
    fn draws_the_tray_icon() {
        let icons = super::icon_pixmaps();
        assert_eq!(icons.len(), 6, "the SVG loader is missing");
        for i in &icons {
            assert_eq!(i.data.len(), (i.width * i.height * 4) as usize);
            assert!(i.data.as_chunks::<4>().0.iter().any(|px| px[0] == 255));
        }
        assert_eq!((icons[1].width, icons[1].height), (22, 22));
    }
}
