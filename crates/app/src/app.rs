//! Main window: system check → choose installer → (host setup) → progress → done/error.

use crate::i18n::tr;
use crate::media::Media;
use crate::messages;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gio;
use relm4::prelude::*;
use rli_core::distro::Distro;
use rli_core::flow::{self, Outcome};
use rli_core::gpu::Gpu;
use rli_core::hardware::{self, Report};
use rli_core::health::{self, Problem, Repair};
use rli_core::host::{HostRunner, Runner};
use rli_core::installer::Event;
use rli_core::installer_file::{self, parse_filename, InstallerFile};
use rli_core::recipe::Recipe;
use rli_core::remote::{self, UpdateInfo};
use rli_core::state::{self, State, ToolSource};
use rli_core::tools::{self, Action};
use rli_core::{diagnostics, integrate, launcher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Check,
    Source,
    Progress,
    Done,
    Failed,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Source => "source",
            Self::Progress => "progress",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

pub struct App {
    page: Page,
    report: Report,
    gpus: Vec<Gpu>,
    distro: Option<Distro>,
    installers: Vec<InstallerFile>,
    watching: Option<Arc<AtomicBool>>,
    progress: (usize, usize, String),
    outcome: Option<Outcome>,
    error: Option<(String, String)>,
    toasts: adw::ToastOverlay,
    _media: Controller<Media>,
    installed: Option<rli_core::state::Installation>,
    update: Option<UpdateInfo>,
    problems: Vec<Problem>,
}

#[derive(Debug)]
pub enum Msg {
    Recheck,
    Proceed,
    ChooseFile,
    FileChosen(PathBuf),
    OpenDownloadPage,
    Install(usize),
    StartInstall(InstallerFile),
    OpenResolve,
    AskUninstall,
    RemoveContainer(String),
    Repair(Repair),
    RunHealth,
    Uninstall {
        delete_projects: bool,
        tools: Vec<String>,
    },
    CopyReport,
    Back,
}

#[derive(Debug)]
pub enum Cmd {
    Downloads(Vec<InstallerFile>),
    Progress(Event),
    HostReady,
    Finished(Result<Outcome, (String, String)>),
    Update(UpdateInfo),
    Uninstalled(rli_core::uninstall::Summary),
    Health(Vec<Problem>),
}

pub struct Widgets {
    installed_group: adw::PreferencesGroup,
    installed_row: adw::ActionRow,
    problem_rows: Vec<adw::ActionRow>,
    banner: adw::Banner,
    stack: gtk::Stack,
    status: adw::StatusPage,
    check_group: adw::PreferencesGroup,
    check_rows: Vec<adw::ActionRow>,
    proceed: gtk::Button,
    source_group: adw::PreferencesGroup,
    source_rows: Vec<adw::ActionRow>,
    watch_label: gtk::Label,
    progress_bar: gtk::ProgressBar,
    progress_label: gtk::Label,
    done_page: adw::StatusPage,
    failed_page: adw::StatusPage,
    failed_details: gtk::Label,
}

fn probe() -> (Report, Vec<Gpu>, Option<Distro>) {
    let sys = hardware::probe(&state::home());
    let gpus = sys.gpus.clone();
    (hardware::check(&sys), gpus, Distro::detect().ok())
}

fn downloads_dir() -> PathBuf {
    gtk::glib::user_special_dir(gtk::glib::UserDirectory::Downloads)
        .unwrap_or_else(|| state::home().join("Downloads"))
}

fn button(label: &str, classes: &[&str]) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    for c in classes {
        b.add_css_class(c);
    }
    b
}

fn page_box(children: &[&gtk::Widget]) -> gtk::ScrolledWindow {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for c in children {
        content.append(*c);
    }
    let clamp = adw::Clamp::builder()
        .maximum_size(640)
        .margin_start(16)
        .margin_end(16)
        .margin_bottom(24)
        .child(&content)
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build()
}

fn button_row(buttons: &[&gtk::Button]) -> gtk::Box {
    let b = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::Center)
        .margin_top(24)
        .build();
    for x in buttons {
        b.append(*x);
    }
    b
}

