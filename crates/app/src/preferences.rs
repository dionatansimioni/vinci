//! Preferences dialog. Every change is saved immediately to settings.toml.

use crate::i18n::tr;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gio;
use rli_core::gpu::Gpu;
use rli_core::host::HostRunner;
use rli_core::recipe::Recipe;
use rli_core::settings::{self, Settings, LANGUAGES};
use rli_core::state::Installation;
use rli_core::{terminal, tools};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn save(s: &Settings) {
    let _ = s.save(&Settings::default_path());
}

fn videos_dir() -> PathBuf {
    gtk::glib::user_special_dir(gtk::glib::UserDirectory::Videos)
        .unwrap_or_else(|| rli_core::state::home().join("Videos"))
}

/// A row that shows a folder, lets the user pick another one, or turn it off.
fn folder_row(
    title: &str,
    current: Option<PathBuf>,
    default: PathBuf,
    store: Rc<RefCell<Settings>>,
    set: fn(&mut Settings, Option<PathBuf>),
    window: Option<gtk::Window>,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    let label = |p: &Option<PathBuf>| {
        p.as_ref()
            .map_or_else(|| tr("Off"), |p| p.display().to_string())
    };
    row.set_subtitle(&label(&current));
    let toggle = gtk::Switch::builder()
        .active(current.is_some())
        .valign(gtk::Align::Center)
        .build();
    let choose = gtk::Button::from_icon_name("folder-symbolic");
    choose.set_valign(gtk::Align::Center);
    choose.set_tooltip_text(Some(&tr("Choose folder…")));
    choose.set_sensitive(current.is_some());
    row.add_suffix(&choose);
    row.add_suffix(&toggle);

    toggle.connect_active_notify({
        let (row, store, choose) = (row.clone(), store.clone(), choose.clone());
        move |t| {
            let value = t.is_active().then(|| default.clone());
            if let Some(dir) = &value {
                let _ = std::fs::create_dir_all(dir);
            }
            set(&mut store.borrow_mut(), value.clone());
            save(&store.borrow());
            row.set_subtitle(&label(&value));
            choose.set_sensitive(value.is_some());
        }
    });
    choose.connect_clicked({
        let row = row.clone();
        move |_| {
            let dialog = gtk::FileDialog::builder()
                .title(tr("Choose folder…"))
                .build();
            let (row, store) = (row.clone(), store.clone());
            dialog.select_folder(window.as_ref(), None::<&gio::Cancellable>, move |res| {
                if let Some(path) = res.ok().and_then(|f| f.path()) {
                    row.set_subtitle(&path.display().to_string());
                    set(&mut store.borrow_mut(), Some(path));
                    save(&store.borrow());
                }
            });
        }
    });
    row
}

/// Opens `dir` in the file manager (through the portal inside the Flatpak).
fn open_folder(dir: &Path, window: Option<&gtk::Window>) {
    gtk::FileLauncher::new(Some(&gio::File::for_path(dir))).launch(
        window,
        None::<&gio::Cancellable>,
        |_| {},
    );
}

