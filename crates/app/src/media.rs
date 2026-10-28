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
use rli_core::host::{HostRunner, Runner};
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

pub const CHOICES: [Choice; 5] = [
    Choice::Import(ImportPreset::DnxhrHq),
    Choice::Import(ImportPreset::Prores422),
    Choice::Import(ImportPreset::AudioOnly),
    Choice::Export(ExportPreset::Web),
    Choice::Export(ExportPreset::Archive),
];

pub fn choice_label(c: Choice) -> String {
    match c {
        Choice::Import(ImportPreset::DnxhrHq) => {
            tr("For editing: DNxHR HQ (works in every edition)")
        }
        Choice::Import(ImportPreset::Prores422) => tr("For editing: ProRes 422"),
        Choice::Import(ImportPreset::AudioOnly) => {
            tr("For editing: fix audio only (Studio with NVIDIA)")
        }
        Choice::Export(ExportPreset::Web) => tr("For publishing: MP4 H.264 (YouTube, web)"),
        Choice::Export(ExportPreset::Archive) => tr("For publishing: MP4 H.265 (smaller files)"),
        Choice::Export(ExportPreset::Custom(_)) => tr("Custom"),
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
    start: gtk::Button,
    empty: adw::StatusPage,
    content: gtk::Box,
}

/// The installed container and a distrobox to reach it, if Resolve is installed.
fn engine() -> Option<(Engine, State)> {
    let st = State::load(&State::default_path()).ok()?;
    let inst = st.installation.clone()?;
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

pub fn pick_export_encoder(engine: &Engine, codec: VideoCodec, gpu: Option<&Gpu>) -> Encoder {
    let argv: Vec<String> = ["ffmpeg", "-hide_banner", "-encoders"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let argv = match engine {
        Engine::Container {
            distrobox,
            container,
        } => convert::in_container(distrobox, container, &argv),
        Engine::Direct => argv,
    };
    let available = HostRunner::new()
        .run(&argv)
        .map(|o| ffmpeg::parse_encoders(&o.stdout))
        .unwrap_or_default();
    let node = gpu
        .and_then(|g| g.render_node.as_ref())
        .map(|n| PathBuf::from(format!("/dev/dri/{n}")));
    ffmpeg::pick_encoder(codec, gpu.map(|g| g.vendor), node.as_deref(), &available)
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
            .and_then(|s| s.installation)
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
        let preset_group = adw::PreferencesGroup::new();
        preset_group.add(&preset_row);

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
                    let encoder = match choice {
                        Choice::Export(p) => Some(pick_export_encoder(
                            &engine,
                            p.settings().codec,
                            gpu.as_ref(),
                        )),
                        Choice::Import(_) => None,
                    };
                    for (i, input) in todo {
                        let dir = input.parent().map(PathBuf::from).unwrap_or_default();
                        let command = match (choice, &encoder) {
                            (Choice::Import(p), _) => ffmpeg::import_command(&input, &dir, p),
                            (Choice::Export(p), Some(e)) => {
                                ffmpeg::export_command(&input, &dir, p, e)
                            }
                            (Choice::Export(p), None) => {
                                ffmpeg::export_command(&input, &dir, p, &Encoder::Cpu)
                            }
                        };
                        let result = convert::convert_file(&engine, command, &input, |f| {
                            let _ = out.send(MediaCmd::Progress(i, f));
                        })
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
        // Rebuild only when the list length changed; otherwise update in place.
        if w.rows.len() != self.jobs.len() {
            for r in w.rows.drain(..) {
                w.list.remove(&r);
            }
            w.bars.clear();
            for j in &self.jobs {
                let row = adw::ActionRow::builder()
                    .title(j.input.file_name().unwrap_or_default().to_string_lossy())
                    .build();
                let bar = gtk::ProgressBar::builder()
                    .valign(gtk::Align::Center)
                    .width_request(120)
                    .build();
                row.add_suffix(&bar);
                w.list.append(&row);
                w.rows.push(row);
                w.bars.push(bar);
            }
        }
        w.list.set_visible(has_jobs);
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