/// Stops the background watcher of the Downloads folder, if running.
fn stop_watching(w: &mut Option<Arc<AtomicBool>>) {
    if let Some(flag) = w.take() {
        flag.store(true, Ordering::Relaxed);
    }
}

impl App {
    fn chosen_gpu(&self) -> Option<&Gpu> {
        self.report.chosen_gpu.as_ref()
    }

    fn refresh_installers(&mut self) {
        let mut found = installer_file::ready_in_dir(&downloads_dir());
        found.extend(installer_file::ready_in_dir(&flow::installers_dir(
            &state::home(),
        )));
        found.dedup_by(|a, b| a.path.file_name() == b.path.file_name());
        self.installers = found;
    }

    /// Polls ~/Downloads until a complete installer appears with a stable size.
    fn watch_downloads(&mut self, sender: &ComponentSender<Self>) {
        if self.watching.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.watching = Some(stop.clone());
        sender.spawn_command(move |out| {
            let dir = downloads_dir();
            let mut last: Vec<(PathBuf, u64)> = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let now: Vec<(PathBuf, u64)> = installer_file::ready_in_dir(&dir)
                    .into_iter()
                    .map(|f| {
                        let size = std::fs::metadata(&f.path).map_or(0, |m| m.len());
                        (f.path, size)
                    })
                    .collect();
                if !now.is_empty() && now == last {
                    let _ = out.send(Cmd::Downloads(installer_file::ready_in_dir(&dir)));
                    return;
                }
                last = now;
                std::thread::sleep(Duration::from_secs(3));
            }
        });
    }
}

impl Component for App {
    type CommandOutput = Cmd;
    type Init = ();
    type Input = Msg;
    type Output = ();
    type Root = adw::ApplicationWindow;
    type Widgets = Widgets;

