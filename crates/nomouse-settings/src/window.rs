//! The settings window: a sidebar of pages next to a stack of pages inside an adaptive
//! leaflet, a header bar with "Open config file" and the main menu, the session warning and
//! config problem banners, and the toasts.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use nomouse_core::settings::Section;
use nomouse_linux::control::APP_ID;

use crate::args::Page;
use crate::controls::Control;
use crate::daemon::Daemon;
use crate::pages::advanced::confirm_reset;
use crate::pages::doctor::DoctorPage;
use crate::pages::general::GeneralPage;
use crate::pages::section::SectionPage;
use crate::state::State;
use crate::widgets;

/// Styles the stock theme lacks.
const CSS: &str = ".error-subtitle .subtitle { color: @error_color; opacity: 1; }\n\
                   .preview-card { background-color: alpha(@view_fg_color, 0.06); }\n";

/// A window action's body.
type Action = Box<dyn Fn(&Rc<SettingsWindow>)>;

/// Every page, for callers (and the UI test) that drive the widgets.
pub struct Pages {
    /// General.
    pub general: Rc<GeneralPage>,
    /// The settings sections, in [`Section::ALL`] order.
    pub sections: Vec<SectionPage>,
    /// Doctor.
    pub doctor: Rc<DoctorPage>,
}

impl Pages {
    /// The page of a settings section.
    pub fn section(&self, section: Section) -> Option<&SectionPage> {
        let index = Section::ALL.iter().position(|s| *s == section)?;
        self.sections.get(index)
    }

    /// The control of the setting `key` (`section.key`).
    pub fn control(&self, key: &str) -> Option<&Control> {
        self.sections.iter().find_map(|page| page.control(key))
    }

    /// The page widget of `page`.
    pub fn widget(&self, page: Page) -> Option<&adw::PreferencesPage> {
        match page {
            Page::General => Some(&self.general.page),
            Page::Settings(section) => self.section(section).map(|section| &section.page),
            Page::Doctor => Some(&self.doctor.page),
        }
    }
}

/// The window and its parts.
pub struct SettingsWindow {
    /// The window.
    pub window: adw::ApplicationWindow,
    /// The shared state.
    pub state: Rc<State>,
    /// The pages.
    pub pages: Pages,
    stack: gtk::Stack,
    sidebar: gtk::ListBox,
    leaflet: adw::Leaflet,
    title: adw::WindowTitle,
}

impl SettingsWindow {
    /// Builds the window for the config file at `path` (not shown yet).
    ///
    /// # Errors
    /// When the file exists but cannot be read.
    pub fn new(
        app: Option<&adw::Application>,
        path: PathBuf,
        daemon: Rc<Daemon>,
    ) -> Result<Rc<Self>, String> {
        install_css();
        let toasts = adw::ToastOverlay::new();
        let state = State::new(path, daemon, toasts.clone())?;
        let pages = Pages {
            general: GeneralPage::new(&state),
            sections: Section::ALL
                .into_iter()
                .map(|section| SectionPage::new(&state, section))
                .collect(),
            doctor: DoctorPage::new(),
        };
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .hexpand(true)
            .build();
        for page in Page::ALL {
            if let Some(widget) = pages.widget(page) {
                stack.add_titled(widget, Some(page.id()), page.title());
            }
        }
        let sidebar = sidebar();
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(210)
            .child(&sidebar)
            .build();
        let leaflet = adw::Leaflet::builder().can_navigate_back(true).build();
        leaflet.append(&sidebar_scroll);
        leaflet.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        leaflet.append(&stack);
        leaflet.set_visible_child(&stack);

        let title = adw::WindowTitle::new("nomouse Settings", "");
        let header = header_bar(&title);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header.bar);
        if let Some(warning) = state.shortcuts.warning() {
            content.append(&banner(&warning));
        }
        content.append(state.problem_bar());
        content.append(&leaflet);
        toasts.set_child(Some(&content));

