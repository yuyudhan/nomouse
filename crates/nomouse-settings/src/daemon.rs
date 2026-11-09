//! The running daemon, over its D-Bus control interface: whether it runs and is paused,
//! `Pause` / `Resume` / `Reload`, and starting it when it is not running.
//!
//! Every call is asynchronous and errors are ignored: without a daemon there is nothing to
//! pause or reload, and the status line says it is not running.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::rc::{Rc, Weak};

use gtk::gio;
use nomouse_linux::control::{BUS_NAME, INTERFACE, OBJECT_PATH};

use crate::launch;

/// What the settings window knows about the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonStatus {
    /// Not known yet (the bus has not answered).
    Unknown,
    /// No process owns the daemon's bus name.
    NotRunning,
    /// The daemon runs.
    Running {
        /// Whether its hotkeys are paused.
        paused: bool,
    },
}

/// Milliseconds a D-Bus call may take.
const TIMEOUT_MS: i32 = 3000;

/// Called with every status change.
type Listener = Rc<dyn Fn(DaemonStatus)>;

/// The daemon as seen from the session bus.
pub struct Daemon {
    connection: Option<gio::DBusConnection>,
    status: Cell<DaemonStatus>,
    listeners: RefCell<Vec<Listener>>,
    /// Set while a hotkey recorder paused the daemon, so it resumes only what it paused.
    paused_for_recording: Cell<bool>,
    /// Stops following the bus name (`gio` does not export the watcher id's type).
    unwatch: RefCell<Option<Box<dyn FnOnce()>>>,
    subscription: RefCell<Option<gio::SignalSubscription>>,
}

impl Daemon {
    /// Connects to the session bus and starts following the daemon's name and `Paused`.
    pub fn new() -> Rc<Self> {
        let connection = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).ok();
        let daemon = Rc::new(Self {
            connection,
            status: Cell::new(DaemonStatus::Unknown),
            listeners: RefCell::new(Vec::new()),
            paused_for_recording: Cell::new(false),
            unwatch: RefCell::new(None),
            subscription: RefCell::new(None),
        });
        let Some(connection) = daemon.connection.clone() else {
            daemon.set_status(DaemonStatus::NotRunning);
            return daemon;
        };
        let appeared = Rc::downgrade(&daemon);
        let vanished = Rc::downgrade(&daemon);
        let watcher = gio::bus_watch_name_on_connection(
            &connection,
            BUS_NAME,
            gio::BusNameWatcherFlags::NONE,
            move |_, _, _| {
                if let Some(daemon) = appeared.upgrade() {
                    daemon.refresh_paused();
                }
            },
            move |_, _| {
                if let Some(daemon) = vanished.upgrade() {
                    daemon.paused_for_recording.set(false);
                    daemon.set_status(DaemonStatus::NotRunning);
                }
            },
        );
        daemon
            .unwatch
            .replace(Some(Box::new(move || gio::bus_unwatch_name(watcher))));
        let changed = Rc::downgrade(&daemon);
        let subscription = connection.subscribe_to_signal(
            Some(BUS_NAME),
            Some("org.freedesktop.DBus.Properties"),
            Some("PropertiesChanged"),
            Some(OBJECT_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |_| {
                if let Some(daemon) = changed.upgrade() {
                    daemon.refresh_paused();
                }
            },
        );
        daemon.subscription.replace(Some(subscription));
        daemon
    }

    /// The last known status.
    pub fn status(&self) -> DaemonStatus {
        self.status.get()
    }