    fn init_root() -> Self::Root {
        adw::ApplicationWindow::builder()
            .title("Vinci")
            .default_width(720)
            .default_height(680)
            .build()
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let (report, gpus, distro) = probe();
        let toasts = adw::ToastOverlay::new();
        let media = Media::builder().launch(report.chosen_gpu.clone()).detach();
        let media_root = media.widget().clone();
        let model = App {
            page: Page::Check,
            report,
            gpus,
            distro,
            installers: Vec::new(),
            watching: None,
            progress: (0, 1, String::new()),
            outcome: None,
            error: None,
            toasts: toasts.clone(),
            _media: media,
            installed: State::load(&State::default_path())
                .ok()
                .and_then(|s| s.installation),
            update: None,
            problems: Vec::new(),
        };
        let input = |m: fn() -> Msg| {
            let sender = sender.clone();
            move |_: &gtk::Button| sender.input(m())
        };

        // Check page
        let status = adw::StatusPage::builder()
            .icon_name("computer-symbolic")
            .title(tr("System check"))
            .build();
        let check_group = adw::PreferencesGroup::new();
        let installed_group = adw::PreferencesGroup::builder().margin_bottom(24).build();
        let installed_row = adw::ActionRow::new();
        installed_row.add_prefix(&gtk::Image::from_icon_name("object-select-symbolic"));
        let open_installed = button(&tr("Open"), &["suggested-action"]);
        open_installed.set_valign(gtk::Align::Center);
        open_installed.connect_clicked(input(|| Msg::OpenResolve));
        let uninstall = button(&tr("Uninstall…"), &["destructive-action"]);
        uninstall.set_valign(gtk::Align::Center);
        uninstall.connect_clicked(input(|| Msg::AskUninstall));
        installed_row.add_suffix(&uninstall);
        installed_row.add_suffix(&open_installed);
        installed_group.add(&installed_row);
        let recheck = button(&tr("Check again"), &["pill"]);
        recheck.connect_clicked(input(|| Msg::Recheck));
        let proceed = button(&tr("Continue"), &["suggested-action", "pill"]);
        proceed.connect_clicked(input(|| Msg::Proceed));
        let report_check = button(&tr("Copy diagnostic report"), &["flat"]);
        report_check.connect_clicked(input(|| Msg::CopyReport));
        let check_page = page_box(&[
            status.upcast_ref(),
            installed_group.upcast_ref(),
            check_group.upcast_ref(),
            button_row(&[&recheck, &proceed]).upcast_ref(),
            button_row(&[&report_check]).upcast_ref(),
        ]);

        // Source page
        let source_status = adw::StatusPage::builder()
            .icon_name("folder-download-symbolic")
            .title(tr("Choose the installer"))
            .description(tr("Download DaVinci Resolve for Linux from Blackmagic Design's website. Vinci finds the file in your Downloads folder automatically."))
            .build();
        let source_group = adw::PreferencesGroup::new();
        let watch_label = gtk::Label::builder()
            .wrap(true)
            .margin_top(12)
            .css_classes(["dim-label"])
            .build();
        let download = button(&tr("Open download page"), &["suggested-action", "pill"]);
        download.connect_clicked(input(|| Msg::OpenDownloadPage));
        let choose = button(&tr("Choose file…"), &["pill"]);
        choose.connect_clicked(input(|| Msg::ChooseFile));
        let back = button(&tr("Back"), &["pill"]);
        back.connect_clicked(input(|| Msg::Back));
        let source_page = page_box(&[
            source_status.upcast_ref(),
            source_group.upcast_ref(),
            watch_label.upcast_ref(),
            button_row(&[&back, &choose, &download]).upcast_ref(),
        ]);

        // Progress page
        let progress_bar = gtk::ProgressBar::builder()
            .show_text(false)
            .margin_top(12)
            .build();
        let progress_label = gtk::Label::builder().margin_top(12).wrap(true).build();
        let progress_page = page_box(&[
            adw::StatusPage::builder()
                .icon_name("system-software-install-symbolic")
                .title(tr("Installing DaVinci Resolve"))
                .description(tr(
                    "This takes a few minutes. You can keep using your computer.",
                ))
                .build()
                .upcast_ref(),
            progress_bar.upcast_ref(),
            progress_label.upcast_ref(),
        ]);

        // Done page
        let done_page = adw::StatusPage::builder()
            .icon_name("object-select-symbolic")
            .title(tr("DaVinci Resolve is installed"))
            .description(tr(
                "It is in your applications menu. Your projects stay in your home folder.",
            ))
            .build();
        let open = button(&tr("Open DaVinci Resolve"), &["suggested-action", "pill"]);
        open.connect_clicked(input(|| Msg::OpenResolve));
        let done_box = page_box(&[done_page.upcast_ref(), button_row(&[&open]).upcast_ref()]);

        // Failed page
        let failed_page = adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .title(tr("Installation failed"))
            .build();
        let failed_details = gtk::Label::builder()
            .wrap(true)
            .selectable(true)
            .xalign(0.0)
            .css_classes(["monospace", "dim-label"])
            .build();
        let retry = button(&tr("Try again"), &["pill"]);
        retry.connect_clicked(input(|| Msg::Proceed));
        let report_failed = button(&tr("Copy diagnostic report"), &["pill"]);
        report_failed.connect_clicked(input(|| Msg::CopyReport));
        let failed_box = page_box(&[
            failed_page.upcast_ref(),
            failed_details.upcast_ref(),
            button_row(&[&report_failed, &retry]).upcast_ref(),
        ]);

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::SlideLeftRight)
            .build();
        stack.add_named(&check_page, Some(Page::Check.name()));
        stack.add_named(&source_page, Some(Page::Source.name()));
        stack.add_named(&progress_page, Some(Page::Progress.name()));
        stack.add_named(&done_box, Some(Page::Done.name()));
        stack.add_named(&failed_box, Some(Page::Failed.name()));

