//! The window's shared state: the one [`Store`] the window edits through, telling the daemon
//! to reload, keeping the widgets in step with the file, and the toasts and problem banner.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use nomouse_core::Config;
use nomouse_core::settings::{Change, Value};
use nomouse_linux::shortcuts::Shortcuts;

use crate::args::Page;
use crate::daemon::Daemon;
use crate::store::{Store, StoreError};

/// A callback that receives the config.
type ConfigListener = Rc<dyn Fn(&Config)>;
/// Switches the window to a page.
type Navigator = Rc<dyn Fn(Page)>;

/// How often the file is checked for edits made elsewhere.
const WATCH_TICK: Duration = Duration::from_millis(500);

/// Shared by every page.
pub struct State {
    store: RefCell<Store>,
    /// Sets widgets from a config that changed under them (the file, a reset).
    refreshers: RefCell<Vec<ConfigListener>>,
    /// Follows every change, ours or the file's (the preview, the compositor instructions).
    observers: RefCell<Vec<ConfigListener>>,
    refreshing: Cell<bool>,
    toasts: adw::ToastOverlay,
    /// The toast queued or shown last, until it is dismissed: the same message is not queued
    /// twice (libadwaita 1.1 cannot dismiss a toast that is still queued).
    last_toast: Rc<RefCell<Option<(String, adw::Toast)>>>,
    problem: gtk::InfoBar,
    problem_label: gtk::Label,
    navigator: RefCell<Option<Navigator>>,
    /// The daemon's control interface.
    pub daemon: Rc<Daemon>,
    /// Who owns the global shortcuts in this session.
    pub shortcuts: Shortcuts,
}

impl State {
    /// Opens the config file at `path`.
    ///
    /// # Errors
    /// When the file exists but cannot be read.
    pub fn new(
        path: PathBuf,
        daemon: Rc<Daemon>,
        toasts: adw::ToastOverlay,
    ) -> Result<Rc<Self>, String> {
        let store = Store::open(path)?;
        let problem_label = gtk::Label::builder().wrap(true).xalign(0.0).build();
        let problem = gtk::InfoBar::new();
        problem.set_message_type(gtk::MessageType::Error);
        problem.add_child(&problem_label);
        let state = Rc::new(Self {
            store: RefCell::new(store),
            refreshers: RefCell::new(Vec::new()),
            observers: RefCell::new(Vec::new()),
            refreshing: Cell::new(false),
            toasts,
            last_toast: Rc::new(RefCell::new(None)),
            problem,
            problem_label,
            navigator: RefCell::new(None),
            daemon,
            shortcuts: Shortcuts::current(),
        });
        state.show_problem();
        Ok(state)
    }

    /// The config file.
    pub fn path(&self) -> PathBuf {
        self.store.borrow().path().to_owned()
    }

    /// A copy of the configuration in effect.
    pub fn config(&self) -> Config {
        self.store.borrow().config().clone()
    }

    /// The value of `key` in effect.
    pub fn value(&self, key: &str) -> Value {
        self.store.borrow().value(key)
    }

    /// The banner that says why the file on disk is not in effect (hidden while it is).
    pub fn problem_bar(&self) -> &gtk::InfoBar {
        &self.problem
    }

    /// Sets `key` and saves it; when the file changed, asks the daemon to reload. Ignored
    /// while the widgets are being refreshed.
    ///
    /// # Errors
    /// [`StoreError::Rejected`] for an invalid value (for the control to show; nothing is
    /// written), [`StoreError::Failed`] when the file could not be changed (also toasted).
    pub fn set(&self, key: &str, value: Value) -> Result<(), StoreError> {
        if self.refreshing.get() {
            return Ok(());
        }
        let result = self.store.borrow_mut().set(key, value);
        match result {
            Ok(changed) => {
                if changed {
                    self.daemon.reload();
                    self.toast("Saved");
                }
                self.show_problem();
                self.notify(&self.config());
                Ok(())
            }
            Err(StoreError::Failed(error)) => {
                self.toast(&error);
                Err(StoreError::Failed(error))
            }
            Err(rejected) => Err(rejected),
        }
    }

