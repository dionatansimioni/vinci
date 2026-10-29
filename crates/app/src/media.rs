//! "Media" tab: converts files Resolve can't open on Linux, and renders into MP4.

use crate::i18n::tr;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gio;
use relm4::prelude::*;
use rli_core::convert::{self, Engine};
use rli_core::ffmpeg::{self, Encoder, ExportPreset, ImportPreset, VideoCodec};
use rli_core::gpu::Gpu;
use rli_core::host::HostRunner;
use rli_core::recipe::Recipe;
use rli_core::state::{self, State};
use rli_core::tools;
use std::path::PathBuf;

/// The presets offered in the drop-down, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Import(ImportPreset),
    Export(ExportPreset),
}

pub const CHOICES: [Choice; 8] = [
    Choice::Import(ImportPreset::DnxhrHq),
    Choice::Import(ImportPreset::Prores422),
    Choice::Import(ImportPreset::Mpeg4 { short_side: None }),
    Choice::Import(ImportPreset::Mpeg4 {
        short_side: Some(1080),
    }),
    Choice::Import(ImportPreset::Mpeg4 {
        short_side: Some(720),
    }),
    Choice::Import(ImportPreset::AudioOnly),
    Choice::Export(ExportPreset::Web),
    Choice::Export(ExportPreset::Archive),
];

pub fn choice_label(c: Choice) -> String {
    match c {
        Choice::Import(ImportPreset::DnxhrHq) => {
            tr("For editing: DNxHR HQ (every edition, large files)")
        }
        Choice::Import(ImportPreset::Prores422) => tr("For editing: ProRes 422 (large files)"),
        Choice::Import(ImportPreset::AudioOnly) => {
            tr("For editing: fix audio only (Studio with NVIDIA)")
        }
        Choice::Import(ImportPreset::Mpeg4 { short_side: None }) => {
            tr("For editing: MPEG-4 (Diolinux preset)")
        }
        Choice::Import(ImportPreset::Mpeg4 {
            short_side: Some(side),
        }) => {
            tr("For editing: MPEG-4 {size}p (Diolinux preset)").replace("{size}", &side.to_string())
        }
        Choice::Export(ExportPreset::Web) => tr("For publishing: MP4 H.264 (YouTube, web)"),
        Choice::Export(ExportPreset::Archive) => tr("For publishing: MP4 H.265 (smaller files)"),
        Choice::Export(ExportPreset::Custom(_)) => tr("Custom"),
    }
}