        let banner = adw::Banner::builder().button_label(tr("Update")).build();
        banner.connect_button_clicked({
            let sender = sender.clone();
            move |_| sender.input(Msg::Proceed)
        });
        let views = adw::ViewStack::new();
        let install_page = views.add_titled(&stack, Some("install"), &tr("Installation"));
        install_page.set_icon_name(Some("system-software-install-symbolic"));
        let media_page = views.add_titled(&media_root, Some("media"), &tr("Media"));
        media_page.set_icon_name(Some("video-x-generic-symbolic"));
        let switcher = adw::ViewSwitcher::builder()
            .stack(&views)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&switcher));
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&banner);
        toasts.set_child(Some(&views));
        toolbar.set_content(Some(&toasts));
        root.set_content(Some(&toolbar));

        if let Some(inst) = model.installed.clone() {
            // Offline-first: a failed check just shows nothing.
            sender.spawn_command(move |out| {
                let info = remote::check_update(
                    &remote::Http::new(),
                    &Recipe::bundled(),
                    inst.edition,
                    Some(&inst.version),
                );
                let _ = out.send(Cmd::Update(info));
            });
        }
        sender.input(Msg::RunHealth);
        let mut widgets = Widgets {
            installed_group,
            installed_row,
            problem_rows: Vec::new(),
            banner,
            stack,
            status,
            check_group,
            check_rows: Vec::new(),
            proceed,
            source_group,
            source_rows: Vec::new(),
            watch_label,
            progress_bar,
            progress_label,
            done_page,
            failed_page,
            failed_details,
        };
        Component::update_view(&model, &mut widgets, sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Msg, sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            Msg::Recheck => (self.report, self.gpus, self.distro) = probe(),
            Msg::Proceed => {
                self.refresh_installers();
                self.page = Page::Source;
            }
            Msg::Back => {
                stop_watching(&mut self.watching);
                self.page = Page::Check;
            }
            Msg::OpenDownloadPage => {
                let url = Recipe::bundled().blackmagic.download_page;
                gtk::UriLauncher::new(&url).launch(Some(root), None::<&gio::Cancellable>, |_| {});
                self.watch_downloads(&sender);
            }
            Msg::ChooseFile => {
                let filter = gtk::FileFilter::new();
                filter.add_pattern("DaVinci_Resolve_*_Linux.zip");
                filter.add_pattern("DaVinci_Resolve_*_Linux.run");
                filter.set_name(Some(&tr("DaVinci Resolve installer")));
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                let dialog = gtk::FileDialog::builder()
                    .title(tr("Choose the installer"))
                    .filters(&filters)
                    .build();
                let sender = sender.clone();
                dialog.open(Some(root), None::<&gio::Cancellable>, move |res| {
                    if let Some(path) = res.ok().and_then(|f| f.path()) {
                        sender.input(Msg::FileChosen(path));
                    }
                });
            }
            Msg::FileChosen(path) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if let Some((edition, version, kind)) = parse_filename(&name) {
                    self.installers.insert(
                        0,
                        InstallerFile {
                            path,
                            edition,
                            version,
                            kind,
                        },
                    );
                }
            }
            Msg::Install(i) => {
                if let Some(file) = self.installers.get(i).cloned() {
                    stop_watching(&mut self.watching);
                    self.confirm_host_setup(file, &sender, root);
                }
            }
            Msg::StartInstall(file) => self.start_install(file, &sender),
            Msg::AskUninstall => self.ask_uninstall(&sender, root),
            Msg::RemoveContainer(name) => {
                sender.spawn_command(move |_| {
                    let _ = HostRunner::new().run(&[
                        "podman".into(),
                        "rm".into(),
                        "--force".into(),
                        "--ignore".into(),
                        name,
                    ]);
                });
            }
            Msg::RunHealth => {
                let (Some(inst), Some(gpu)) = (self.installed.clone(), self.chosen_gpu().cloned())
                else {
                    self.problems.clear();
                    return;
                };
                sender.spawn_command(move |out| {
                    let runner = HostRunner::new();
                    let home = state::home();
                    let recipe = Recipe::bundled();
                    let tools = tools::detect(&runner, &recipe, &home);
                    let problems = match tools.distrobox.executable() {
                        Some(dbx) => health::check(
                            &runner,
                            &recipe,
                            &dbx,
                            &inst,
                            Some(gpu.vendor),
                            &integrate::data_home(&home),
                        ),
                        None => vec![Problem::ContainerMissing],
                    };
                    let _ = out.send(Cmd::Health(problems));
                });
            }
            Msg::Repair(Repair::Reinstall) => sender.input(Msg::Proceed),
            Msg::Repair(Repair::Explain) => {}
            Msg::Repair(Repair::RecreateLauncher) => {
                let (Some(inst), Some(gpu)) = (self.installed.clone(), self.chosen_gpu().cloned())
                else {
                    return;
                };
                let gpus = self.gpus.clone();
                let input = sender.input_sender().clone();
                sender.spawn_command(move |_out| {
                    let runner = HostRunner::new();
                    let home = state::home();
                    let recipe = Recipe::bundled();
                    if let Some(dbx) = tools::detect(&runner, &recipe, &home)
                        .distrobox
                        .executable()
                    {
                        let _ = flow::write_launcher(
                            &runner,
                            &recipe,
                            &dbx,
                            &inst,
                            &gpus,
                            &gpu,
                            &integrate::data_home(&home),
                        );
                    }
                    input.emit(Msg::RunHealth);
                });
            }
            Msg::Uninstall {
                delete_projects,
                tools,
            } => {
                let distro = self.distro.clone().unwrap_or_else(|| Distro::parse(""));
                sender.spawn_command(move |out| {
                    let runner = HostRunner::new();
                    let home = state::home();
                    let summary = rli_core::uninstall::uninstall(
                        &runner,
                        &|argv: &[String]| runner.run_root(argv).is_ok_and(|o| o.success),
                        &Recipe::bundled(),
                        &distro,
                        &rli_core::uninstall::Locations {
                            home: &home,
                            data_home: &integrate::data_home(&home),
                            state_path: &State::default_path(),
                        },
                        &rli_core::uninstall::Options {
                            delete_projects,
                            remove_tools: tools,
                        },
                    );
                    let _ = out.send(Cmd::Uninstalled(summary));
                });
            }
            Msg::CopyReport => {
                root.clipboard().set_text(&self.diagnostic_report());
                self.toasts.add_toast(adw::Toast::new(&tr(
                    "Diagnostic report copied. Paste it in your issue or message.",
                )));
            }
            Msg::OpenResolve => {
                let container = self
                    .outcome
                    .as_ref()
                    .map(|o| o.installation.container.clone())
                    .or_else(|| self.installed.as_ref().map(|i| i.container.clone()));
                if let (Some(container), Some(g)) = (container, self.chosen_gpu()) {
                    let recipe = Recipe::bundled();
                    let runner = HostRunner::new();
                    let tools = tools::detect(&runner, &recipe, &state::home());
                    if let Some(dbx) = tools.distrobox.executable() {
                        let env = launcher::environment(&recipe, &self.gpus, g);
                        let argv = launcher::command(&recipe, &dbx, &container, &env);
                        let _ = runner.spawn_detached(&argv);
                    }
                }
            }
        }
    }

    fn update_cmd(&mut self, cmd: Cmd, sender: ComponentSender<Self>, root: &Self::Root) {
        match cmd {
            Cmd::Downloads(found) => {
                self.installers = found;
                self.watching = None;
            }
            Cmd::Progress(Event::Started { index, total, kind }) => {
                self.progress = (index, total, messages::step(kind));
            }
            Cmd::Progress(Event::TryingMirror { image }) => {
                self.progress.2 = format!("{} {image}", tr("Trying mirror"));
            }
            Cmd::Progress(Event::RecreatingContainer) => {
                self.progress.2 = tr("Recreating the container and trying again");
            }
            Cmd::Progress(Event::Finished { .. }) => self.progress.0 += 1,
            Cmd::HostReady => {}
            Cmd::Update(info) => self.update = Some(info),
            Cmd::Health(problems) => self.problems = problems,
            Cmd::Uninstalled(summary) => {
                self.installed = None;
                self.outcome = None;
                self.update = None;
                let text = if summary.warnings.is_empty() {
                    tr("DaVinci Resolve was uninstalled.")
                } else {
                    format!(
                        "{} {}",
                        tr("Uninstalled, with warnings:"),
                        summary.warnings.join(", ")
                    )
                };
                self.toasts.add_toast(adw::Toast::new(&text));
            }
            Cmd::Finished(Ok(outcome)) => {
                if let Some(old) = &outcome.replaced {
                    self.offer_removing(old.clone(), &sender, root);
                }
                self.installed = Some(outcome.installation.clone());
                self.outcome = Some(outcome);
                self.page = Page::Done;
            }
            Cmd::Finished(Err(e)) => {
                self.error = Some(e);
                self.page = Page::Failed;
            }
        }
    }

    fn update_view(&self, w: &mut Widgets, _sender: ComponentSender<Self>) {
        w.stack.set_visible_child_name(self.page.name());
        match &self.update {
            Some(UpdateInfo {
                latest: Some(v),
                update_available: true,
                latest_is_bad: None,
            }) => {
                w.banner.set_title(
                    &tr("DaVinci Resolve {version} is available")
                        .replace("{version}", &v.to_string()),
                );
                w.banner.set_button_label(Some(&tr("Update")));
                w.banner.set_revealed(self.page == Page::Check);
            }
            Some(UpdateInfo {
                latest: Some(v),
                update_available: true,
                latest_is_bad: Some(bad),
            }) => {
                w.banner.set_title(&format!(
                    "{} ({})",
                    tr("Version {version} has known problems. Keep your current version for now.")
                        .replace("{version}", &v.to_string()),
                    bad.reason
                ));
                w.banner.set_button_label(None);
                w.banner.set_revealed(self.page == Page::Check);
            }
            _ => w.banner.set_revealed(false),
        }

        let distro = self
            .distro
            .as_ref()
            .map_or_else(|| tr("Unknown distribution"), |d| d.pretty_name.clone());
        let gpu = self
            .chosen_gpu()
            .map_or_else(|| tr("no compatible GPU"), messages::gpu_label);
        w.status.set_description(Some(&format!("{distro} · {gpu}")));
        w.installed_group.set_visible(self.installed.is_some());
        if let Some(i) = &self.installed {
            w.installed_row.set_title(&format!(
                "{} {}",
                messages::edition_name(i.edition),
                i.version
            ));
            w.installed_row.set_subtitle(&tr("Installed"));
        }
        for row in w.problem_rows.drain(..) {
            w.installed_group.remove(&row);
        }
        for p in &self.problems {
            let (title, subtitle) = messages::problem(p);
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(subtitle)
                .subtitle_lines(0)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
            let repair = p.repair();
            if repair != Repair::Explain {
                let b = button(&tr("Repair"), &[]);
                b.set_valign(gtk::Align::Center);
                let sender = _sender.clone();
                b.connect_clicked(move |_| sender.input(Msg::Repair(repair)));
                row.add_suffix(&b);
            }
            w.installed_group.add(&row);
            w.problem_rows.push(row);
        }
        for row in w.check_rows.drain(..) {
            w.check_group.remove(&row);
        }
        if self.report.findings.is_empty() {
            let row = adw::ActionRow::builder()
                .title(tr("Everything looks good"))
                .subtitle(tr(
                    "This computer meets the requirements for DaVinci Resolve.",
                ))
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("object-select-symbolic"));
            w.check_group.add(&row);
            w.check_rows.push(row);
        }
        for f in &self.report.findings {
            let (title, subtitle) = messages::finding(f);
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(subtitle)
                .subtitle_lines(0)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name(messages::severity_icon(
                f.severity(),
            )));
            w.check_group.add(&row);
            w.check_rows.push(row);
        }
        w.proceed.set_sensitive(!self.report.is_blocked());

        for row in w.source_rows.drain(..) {
            w.source_group.remove(&row);
        }
        for (i, f) in self.installers.iter().enumerate() {
            let row = adw::ActionRow::builder()
                .title(messages::installer_title(f))
                .subtitle(f.path.display().to_string())
                .activatable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("package-x-generic-symbolic"));
            let install = button(&tr("Install"), &["suggested-action"]);
            install.set_valign(gtk::Align::Center);
            let sender = _sender.clone();
            install.connect_clicked(move |_| sender.input(Msg::Install(i)));
            row.add_suffix(&install);
            w.source_group.add(&row);
            w.source_rows.push(row);
        }
        w.watch_label.set_label(&if self.watching.is_some() {
            tr("Waiting for the download to finish in your Downloads folder…")
        } else if self.installers.is_empty() {
            tr("No installer found in your Downloads folder yet.")
        } else {
            String::new()
        });

        let (done, total, label) = &self.progress;
        w.progress_bar
            .set_fraction(*done as f64 / (*total).max(1) as f64);
        w.progress_label.set_label(label);

        if let Some(o) = &self.outcome {
            w.done_page.set_title(&format!(
                "{} {}",
                messages::edition_name(o.installation.edition),
                o.installation.version
            ));
        }
        if let Some((summary, details)) = &self.error {
            w.failed_page.set_description(Some(summary));
            w.failed_details.set_label(details);
        }
    }
}