/// A shell inside the Resolve container, in the user's terminal emulator. When none can
/// be started, or it closes with an error right away, the reason and the command to type
/// by hand.
fn open_terminal(container: &str, parent: &adw::PreferencesDialog) {
    let runner = HostRunner::new();
    let dbx = tools::detect(&runner, &Recipe::bundled(), &rli_core::state::home())
        .distrobox
        .executable();
    let manual = terminal::manual_command(dbx.as_deref().unwrap_or("distrobox"), container);
    let Some(dbx) = dbx else {
        show_manual_terminal(parent, &tr("distrobox is missing"), manual);
        return;
    };
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let on_error =
        tr("The container's shell has ended (exit code in parentheses). Press Enter to close.");
    let report = rli_core::log::dir().join("terminal.status");
    let _ = std::fs::create_dir_all(rli_core::log::dir());
    let _ = std::fs::remove_file(&report);
    let rc = terminal::rc_path(&rli_core::state::home());
    let welcome = tr("You are inside the {container} container. Type exit to leave.")
        .replace("{container}", container);
    let _ = std::fs::create_dir_all(rc.parent().unwrap_or(&rc))
        .and_then(|()| std::fs::write(&rc, terminal::container_rc(container, &welcome)));
    let Some(argv) = terminal::find(
        &runner,
        &desktop,
        &terminal::shell_in(&dbx, container, &on_error, &report, &rc),
    ) else {
        show_manual_terminal(
            parent,
            &tr("No terminal app found on this computer."),
            manual,
        );
        return;
    };
    let err_path = std::env::temp_dir().join(format!("vinci-terminal-{}.log", std::process::id()));
    let started = std::fs::File::create(&err_path)
        .and_then(|err| runner.spawn_window(&argv, &activation_env(parent), err));
    let mut child = match started {
        Ok(c) => c,
        Err(e) => {
            rli_core::log::info(&format!("terminal did not start: {e}"));
            show_manual_terminal(parent, &e.to_string(), manual);
            return;
        }
    };
    // Some terminals hand the window to a server and exit 0 at once (GNOME Terminal);
    // a quick exit with an error means no window ever showed up.
    let parent = parent.clone();
    let opened = std::time::Instant::now();
    gtk::glib::spawn_future_local(async move {
        let status = gio::spawn_blocking(move || child.wait()).await;
        let errors = std::fs::read_to_string(&err_path).unwrap_or_default();
        let _ = std::fs::remove_file(&err_path);
        let Ok(Ok(status)) = status else { return };
        let elapsed = opened.elapsed();
        let mut shell = terminal::read_report(&report);
        if shell == terminal::ShellRun::NeverStarted && status.success() {
            // A terminal server (GNOME Terminal) may run the command a moment after the
            // launcher has exited.
            gtk::glib::timeout_future_seconds(5).await;
            shell = terminal::read_report(&report);
        }
        rli_core::log::info(&format!(
            "terminal ended ({status}) after {}s, shell: {shell:?}{}",
            elapsed.as_secs(),
            if errors.trim().is_empty() {
                String::new()
            } else {
                format!(":\n{}", errors.trim_end())
            }
        ));
        // Without a report the command never ran: the user saw no shell at all.
        let quick = !status.success() && elapsed < std::time::Duration::from_secs(5);
        if quick || shell == terminal::ShellRun::NeverStarted {
            let last = errors.lines().rev().find(|l| !l.trim().is_empty());
            let reason = match last {
                Some(l) => {
                    tr("The terminal closed right away: {error}").replace("{error}", l.trim())
                }
                None => tr("The terminal closed right away."),
            };
            show_manual_terminal(&parent, &reason, manual);
        }
    });
}