/// How the output size compares with the input, shown under the drop-down. `true` when
/// the files get much bigger, so the note is shown as a warning.
pub fn size_note(c: Choice) -> (String, bool) {
    match c {
        // DNxHR HQ ≈ 180 Mbit/s and ProRes 422 ≈ 150 Mbit/s at 1080p30, against the
        // 10–20 Mbit/s of phone and camera H.264.
        Choice::Import(ImportPreset::DnxhrHq | ImportPreset::Prores422) => (
            tr("Files get about 10 times bigger than phone or camera videos: roughly 1 GB per minute in 1080p and 4 GB in 4K. Check your free space."),
            true,
        ),
        Choice::Import(ImportPreset::AudioOnly) => (
            tr("The video is copied as is; only the audio becomes uncompressed, so files grow a little."),
            false,
        ),
        // Measured: about 3.5 times an H.264 phone clip at the same size, a fraction of
        // DNxHR HQ.
        Choice::Import(ImportPreset::Mpeg4 { short_side }) => {
            let mut note = tr("Diolinux's WinFF preset: MPEG-4 at the highest quality with uncompressed audio, keeping every audio track. Files get a few times bigger than phone videos, but much smaller than DNxHR and ProRes.");
            if let Some(side) = short_side {
                note.push(' ');
                note.push_str(
                    &tr("Resized to {size}p, lighter to edit on modest computers.")
                        .replace("{size}", &side.to_string()),
                );
            }
            (note, true)
        }
        Choice::Export(ExportPreset::Web) => (
            tr("Much smaller than the render from Resolve, with high quality."),
            false,
        ),
        Choice::Export(ExportPreset::Archive | ExportPreset::Custom(_)) => (
            tr("The smallest files: about half the size of H.264 at the same quality."),
            false,
        ),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Status {
    Waiting,
    Running(Option<f64>),
    Done(PathBuf),
    Failed(String),
}

#[derive(Debug)]
struct Job {
    input: PathBuf,
    status: Status,
}

pub struct Media {
    jobs: Vec<Job>,
    choice: usize,
    running: bool,
    gpu: Option<Gpu>,
}

#[derive(Debug)]
pub enum MediaMsg {
    AddFiles,
    Added(Vec<PathBuf>),
    Choose(usize),
    Start,
    Clear,
    /// Opens the file manager at the converted file of job `i`.
    Reveal(usize),
}

#[derive(Debug)]
pub enum MediaCmd {
    Progress(usize, Option<f64>),
    Done(usize, Result<PathBuf, String>),
    AllDone,
}

pub struct MediaWidgets {
    list: gtk::ListBox,
    rows: Vec<adw::ActionRow>,
    bars: Vec<gtk::ProgressBar>,
    reveal: Vec<gtk::Button>,
    size_row: adw::ActionRow,
    size_icon: gtk::Image,
    start: gtk::Button,
    empty: adw::StatusPage,
    content: gtk::Box,
}

/// The installed container and a distrobox to reach it, if Resolve is installed.
fn engine() -> Option<(Engine, State)> {
    let st = State::load(&State::default_path()).ok()?;
    // Free first: its import preset suits both editions.
    let inst = st.installations.first().cloned()?;
    let host = tools::detect(&HostRunner::new(), &Recipe::bundled(), &state::home());
    let distrobox = host.distrobox.executable()?;
    Some((
        Engine::Container {
            distrobox,
            container: inst.container,
        },
        st,
    ))
}

/// Encoders that pass a test encode in the container, best first (CPU last).
pub fn export_encoders(engine: &Engine, codec: VideoCodec, gpu: Option<&Gpu>) -> Vec<Encoder> {
    let node = gpu
        .and_then(|g| g.render_node.as_ref())
        .map(|n| PathBuf::from(format!("/dev/dri/{n}")));
    convert::working_encoders(engine, codec, gpu.map(|g| g.vendor), node.as_deref())
}

impl Component for Media {
    type CommandOutput = MediaCmd;
    type Init = Option<Gpu>;
    type Input = MediaMsg;
    type Output = ();
    type Root = gtk::Box;
    type Widgets = MediaWidgets;

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 0)
    }

    fn init(
        gpu: Option<Gpu>,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let edition = State::load(&State::default_path())
            .ok()
            .and_then(|s| s.installations.into_iter().next())
            .map(|i| i.edition);
        let default = edition.map_or(ImportPreset::DnxhrHq, |e| {
            ffmpeg::default_import(e, gpu.as_ref().map(|g| g.vendor))
        });
        let choice = CHOICES
            .iter()
            .position(|c| *c == Choice::Import(default))
            .unwrap_or(0);
        let model = Media {
            jobs: Vec::new(),
            choice,
            running: false,
            gpu,
        };

        let empty = adw::StatusPage::builder()
            .icon_name("video-x-generic-symbolic")
            .title(tr("Prepare media"))
            .description(tr("On Linux, DaVinci Resolve can't open AAC audio, and the free edition can't open H.264/H.265 video, which phones and most cameras record. Convert those files here before importing them, or turn your renders into MP4."))
            .vexpand(true)
            .build();

        let labels: Vec<String> = CHOICES.iter().map(|c| choice_label(*c)).collect();
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let dropdown = gtk::DropDown::from_strings(&label_refs);
        dropdown.set_selected(choice as u32);
        dropdown.connect_selected_notify({
            let sender = sender.clone();
            move |d| sender.input(MediaMsg::Choose(d.selected() as usize))
        });
        let preset_row = adw::ActionRow::builder().title(tr("Convert to")).build();
        dropdown.set_valign(gtk::Align::Center);
        preset_row.add_suffix(&dropdown);
        // Its own row: beside the drop-down the note was squeezed into a thin column.
        let size_row = adw::ActionRow::builder().title_lines(0).build();
        size_row.add_css_class("dim-label");
        let size_icon = gtk::Image::new();
        size_row.add_prefix(&size_icon);
        let preset_group = adw::PreferencesGroup::new();
        preset_group.add(&preset_row);
        preset_group.add(&size_row);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .margin_top(12)
            .build();

        let add = gtk::Button::with_label(&tr("Add files…"));
        add.add_css_class("pill");
        add.connect_clicked({
            let sender = sender.clone();
            move |_| sender.input(MediaMsg::AddFiles)
        });
        let clear = gtk::Button::with_label(&tr("Clear list"));
        clear.add_css_class("pill");
        clear.connect_clicked({
            let sender = sender.clone();
            move |_| sender.input(MediaMsg::Clear)
        });
        let start = gtk::Button::with_label(&tr("Convert"));
        start.add_css_class("pill");
        start.add_css_class("suggested-action");
        start.connect_clicked({
            let sender = sender.clone();
            move |_| sender.input(MediaMsg::Start)
        });
        let buttons = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .margin_top(24)
            .build();
        buttons.append(&clear);
        buttons.append(&add);
        buttons.append(&start);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&preset_group);
        content.append(&list);
        let clamp_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        clamp_box.append(&empty);
        clamp_box.append(&content);
        clamp_box.append(&buttons);
        let clamp = adw::Clamp::builder()
            .maximum_size(640)
            .margin_start(16)
            .margin_end(16)
            .margin_top(12)
            .margin_bottom(24)
            .child(&clamp_box)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        root.append(&scroll);

        let mut widgets = MediaWidgets {
            list,
            rows: Vec::new(),
            bars: Vec::new(),
            reveal: Vec::new(),
            size_row,
            size_icon,
            start,
            empty,
            content,
        };
        Component::update_view(&model, &mut widgets, sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: MediaMsg, sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            MediaMsg::AddFiles => {
                let filter = gtk::FileFilter::new();
                for ext in convert::MEDIA_EXTENSIONS {
                    filter.add_suffix(ext);
                }
                filter.set_name(Some(&tr("Video and audio")));
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                let dialog = gtk::FileDialog::builder()
                    .title(tr("Add files…"))
                    .filters(&filters)
                    .build();
                let window = root.root().and_downcast::<gtk::Window>();
                let sender = sender.clone();
                dialog.open_multiple(window.as_ref(), None::<&gio::Cancellable>, move |res| {
                    let Ok(files) = res else { return };
                    let paths = (0..files.n_items())
                        .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                        .filter_map(|f| f.path())
                        .collect();
                    sender.input(MediaMsg::Added(paths));
                });
            }
            MediaMsg::Added(paths) => {
                for p in paths {
                    if !self.jobs.iter().any(|j| j.input == p) {
                        self.jobs.push(Job {
                            input: p,
                            status: Status::Waiting,
                        });
                    }
                }
            }
            MediaMsg::Choose(i) => self.choice = i.min(CHOICES.len() - 1),
            MediaMsg::Reveal(i) => {
                if let Some(Status::Done(p)) = self.jobs.get(i).map(|j| &j.status) {
                    let window = root.root().and_downcast::<gtk::Window>();
                    gtk::FileLauncher::new(Some(&gio::File::for_path(p))).open_containing_folder(
                        window.as_ref(),
                        None::<&gio::Cancellable>,
                        |_| {},
                    );
                }
            }
            MediaMsg::Clear => {
                if !self.running {
                    self.jobs.clear();
                }
            }
            MediaMsg::Start => {
                let Some((engine, _)) = engine() else {
                    for j in &mut self.jobs {
                        j.status = Status::Failed(tr("Install DaVinci Resolve with Vinci first."));
                    }
                    return;
                };
                let todo: Vec<(usize, PathBuf)> = self
                    .jobs
                    .iter()
                    .enumerate()
                    .filter(|(_, j)| !matches!(j.status, Status::Done(_)))
                    .map(|(i, j)| (i, j.input.clone()))
                    .collect();
                if todo.is_empty() {
                    return;
                }
                self.running = true;
                let choice = CHOICES[self.choice];
                let gpu = self.gpu.clone();
                sender.spawn_command(move |out| {
                    let encoders = match choice {
                        Choice::Export(p) => {
                            export_encoders(&engine, p.settings().codec, gpu.as_ref())
                        }
                        Choice::Import(_) => Vec::new(),
                    };
                    for (i, input) in todo {
                        let dir = input.parent().map(PathBuf::from).unwrap_or_default();
                        let progress = |f| {
                            let _ = out.send(MediaCmd::Progress(i, f));
                        };
                        let result = match choice {
                            Choice::Import(p) => {
                                convert::convert_import(&engine, &input, &dir, p, progress)
                            }
                            Choice::Export(p) => convert::convert_export(
                                &engine, &input, &dir, p, &encoders, progress,
                            ),
                        }
                        .map_err(|e| match e {
                            convert::ConvertError::Failed { stderr } => {
                                stderr.lines().rev().take(3).collect::<Vec<_>>().join(" · ")
                            }
                            other => other.to_string(),
                        });
                        let _ = out.send(MediaCmd::Done(i, result));
                    }
                    let _ = out.send(MediaCmd::AllDone);
                });
            }
        }
    }

    fn update_cmd(&mut self, cmd: MediaCmd, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match cmd {
            MediaCmd::Progress(i, f) => {
                if let Some(j) = self.jobs.get_mut(i) {
                    j.status = Status::Running(f);
                }
            }
            MediaCmd::Done(i, r) => {
                if let Some(j) = self.jobs.get_mut(i) {
                    j.status = match r {
                        Ok(p) => Status::Done(p),
                        Err(e) => Status::Failed(e),
                    };
                }
            }
            MediaCmd::AllDone => self.running = false,
        }
    }

    fn update_view(&self, w: &mut MediaWidgets, _sender: ComponentSender<Self>) {
        let has_jobs = !self.jobs.is_empty();
        w.empty.set_visible(!has_jobs);
        w.content.set_visible(true);
        w.start.set_sensitive(has_jobs && !self.running);
        let (note, bigger) = size_note(CHOICES[self.choice]);
        w.size_row.set_title(&note);
        w.size_row
            .set_css_classes(if bigger { &["warning"] } else { &["dim-label"] });
        w.size_icon.set_icon_name(Some(if bigger {
            "dialog-warning-symbolic"
        } else {
            "dialog-information-symbolic"
        }));
        // Rebuild only when the list length changed; otherwise update in place.
        if w.rows.len() != self.jobs.len() {
            for r in w.rows.drain(..) {
                w.list.remove(&r);
            }
            w.bars.clear();
            w.reveal.clear();
            for (i, j) in self.jobs.iter().enumerate() {
                let row = adw::ActionRow::builder()
                    .title(j.input.file_name().unwrap_or_default().to_string_lossy())
                    .build();
                let bar = gtk::ProgressBar::builder()
                    .valign(gtk::Align::Center)
                    .width_request(120)
                    .build();
                row.add_suffix(&bar);
                let reveal = gtk::Button::from_icon_name("folder-open-symbolic");
                reveal.set_tooltip_text(Some(&tr("Show in folder")));
                reveal.set_valign(gtk::Align::Center);
                reveal.add_css_class("flat");
                let sender = _sender.clone();
                reveal.connect_clicked(move |_| sender.input(MediaMsg::Reveal(i)));
                row.add_suffix(&reveal);
                w.list.append(&row);
                w.rows.push(row);
                w.bars.push(bar);
                w.reveal.push(reveal);
            }
        }
        w.list.set_visible(has_jobs);
        for (j, reveal) in self.jobs.iter().zip(&w.reveal) {
            reveal.set_visible(matches!(j.status, Status::Done(_)));
        }
        for ((j, row), bar) in self.jobs.iter().zip(&w.rows).zip(&w.bars) {
            match &j.status {
                Status::Waiting => {
                    row.set_subtitle(&tr("Waiting"));
                    bar.set_fraction(0.0);
                }
                Status::Running(Some(f)) => {
                    row.set_subtitle(&format!("{:.0}%", f * 100.0));
                    bar.set_fraction(*f);
                }
                Status::Running(None) => {
                    row.set_subtitle(&tr("Converting…"));
                    bar.pulse();
                }
                Status::Done(p) => {
                    row.set_subtitle(&format!(
                        "{} {}",
                        tr("Saved as"),
                        p.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    bar.set_fraction(1.0);
                }
                Status::Failed(e) => {
                    row.set_subtitle(&format!("{} {e}", tr("Failed:")));
                    bar.set_fraction(0.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_about_the_presets_that_grow_files() {
        let growing: Vec<Choice> = CHOICES.into_iter().filter(|c| size_note(*c).1).collect();
        assert_eq!(
            growing,
            [
                Choice::Import(ImportPreset::DnxhrHq),
                Choice::Import(ImportPreset::Prores422),
                Choice::Import(ImportPreset::Mpeg4 { short_side: None }),
                Choice::Import(ImportPreset::Mpeg4 {
                    short_side: Some(1080)
                }),
                Choice::Import(ImportPreset::Mpeg4 {
                    short_side: Some(720)
                }),
            ]
        );
        for c in CHOICES {
            assert!(!size_note(c).0.is_empty(), "{c:?} needs a size note");
        }
    }
}
