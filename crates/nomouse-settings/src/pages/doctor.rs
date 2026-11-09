//! Doctor: runs `nomouse doctor` and shows each check as a row.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::args::Page;
use crate::doctor::{DoctorLine, parse};
use crate::launch;
use crate::widgets;

/// The Doctor page.
pub struct DoctorPage {
    /// The page.
    pub page: adw::PreferencesPage,
    group: adw::PreferencesGroup,
    /// "Run again".
    pub run_again: gtk::Button,
    rows: RefCell<Vec<adw::ActionRow>>,
    ran: Cell<bool>,
}

impl DoctorPage {
    /// Builds the page (the report is read on [`DoctorPage::run`]).
    pub fn new() -> Rc<Self> {
        let page = widgets::page(Page::Doctor);
        let group = widgets::group(
            "nomouse doctor",
            "What works in this session, and what to fix.",
        );
        let run_again = widgets::button("Run again", "Run again");
        group.set_header_suffix(Some(&run_again));
        page.add(&group);
        let doctor = Rc::new(Self {
            page,
            group,
            run_again,
            rows: RefCell::new(Vec::new()),
            ran: Cell::new(false),
        });
        let weak = Rc::downgrade(&doctor);
        doctor.run_again.connect_clicked(move |_| {
            if let Some(doctor) = weak.upgrade() {
                doctor.run();
            }
        });
        doctor
    }

    /// Runs the doctor unless it ran already.
    pub fn run_once(self: &Rc<Self>) {
        if !self.ran.get() {
            self.run();
        }
    }

    /// Runs `nomouse doctor` and shows its report.
    pub fn run(self: &Rc<Self>) {
        self.ran.set(true);
        self.run_again.set_sensitive(false);
        self.show(&[DoctorLine::Note("Running nomouse doctor…".to_owned())]);
        let program = launch::nomouse_program();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let lines = match gio::Subprocess::newv(
                &[program.as_os_str(), "doctor".as_ref()],
                gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE,
            ) {
                Ok(process) => match process.communicate_utf8_future(None).await {
                    Ok((stdout, _)) => {
                        let lines = parse(stdout.as_deref().unwrap_or_default());
                        if lines.is_empty() {
                            vec![DoctorLine::Note(
                                "nomouse doctor printed nothing".to_owned(),
                            )]
                        } else {
                            lines
                        }
                    }
                    Err(error) => vec![DoctorLine::Note(format!("nomouse doctor failed: {error}"))],
                },
                Err(error) => vec![DoctorLine::Note(format!(
                    "Cannot run {} doctor: {error}",
                    program.display()
                ))],
            };
            if let Some(doctor) = weak.upgrade() {
                doctor.show(&lines);
                doctor.run_again.set_sensitive(true);
            }
        });
    }

    fn show(&self, lines: &[DoctorLine]) {
        for row in self.rows.borrow_mut().drain(..) {
            self.group.remove(&row);
        }
        for line in lines {
            let row = match line {
                DoctorLine::Check { ok, name, detail } => {
                    let row = widgets::row(name, detail);
                    let icon = gtk::Image::from_icon_name(if *ok {
                        "emblem-ok-symbolic"
                    } else {
                        "dialog-warning-symbolic"
                    });
                    icon.add_css_class(if *ok { "success" } else { "warning" });
                    icon.set_tooltip_text(Some(if *ok { "OK" } else { "Problem" }));
                    row.add_prefix(&icon);
                    row
                }
                DoctorLine::Note(text) => {
                    let row = widgets::row(text, "");
                    row.add_css_class("dim-label");
                    row
                }
            };
            row.set_title_lines(0);
            self.group.add(&row);
            self.rows.borrow_mut().push(row);
        }
    }
}