impl App {
    fn diagnostic_report(&self) -> String {
        let recipe = Recipe::bundled();
        let home = state::home();
        let tools = tools::detect(&HostRunner::new(), &recipe, &home);
        let st = State::load(&State::default_path()).unwrap_or_default();
        diagnostics::report(&diagnostics::Input {
            app_version: env!("CARGO_PKG_VERSION"),
            recipe_revision: recipe.revision,
            distro: self.distro.as_ref(),
            gpus: &self.gpus,
            report: &self.report,
            ram_bytes: hardware::read_ram(),
            free_disk_bytes: hardware::free_disk(&home),
            tools: Some(&tools),
            state: &st,
            error: self.error.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
            home: &home,
        })
    }

    /// After switching edition (Free ↔ Studio), offers to remove the old container, ticked
    /// by default (SPEC §7). Projects live in the home folder and are not affected.
    fn offer_removing(
        &self,
        old: rli_core::state::Installation,
        sender: &ComponentSender<Self>,
        root: &adw::ApplicationWindow,
    ) {
        let dialog = adw::AlertDialog::new(
            Some(&tr("Remove the previous installation?")),
            Some(&tr("{name} is still installed in its own container. Removing it frees disk space; your projects are not affected.")
                .replace("{name}", &format!("{} {}", messages::edition_name(old.edition), old.version))),
        );
        dialog.add_response("keep", &tr("Keep"));
        dialog.add_response("remove", &tr("Remove"));
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("remove"));
        let sender = sender.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "remove" {
                sender.input(Msg::RemoveContainer(old.container.clone()));
            }
        });
        dialog.present(Some(root));
    }

    /// Confirms the uninstall. Deleting projects is opt-in (unticked) per SPEC §7, and
    /// only tools Vinci itself installed are offered.
    fn ask_uninstall(&self, sender: &ComponentSender<Self>, root: &adw::ApplicationWindow) {
        let recipe = Recipe::bundled();
        let runner = HostRunner::new();
        let st = State::load(&State::default_path()).unwrap_or_default();
        let tools = rli_core::uninstall::removable_tools(&st, &runner, &recipe);

        let boxed = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let projects =
            gtk::CheckButton::with_label(&tr("Also delete projects and the Resolve database"));
        boxed.append(&projects);
        let tool_checks: Vec<(String, gtk::CheckButton)> = tools
            .iter()
            .map(|t| {
                let c = gtk::CheckButton::with_label(&format!("{} {}", tr("Also remove"), t.name));
                boxed.append(&c);
                (t.name.clone(), c)
            })
            .collect();
        let dialog = adw::AlertDialog::new(
            Some(&tr("Uninstall DaVinci Resolve?")),
            Some(&tr("The container and menu entry are removed. Your projects stay unless you tick the option below.")),
        );
        dialog.set_extra_child(Some(&boxed));
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("uninstall", &tr("Uninstall"));
        dialog.set_response_appearance("uninstall", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        let sender = sender.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "uninstall" {
                sender.input(Msg::Uninstall {
                    delete_projects: projects.is_active(),
                    tools: tool_checks
                        .iter()
                        .filter(|(_, c)| c.is_active())
                        .map(|(n, _)| n.clone())
                        .collect(),
                });
            }
        });
        dialog.present(Some(root));
    }

    /// Shows what will be installed on the host and asks first (SPEC §3); skips straight
    /// to the install when nothing is missing.
    fn confirm_host_setup(
        &mut self,
        file: InstallerFile,
        sender: &ComponentSender<Self>,
        root: &adw::ApplicationWindow,
    ) {
        let recipe = Recipe::bundled();
        let home = state::home();
        let runner = HostRunner::new();
        let host = tools::detect(&runner, &recipe, &home);
        let distro = self.distro.clone().unwrap_or_else(|| Distro::parse(""));
        let actions = tools::plan(&host, &distro, &recipe, &home);
        if actions.is_empty() {
            sender.input(Msg::StartInstall(file));
            return;
        }
        if let Some(Action::ManualPodman { .. }) = actions.first() {
            self.error = Some((
                tr("Podman is required"),
                tr("Install podman with your distribution's package manager, then open Vinci again."),
            ));
            self.page = Page::Failed;
            return;
        }
        let items: Vec<String> = actions.iter().map(messages::host_action).collect();
        let dialog = adw::AlertDialog::new(
            Some(&tr("Vinci needs to install some tools")),
            Some(&format!(
                "{}\n\n• {}",
                tr("These tools run DaVinci Resolve in an isolated container:"),
                items.join("\n• ")
            )),
        );
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("install", &tr("Install"));
        dialog.set_response_appearance("install", adw::ResponseAppearance::Suggested);
        let sender = sender.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "install" {
                return;
            }
            let actions = actions.clone();
            let file = file.clone();
            let input = sender.input_sender().clone();
            sender.spawn_command(move |out| match run_host_actions(&actions) {
                Ok(()) => {
                    let _ = out.send(Cmd::HostReady);
                    input.emit(Msg::StartInstall(file));
                }
                Err(e) => {
                    let _ = out.send(Cmd::Finished(Err(e)));
                }
            });
        });
        dialog.present(Some(root));
    }

    fn start_install(&mut self, file: InstallerFile, sender: &ComponentSender<Self>) {
        let Some(gpu) = self.chosen_gpu().cloned() else {
            return;
        };
        self.page = Page::Progress;
        self.progress = (0, 1, tr("Preparing"));
        let gpus = self.gpus.clone();
        sender.spawn_command(move |out| {
            let result = install_blocking(&file, &gpus, &gpu, &out);
            let _ = out.send(Cmd::Finished(result));
        });
    }
}