    /// Removes every setting from the file and shows the defaults.
    pub fn reset(&self) {
        let result = self.store.borrow_mut().reset();
        match result {
            Ok(_) => {
                self.daemon.reload();
                self.refresh();
                self.toast("Reset to defaults");
            }
            Err(error) => self.toast(&error),
        }
    }

    /// Sets every widget from the configuration in effect, without saving.
    pub fn refresh(&self) {
        let config = self.config();
        self.refreshing.set(true);
        let refreshers = self.refreshers.borrow().clone();
        for refresh in refreshers {
            refresh(&config);
        }
        self.refreshing.set(false);
        self.show_problem();
        self.notify(&config);
    }

    fn notify(&self, config: &Config) {
        let observers = self.observers.borrow().clone();
        for observe in observers {
            observe(config);
        }
    }

    fn show_problem(&self) {
        let problem = self.store.borrow().problem().map(str::to_owned);
        if let Some(problem) = &problem {
            self.problem_label.set_text(&format!(
                "The config file has a mistake, so the settings shown are the last good ones: {problem}"
            ));
        }
        self.problem.set_visible(problem.is_some());
    }

    /// Registers how a page sets its widgets from the config, and runs it once now. Changes
    /// the widgets make while it runs are not saved.
    pub fn connect_refresh(&self, refresh: impl Fn(&Config) + 'static) {
        let refresh: ConfigListener = Rc::new(refresh);
        let config = self.config();
        self.refreshing.set(true);
        refresh(&config);
        self.refreshing.set(false);
        self.refreshers.borrow_mut().push(refresh);
    }

    /// Registers a callback for every change of the config, and runs it once now.
    pub fn connect_changed(&self, observe: impl Fn(&Config) + 'static) {
        let observe: ConfigListener = Rc::new(observe);
        observe(&self.config());
        self.observers.borrow_mut().push(observe);
    }

    /// Whether widgets are being set from the config (their change signals are not edits).
    pub fn is_refreshing(&self) -> bool {
        self.refreshing.get()
    }

    /// Shows `text` briefly, unless it is already showing or waiting to show.
    pub fn toast(&self, text: &str) {
        if self
            .last_toast
            .borrow()
            .as_ref()
            .is_some_and(|(shown, _)| shown == text)
        {
            return;
        }
        let toast = adw::Toast::new(&glib::markup_escape_text(text));
        toast.set_timeout(if text.len() > 40 { 6 } else { 2 });
        let last = Rc::downgrade(&self.last_toast);
        toast.connect_dismissed(move |dismissed| {
            if let Some(last) = last.upgrade() {
                let mut last = last.borrow_mut();
                if last.as_ref().is_some_and(|(_, toast)| toast == dismissed) {
                    *last = None;
                }
            }
        });
        self.toasts.add_toast(toast.clone());
        self.last_toast.replace(Some((text.to_owned(), toast)));
    }

    /// Sets how [`State::navigate`] switches pages.
    pub fn set_navigator(&self, navigate: impl Fn(Page) + 'static) {
        self.navigator.replace(Some(Rc::new(navigate)));
    }

    /// Shows `page`.
    pub fn navigate(&self, page: Page) {
        let navigate = self.navigator.borrow().clone();
        if let Some(navigate) = navigate {
            navigate(page);
        }
    }

    /// Follows the file on disk from a timer: a different valid config refreshes the
    /// widgets (our own saves are not changes), an invalid one shows the problem.
    pub fn watch(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(WATCH_TICK, move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let change = state.store.borrow_mut().watch();
            match change {
                Change::Unchanged => {}
                Change::Reloaded => state.refresh(),
                Change::Invalid(error) => {
                    state.show_problem();
                    state.toast(&error.to_string());
                }
            }
            glib::ControlFlow::Continue
        });
    }

    /// Opens the config file in the default editor, creating it first when it is missing.
    pub fn open_config_file(&self) {
        let created = self.store.borrow_mut().ensure_file();
        if let Err(error) = created {
            self.toast(&error);
            return;
        }
        let path = self.path();
        let uri = gio::File::for_path(&path).uri();
        if let Err(error) =
            gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        {
            self.toast(&format!("Cannot open {}: {error}", Path::display(&path)));
        }
    }
}
