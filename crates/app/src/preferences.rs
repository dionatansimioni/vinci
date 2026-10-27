//! Preferences dialog. Every change is saved immediately to settings.toml.

use crate::i18n::tr;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gio;
use rli_core::gpu::Gpu;
use rli_core::settings::{self, Settings, LANGUAGES};
use std::cell::RefCell;
use std::path::PathBuf;
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

/// Shows the dialog. `on_change` runs after any setting is saved.
pub fn present(parent: &impl IsA<gtk::Widget>, gpus: &[Gpu], on_change: impl Fn() + 'static) {
    let store = Rc::new(RefCell::new(Settings::load(&Settings::default_path())));
    let on_change = Rc::new(on_change);
    let window = parent.root().and_downcast::<gtk::Window>();

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

    let safety = adw::PreferencesGroup::builder()
        .title(tr("Updates and disk space"))
        .build();
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
        window,
    ));

    let page = adw::PreferencesPage::new();
    page.add(&general);
    page.add(&safety);
    page.add(&media);
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title(&tr("Preferences"));
    dialog.add(&page);
    dialog.present(Some(parent));
}
