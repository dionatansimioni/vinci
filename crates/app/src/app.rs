//! Main window. For now: the system check screen that gates the installer.

use crate::i18n::tr;
use crate::messages;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;
use rli_core::distro::Distro;
use rli_core::hardware::{self, Report};

pub struct App {
    report: Report,
    distro: Option<Distro>,
}

#[derive(Debug)]
pub enum Msg {
    Recheck,
}

pub struct Widgets {
    status: adw::StatusPage,
    group: adw::PreferencesGroup,
    rows: Vec<adw::ActionRow>,
    proceed: gtk::Button,
}

fn probe() -> (Report, Option<Distro>) {
    let sys = hardware::probe(&rli_core::state::home());
    (hardware::check(&sys), Distro::detect().ok())
}

impl SimpleComponent for App {
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
        let (report, distro) = probe();
        let model = App { report, distro };

        let status = adw::StatusPage::builder()
            .icon_name("computer-symbolic")
            .title(tr("System check"))
            .build();
        let group = adw::PreferencesGroup::new();

        let recheck = gtk::Button::with_label(&tr("Check again"));
        recheck.connect_clicked({
            let sender = sender.clone();
            move |_| sender.input(Msg::Recheck)
        });
        let proceed = gtk::Button::builder()
            .label(tr("Continue"))
            .css_classes(["suggested-action", "pill"])
            .build();
        recheck.add_css_class("pill");
        let buttons = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .margin_top(24)
            .build();
        buttons.append(&recheck);
        buttons.append(&proceed);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&status);
        content.append(&group);
        content.append(&buttons);
        let clamp = adw::Clamp::builder()
            .maximum_size(640)
            .margin_start(16)
            .margin_end(16)
            .margin_bottom(24)
            .child(&content)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&scroll));
        root.set_content(Some(&toolbar));

        let mut widgets = Widgets {
            status,
            group,
            rows: Vec::new(),
            proceed,
        };
        SimpleComponent::update_view(&model, &mut widgets, sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Msg, _sender: ComponentSender<Self>) {
        match msg {
            Msg::Recheck => (self.report, self.distro) = probe(),
        }
    }

    fn update_view(&self, widgets: &mut Widgets, _sender: ComponentSender<Self>) {
        let distro = self
            .distro
            .as_ref()
            .map_or_else(|| tr("Unknown distribution"), |d| d.pretty_name.clone());
        let gpu = self
            .report
            .chosen_gpu
            .as_ref()
            .map_or_else(|| tr("no compatible GPU"), messages::gpu_label);
        widgets
            .status
            .set_description(Some(&format!("{distro} · {gpu}")));

        for row in widgets.rows.drain(..) {
            widgets.group.remove(&row);
        }
        if self.report.findings.is_empty() {
            let row = adw::ActionRow::builder()
                .title(tr("Everything looks good"))
                .subtitle(tr(
                    "This computer meets the requirements for DaVinci Resolve.",
                ))
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("emblem-ok-symbolic"));
            widgets.group.add(&row);
            widgets.rows.push(row);
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
            widgets.group.add(&row);
            widgets.rows.push(row);
        }
        widgets.proceed.set_sensitive(!self.report.is_blocked());
    }
}