/// Variables that let the new window take the focus. On Wayland a window opened without
/// an activation token from the app the user clicked in stays behind it (KDE, GNOME),
/// as if nothing had happened.
fn activation_env(widget: &impl IsA<gtk::Widget>) -> Vec<(String, String)> {
    let display = widget.display();
    if !display.backend().is_wayland() {
        return Vec::new();
    }
    display
        .app_launch_context()
        .startup_notify_id(None::<&gio::AppInfo>, &[])
        .map(|token| {
            ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
                .iter()
                .map(|k| (k.to_string(), token.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// When no terminal could be opened: the command to type in one, ready to copy.
fn show_manual_terminal(parent: &adw::PreferencesDialog, reason: &str, command: String) {
    let body = tr("{reason}\n\nOpen any terminal app and run this command to get a shell inside DaVinci Resolve's container:")
        .replace("{reason}", reason);
    let dialog = adw::AlertDialog::new(Some(&tr("Open the terminal yourself")), Some(&body));
    let field = gtk::Entry::builder()
        .text(&command)
        .editable(false)
        .css_classes(["monospace"])
        .margin_top(6)
        .build();
    dialog.set_extra_child(Some(&field));
    dialog.add_responses(&[("close", &tr("Close")), ("copy", &tr("Copy command"))]);
    dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("copy"));
    let parent_widget = parent.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "copy" {
            parent_widget.clipboard().set_text(&command);
            parent_widget.add_toast(adw::Toast::new(&tr("Command copied")));
        }
    });
    dialog.present(Some(parent));
}

/// Tools for advanced users: a shell in the container and the folder with Resolve's data.
fn advanced_group(
    installed: Vec<Installation>,
    dialog: &adw::PreferencesDialog,
    window: Option<gtk::Window>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Advanced"))
        .build();
    let row_with_button = |title: String, subtitle: String, icon: &str| {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(subtitle)
            .subtitle_lines(0)
            .activatable(true)
            .build();
        let b = gtk::Button::from_icon_name(icon);
        b.set_valign(gtk::Align::Center);
        b.add_css_class("flat");
        row.add_suffix(&b);
        row.set_activatable_widget(Some(&b));
        (row, b)
    };

    // One row per install; a single disabled one when nothing is installed yet.
    let several = installed.len() > 1;
    let targets: Vec<Option<Installation>> = if installed.is_empty() {
        vec![None]
    } else {
        installed.into_iter().map(Some).collect()
    };
    for target in targets {
        let title = match &target {
            Some(i) if several => tr("Open a terminal in the {name} container")
                .replace("{name}", crate::messages::edition_name(i.edition)),
            _ => tr("Open a terminal in the container"),
        };
        let (term_row, term) = row_with_button(
            title,
            tr("A shell inside DaVinci Resolve's container, for troubleshooting or installing extra plugins. Use sudo for system changes."),
            "utilities-terminal-symbolic",
        );
        term_row.set_sensitive(target.is_some());
        term.connect_clicked({
            let (dialog, container) = (dialog.clone(), target.map(|i| i.container));
            move |_| {
                if let Some(c) = &container {
                    open_terminal(c, &dialog);
                }
            }
        });
        group.add(&term_row);
    }

    let data = rli_core::uninstall::resolve_data_dir(&rli_core::state::home());
    let (folder_row, folder) = row_with_button(
        tr("Open the installation folder"),
        tr("Projects, database, settings, LUTs and logs of DaVinci Resolve ({path}). The program itself is inside the container, in /opt/resolve: reach it with the terminal above.")
            .replace("{path}", &data.display().to_string()),
        "folder-open-symbolic",
    );
    folder_row.set_sensitive(data.exists());
    folder.connect_clicked(move |_| open_folder(&data, window.as_ref()));
    group.add(&folder_row);
    group
}

/// Shows the dialog. `on_change` runs after any setting is saved.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    gpus: &[Gpu],
    installed: Vec<Installation>,
    on_change: impl Fn() + 'static,
) {
    let store = Rc::new(RefCell::new(Settings::load(&Settings::default_path())));
    let on_change = Rc::new(on_change);
    let window = parent.root().and_downcast::<gtk::Window>();

    let switch = |title: String,
                  subtitle: String,
                  get: fn(&Settings) -> bool,
                  set: fn(&mut Settings, bool)| {
        let row = adw::SwitchRow::builder()
            .title(title)
            .subtitle(subtitle)
            .active(get(&store.borrow()))
            .build();
        let (store, on_change) = (store.clone(), on_change.clone());
        row.connect_active_notify(move |r| {
            set(&mut store.borrow_mut(), r.is_active());
            save(&store.borrow());
            on_change();
        });
        row
    };
    let general = adw::PreferencesGroup::builder()
        .title(tr("General"))
        .build();
    let mut lang_names = vec![tr("System language")];
    lang_names.extend(LANGUAGES.iter().map(|(_, n)| n.to_string()));
    let lang_refs: Vec<&str> = lang_names.iter().map(String::as_str).collect();
    let current_lang = store
        .borrow()
        .language
        .as_deref()
        .and_then(|l| LANGUAGES.iter().position(|(c, _)| *c == l))
        .map_or(0, |i| i + 1);
    let language = adw::ComboRow::builder()
        .title(tr("Language"))
        .subtitle(tr("Takes effect the next time Vinci opens."))
        .model(&gtk::StringList::new(&lang_refs))
        .selected(current_lang as u32)
        .build();
    language.connect_selected_notify({
        let (store, on_change) = (store.clone(), on_change.clone());
        move |r| {
            let i = r.selected() as usize;
            store.borrow_mut().language = i.checked_sub(1).map(|i| LANGUAGES[i].0.to_string());
            save(&store.borrow());
            on_change();
        }
    });
    general.add(&language);

    let usable: Vec<Gpu> = gpus.iter().filter(|g| g.is_usable()).cloned().collect();
    if usable.len() > 1 {
        let mut names = vec![tr("Automatic")];
        names.extend(usable.iter().map(Gpu::display_name));
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let current = store
            .borrow()
            .preferred_gpu
            .as_deref()
            .and_then(|slot| {
                usable
                    .iter()
                    .position(|g| g.pci_slot.as_deref() == Some(slot))
            })
            .map_or(0, |i| i + 1);
        let row = adw::ComboRow::builder()
            .title(tr("GPU for DaVinci Resolve"))
            .subtitle(tr(
                "Applies to the menu entry the next time Vinci repairs or installs it.",
            ))
            .model(&gtk::StringList::new(&refs))
            .selected(current as u32)
            .build();
        row.connect_selected_notify({
            let (store, on_change) = (store.clone(), on_change.clone());
            move |r| {
                let i = r.selected() as usize;
                store.borrow_mut().preferred_gpu =
                    i.checked_sub(1).and_then(|i| usable[i].pci_slot.clone());
                save(&store.borrow());
                on_change();
            }
        });
        general.add(&row);
    }

    general.add(&switch(
        tr("Keep running in the tray when closed"),
        tr("Closing the window leaves Vinci in the system tray. Turn off to quit instead."),
        |s| s.close_to_tray,
        // Choosing here answers the question asked on the first close.
        |s, v| {
            s.close_to_tray = v;
            s.close_asked = true;
        },
    ));

    let safety = adw::PreferencesGroup::builder()
        .title(tr("Updates and disk space"))
        .build();
    safety.add(&switch(
        tr("Keep a restore point"),
        tr("Saves the current version before each update so you can go back (5–10 GB)."),
        |s| s.keep_restore_point,
        |s, v| s.keep_restore_point = v,
    ));
    safety.add(&switch(
        tr("Keep the installer"),
        tr("Allows reinstalling without downloading again (3–12 GB)."),
        |s| s.keep_installer,
        |s, v| s.keep_installer = v,
    ));

    let media = adw::PreferencesGroup::builder()
        .title(tr("Watch folders"))
        .description(tr(
            "Files placed in these folders are converted automatically.",
        ))
        .build();
    media.add(&switch(
        tr("Convert in the background"),
        tr("Keeps converting with Vinci closed, with an icon in the system tray."),
        |s| s.background_service,
        |s, v| s.background_service = v,
    ));
    let (import_default, export_default) = settings::default_watch_dirs(&videos_dir());
    let current = store.borrow().clone();
    media.add(&folder_row(
        &tr("Import folder (convert for editing)"),
        current.watch_import,
        import_default,
        store.clone(),
        |s, v| s.watch_import = v,
        window.clone(),
    ));
    media.add(&folder_row(
        &tr("Export folder (convert renders to MP4)"),
        current.watch_export,
        export_default,
        store.clone(),
        |s, v| s.watch_export = v,
        window.clone(),
    ));

    let logs = adw::PreferencesGroup::builder().title(tr("Logs")).build();
    let save_logs = switch(
        tr("Save activity logs"),
        tr("Off by default. Records the commands Vinci runs and what they print in a file on this computer, with your user name hidden. Nothing is sent anywhere; attach the file yourself when asking for help."),
        |s| s.save_logs,
        |s, v| s.save_logs = v,
    );
    logs.add(&save_logs);
    let log_dir = rli_core::log::dir();
    let open_logs = adw::ActionRow::builder()
        .title(tr("Open the logs folder"))
        .subtitle(log_dir.display().to_string())
        .activatable(true)
        .sensitive(log_dir.exists())
        .build();
    let open_logs_button = gtk::Button::from_icon_name("folder-open-symbolic");
    open_logs_button.set_valign(gtk::Align::Center);
    open_logs_button.add_css_class("flat");
    open_logs.add_suffix(&open_logs_button);
    open_logs.set_activatable_widget(Some(&open_logs_button));
    open_logs_button.connect_clicked({
        let (dir, window) = (log_dir.clone(), window.clone());
        move |_| open_folder(&dir, window.as_ref())
    });
    // The app starts the log a moment later (on its settings message); make the folder
    // now so the button works right away.
    save_logs.connect_active_notify({
        let open_logs = open_logs.clone();
        move |r| {
            if r.is_active() {
                let _ = std::fs::create_dir_all(&log_dir);
            }
            open_logs.set_sensitive(log_dir.exists());
        }
    });
    logs.add(&open_logs);

    let dialog = adw::PreferencesDialog::new();
    let page = adw::PreferencesPage::new();
    page.add(&general);
    page.add(&safety);
    page.add(&media);
    page.add(&advanced_group(installed, &dialog, window));
    page.add(&logs);
    dialog.set_title(&tr("Preferences"));
    dialog.add(&page);
    dialog.present(Some(parent));
}