        let window = adw::ApplicationWindow::builder()
            .title("nomouse Settings")
            .default_width(900)
            .default_height(640)
            .icon_name(APP_ID)
            .content(&toasts)
            .build();
        if let Some(app) = app {
            window.set_application(Some(app));
        }
        let settings = Rc::new(Self {
            window,
            state,
            pages,
            stack,
            sidebar,
            leaflet,
            title,
        });
        settings.connect(&header);
        settings.state.watch();
        settings.show_page(Page::General);
        Ok(settings)
    }

    fn connect(self: &Rc<Self>, header: &Header) {
        let weak = Rc::downgrade(self);
        self.state.set_navigator(move |page| {
            if let Some(window) = weak.upgrade() {
                window.show_page(page);
            }
        });
        let weak = Rc::downgrade(self);
        self.sidebar.connect_row_activated(move |_, row| {
            let page = usize::try_from(row.index())
                .ok()
                .and_then(|index| Page::ALL.get(index).copied());
            if let (Some(window), Some(page)) = (weak.upgrade(), page) {
                window.show_page(page);
            }
        });
        let back = header.back.clone();
        let weak = Rc::downgrade(self);
        let update_back = move || {
            if let Some(window) = weak.upgrade() {
                let content = window.leaflet.visible_child();
                back.set_visible(
                    window.leaflet.is_folded()
                        && content.as_ref() == Some(window.stack.upcast_ref()),
                );
            }
        };
        let on_fold = update_back.clone();
        self.leaflet.connect_folded_notify(move |_| on_fold());
        self.leaflet
            .connect_visible_child_notify(move |_| update_back());
        let weak = Rc::downgrade(self);
        header.back.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.leaflet.navigate(adw::NavigationDirection::Back);
            }
        });
        let opener = Rc::clone(&self.state);
        header
            .open
            .connect_clicked(move |_| opener.open_config_file());
        self.install_actions();
    }

    /// The window actions behind the main menu.
    fn install_actions(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        let action = |name: &str, run: Action| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade() {
                    run(&window);
                }
            });
            group.add_action(&action);
        };
        action(
            "reset",
            Box::new(|window| {
                confirm_reset(Some(window.window.upcast_ref()), &window.state);
            }),
        );
        action(
            "doctor",
            Box::new(|window| {
                window.show_page(Page::Doctor);
                window.pages.doctor.run();
            }),
        );
        action(
            "check-updates",
            Box::new(|window| {
                window.show_page(Page::General);
                window.pages.general.check_updates();
            }),
        );
        action(
            "about",
            Box::new(|window| {
                window.show_about();
            }),
        );
        self.window.insert_action_group("win", Some(&group));
    }

    /// Shows `page` and selects it in the sidebar.
    pub fn show_page(&self, page: Page) {
        self.stack.set_visible_child_name(page.id());
        self.title.set_subtitle(page.title());
        let index = Page::ALL.iter().position(|p| *p == page);
        if let Some(row) = index
            .and_then(|index| i32::try_from(index).ok())
            .and_then(|index| self.sidebar.row_at_index(index))
        {
            self.sidebar.select_row(Some(&row));
        }
        self.leaflet.set_visible_child(&self.stack);
        if page == Page::Doctor {
            self.pages.doctor.run_once();
        }
    }

    /// The page shown now.
    pub fn page(&self) -> Option<Page> {
        self.stack
            .visible_child_name()
            .and_then(|name| Page::from_id(&name))
    }

    /// Opens the About dialog.
    pub fn show_about(&self) -> gtk::AboutDialog {
        let about = gtk::AboutDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .program_name("nomouse")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Click anything with the keyboard")
            .website("https://github.com/yuyudhan/nomouse")
            .website_label("github.com/yuyudhan/nomouse")
            .license_type(gtk::License::Custom)
            .license("nomouse is licensed under MIT OR Apache-2.0, at your option.")
            .logo_icon_name(APP_ID)
            .build();
        about.present();
        about
    }
}

/// The header bar's interactive parts.
struct Header {
    bar: adw::HeaderBar,
    back: gtk::Button,
    open: gtk::Button,
}

fn header_bar(title: &adw::WindowTitle) -> Header {
    let bar = adw::HeaderBar::new();
    bar.set_title_widget(Some(title));
    let back: gtk::Button = widgets::named("Back");
    back.set_icon_name("go-previous-symbolic");
    back.set_visible(false);
    bar.pack_start(&back);
    let open: gtk::Button = widgets::named("Open config file");
    open.set_icon_name("document-open-symbolic");
    open.set_tooltip_text(Some("Open config file"));
    bar.pack_start(&open);
    let menu = gio::Menu::new();
    menu.append(Some("Reset to defaults…"), Some("win.reset"));
    menu.append(Some("Run doctor"), Some("win.doctor"));
    menu.append(Some("Check for updates"), Some("win.check-updates"));
    menu.append(Some("About nomouse"), Some("win.about"));
    let menu_button: gtk::MenuButton = widgets::named("Main menu");
    menu_button.set_icon_name("open-menu-symbolic");
    menu_button.set_menu_model(Some(&menu));
    menu_button.set_primary(true);
    menu_button.set_tooltip_text(Some("Main menu"));
    bar.pack_end(&menu_button);
    Header { bar, back, open }
}

/// The sidebar: one row per page, named after it.
fn sidebar() -> gtk::ListBox {
    let list: gtk::ListBox = widgets::named("Sections");
    list.add_css_class("navigation-sidebar");
    list.set_selection_mode(gtk::SelectionMode::Single);
    for page in Page::ALL {
        let row: gtk::ListBoxRow = widgets::named(page.title());
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        line.set_margin_top(6);
        line.set_margin_bottom(6);
        line.set_margin_start(6);
        line.append(&gtk::Image::from_icon_name(page.icon()));
        line.append(&gtk::Label::new(Some(page.title())));
        row.set_child(Some(&line));
        list.append(&row);
    }
    list
}

/// The session warning across the top of every page.
fn banner(text: &str) -> gtk::InfoBar {
    let bar = gtk::InfoBar::new();
    bar.set_message_type(gtk::MessageType::Warning);
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .build();
    bar.add_child(&label);
    bar
}

fn install_css() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Runs the main context until `done` holds or `timeout` passes; for tests and screenshots.
pub fn iterate_until(timeout: std::time::Duration, done: impl Fn() -> bool) -> bool {
    let context = glib::MainContext::default();
    let start = std::time::Instant::now();
    while !done() {
        if start.elapsed() > timeout {
            return false;
        }
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    true
}