    /// Calls `listener` with every status change (not with the current one).
    pub fn connect_status(&self, listener: impl Fn(DaemonStatus) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(listener));
    }

    fn set_status(&self, status: DaemonStatus) {
        if self.status.replace(status) == status {
            return;
        }
        let listeners = self.listeners.borrow().clone();
        for listener in listeners {
            listener(status);
        }
    }

    /// Reads the `Paused` property; a failure means the daemon is gone.
    fn refresh_paused(self: &Rc<Self>) {
        let Some(connection) = &self.connection else {
            return;
        };
        let weak: Weak<Self> = Rc::downgrade(self);
        connection.call(
            Some(BUS_NAME),
            OBJECT_PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(INTERFACE, "Paused").into()),
            None,
            gio::DBusCallFlags::NONE,
            TIMEOUT_MS,
            None::<&gio::Cancellable>,
            move |result| {
                let Some(daemon) = weak.upgrade() else {
                    return;
                };
                let paused = result
                    .ok()
                    .and_then(|reply| reply.try_child_value(0)?.as_variant()?.get::<bool>());
                daemon.set_status(match paused {
                    Some(paused) => DaemonStatus::Running { paused },
                    // Running but without the property (an older daemon): assume not paused.
                    None if daemon.status.get() != DaemonStatus::NotRunning => {
                        DaemonStatus::Running { paused: false }
                    }
                    None => DaemonStatus::NotRunning,
                });
            },
        );
    }

    /// Calls `method` (no arguments), then re-reads `Paused`; errors are ignored.
    fn call(self: &Rc<Self>, method: &str) {
        let Some(connection) = &self.connection else {
            return;
        };
        let weak = Rc::downgrade(self);
        connection.call(
            Some(BUS_NAME),
            OBJECT_PATH,
            INTERFACE,
            method,
            None,
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            TIMEOUT_MS,
            None::<&gio::Cancellable>,
            move |_| {
                if let Some(daemon) = weak.upgrade() {
                    daemon.refresh_paused();
                }
            },
        );
    }

    /// Releases the hotkeys until [`Daemon::resume`].
    pub fn pause(self: &Rc<Self>) {
        self.call("Pause");
    }

    /// Grabs the hotkeys again.
    pub fn resume(self: &Rc<Self>) {
        self.call("Resume");
    }

    /// Asks the daemon to re-read `config.toml` now.
    pub fn reload(self: &Rc<Self>) {
        self.call("Reload");
    }

    /// Pauses a running, unpaused daemon while a hotkey is recorded, so its grabs do not
    /// swallow the chord.
    pub fn begin_recording(self: &Rc<Self>) {
        if self.status.get() == (DaemonStatus::Running { paused: false })
            && !self.paused_for_recording.replace(true)
        {
            self.pause();
        }
    }

    /// Resumes the daemon if [`Daemon::begin_recording`] paused it.
    pub fn end_recording(self: &Rc<Self>) {
        if self.paused_for_recording.replace(false) {
            self.resume();
        }
    }

    /// Starts the daemon unless something owns its bus name already.
    pub fn ensure_running(self: &Rc<Self>, on_error: impl Fn(String) + 'static) {
        let Some(connection) = &self.connection else {
            return;
        };
        connection.call(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            Some(&(BUS_NAME,).into()),
            None,
            gio::DBusCallFlags::NONE,
            TIMEOUT_MS,
            None::<&gio::Cancellable>,
            move |result| {
                let owned = result
                    .ok()
                    .and_then(|reply| reply.try_child_value(0)?.get::<bool>());
                if owned == Some(false)
                    && let Err(error) = start()
                {
                    on_error(error);
                }
            },
        );
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(unwatch) = self.unwatch.take() {
            unwatch();
        }
    }
}

/// Spawns `nomouse daemon` in its own process group (so it outlives the window), its output
/// in the log `nomouse launch` uses, truncated first.
///
/// # Errors
/// When the log cannot be created or the program cannot be started.
pub fn start() -> Result<(), String> {
    let program = launch::nomouse_program();
    let log = launch::log_path(|name| std::env::var(name).ok());
    let cannot = |e: std::io::Error| format!("cannot write {}: {e}", log.display());
    let stdout = File::create(&log).map_err(cannot)?;
    let stderr = stdout.try_clone().map_err(cannot)?;
    let mut command = Command::new(&program);
    command
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {} daemon: {e}", program.display()))?;
    // Reap the daemon when it exits, without making anyone wait for it.
    std::thread::spawn(move || child.wait().ok());
    Ok(())
}