fn run_host_actions(actions: &[Action]) -> Result<(), (String, String)> {
    let runner = HostRunner::new();
    let path = State::default_path();
    let mut st = State::load(&path).unwrap_or_default();
    for a in actions {
        let (out, tool, source) = match a {
            Action::InstallPodman { argv, .. } => {
                (runner.run_root(argv), "podman", ToolSource::PackageManager)
            }
            Action::InstallDistrobox { argv, prefix } => (
                runner.run(argv),
                "distrobox",
                ToolSource::UserLocal {
                    prefix: prefix.clone(),
                },
            ),
            Action::ManualPodman { .. } => continue,
        };
        let failed = |detail: String| Err((format!("{} {tool}", tr("Could not install")), detail));
        match out {
            Ok(o) if o.success => st.record_tool(tool, source),
            Ok(o) => return failed(o.stderr),
            Err(e) => return failed(e.to_string()),
        }
    }
    let _ = st.save(&path);
    Ok(())
}

fn install_blocking(
    file: &InstallerFile,
    gpus: &[Gpu],
    gpu: &Gpu,
    out: &relm4::Sender<Cmd>,
) -> Result<Outcome, (String, String)> {
    let recipe = Recipe::bundled();
    let runner = HostRunner::new();
    let home = state::home();
    let host = tools::detect(&runner, &recipe, &home);
    let distrobox = host
        .distrobox
        .executable()
        .ok_or_else(|| (tr("distrobox is missing"), String::new()))?;
    let req = flow::Request {
        installer: file,
        gpus,
        gpu,
        distrobox: &distrobox,
        home: &home,
        data_home: &integrate::data_home(&home),
        state_path: &State::default_path(),
        keep_installer: true,
    };
    flow::install_resolve(&runner, &recipe, &req, &mut |e| {
        let _ = out.send(Cmd::Progress(e));
    })
    .map_err(|e| {
        let details = match &e {
            flow::Error::Step(rli_core::installer::Error::StepFailed { output, .. }) => {
                let tail: Vec<&str> = output.stderr.lines().rev().take(20).collect();
                tail.into_iter().rev().collect::<Vec<_>>().join("\n")
            }
            other => other.to_string(),
        };
        (messages::failure(&e), details)
    })
}
