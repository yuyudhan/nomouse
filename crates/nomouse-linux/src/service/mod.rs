//! The daemon on the session bus: its control interface ([`crate::control`]), and the tray
//! icon as a freedesktop `StatusNotifierItem` with a `com.canonical.dbusmenu` menu, which KDE,
//! waybar, Xfce and GNOME (with the `AppIndicator` extension Ubuntu ships enabled) show.
//!
//! Everything runs on zbus's own executor thread; the daemon's event loop only reads
//! [`Service::next_command`] and pushes state changes with [`Service::update`].

mod icon;
mod item;
mod menu;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_io::block_on;
use futures_util::StreamExt;
use nomouse_core::menu::{MenuAction, MenuState};
use zbus::object_server::SignalEmitter;

use crate::control::{BUS_NAME, OBJECT_PATH};

/// The well-known name of the tray hosts' registry.
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
/// Its object path.
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
/// Where the item lives (the path hosts assume when registered by bus name).
const ITEM_PATH: &str = "/StatusNotifierItem";
/// Where its menu lives.
const MENU_PATH: &str = "/MenuBar";
/// How long a blocking bus call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// What the daemon is asked to do over D-Bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// A tray menu pick, the item's click (Settings), or the control interface's
    /// `Pause`/`Resume`/`Run`/`Quit`.
    Menu(MenuAction),
    /// The control interface's `Reload`.
    Reload,
}

/// State shared with the interfaces on zbus's thread.
struct Shared {
    state: Mutex<MenuState>,
    revision: Mutex<u32>,
    commands: Mutex<Sender<Command>>,
}

impl Shared {
    fn state(&self) -> MenuState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn revision(&self) -> u32 {
        *self.revision.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, command: Command) {
        let sender = self.commands.lock().unwrap_or_else(PoisonError::into_inner);
        // The receiver lives as long as the daemon; a send after it quit has no one to tell.
        let _ = sender.send(command);
    }
}

/// The control interface, `io.github.yuyudhan.nomouse.Daemon1`.
struct Control {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "io.github.yuyudhan.nomouse.Daemon1")]
impl Control {
    /// Releases the hotkeys until `Resume`.
    fn pause(&self) {
        self.shared.send(Command::Menu(MenuAction::Pause));
    }

    /// Grabs the hotkeys again.
    fn resume(&self) {
        self.shared.send(Command::Menu(MenuAction::Resume));
    }

    /// Re-reads the configuration file now.
    fn reload(&self) {
        self.shared.send(Command::Reload);
    }

    /// Stops the daemon.
    fn quit(&self) {
        self.shared.send(Command::Menu(MenuAction::Quit));
    }

    /// Starts `hints`, `continuous`, `scroll` or `search`.
    fn run(&self, mode: &str) -> zbus::fdo::Result<()> {
        let action = match mode {
            "hints" => MenuAction::Hints,
            "continuous" => MenuAction::Continuous,
            "scroll" => MenuAction::Scroll,
            "search" => MenuAction::Search,
            other => {
                return Err(zbus::fdo::Error::InvalidArgs(format!(
                    "unknown mode {other:?}: hints, continuous, scroll or search"
                )));
            }
        };
        self.shared.send(Command::Menu(action));
        Ok(())
    }

    /// Whether the hotkeys are paused.
    #[zbus(property)]
    fn paused(&self) -> bool {
        self.shared.state().paused
    }

    /// Whether the backend runs in this session.
    #[zbus(property)]
    fn active(&self) -> bool {
        self.shared.state().active
    }

    /// The session warning, or empty.
    #[zbus(property)]
    fn warning(&self) -> String {
        self.shared.state().warning.unwrap_or_default()
    }

    /// The daemon's version.
    #[zbus(property)]
    fn version(&self) -> String {
        self.shared.state().version
    }
}

/// Whether the tray icon is shown, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayHost {
    /// Registered with a host that displays it.
    Shown,
    /// No `StatusNotifierWatcher` on the bus, or none with a host.
    Missing(String),
}

/// The daemon's presence on the session bus.
#[derive(Clone)]
pub struct Service {
    conn: zbus::Connection,
    shared: Arc<Shared>,
    commands: Arc<Mutex<Receiver<Command>>>,
}

impl Service {
    /// Connects to the session bus, owns [`BUS_NAME`], serves the control interface, the tray
    /// item and its menu, registers the item with the tray host, and keeps re-registering it
    /// whenever a host (re)starts.
    ///
    /// # Errors
    /// No session bus, or another daemon owns the name.
    pub fn start(state: MenuState) -> Result<Self, String> {
        let (sender, receiver) = channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            revision: Mutex::new(1),
            commands: Mutex::new(sender),
        });
        let conn = block_on(async {
            zbus::connection::Builder::session()?
                .name(BUS_NAME)?
                .serve_at(
                    OBJECT_PATH,
                    Control {
                        shared: Arc::clone(&shared),
                    },
                )?
                .serve_at(ITEM_PATH, item::Item::new(Arc::clone(&shared)))?
                .serve_at(MENU_PATH, menu::Menu::new(Arc::clone(&shared)))?
                .build()
                .await
        })
        .map_err(|e| format!("session bus: {e}"))?;
        let service = Self {
            conn,
            shared,
            commands: Arc::new(Mutex::new(receiver)),
        };
        service.follow_watcher();
        Ok(service)
    }

    /// Registers the item now and whenever a `StatusNotifierWatcher` appears.
    fn follow_watcher(&self) {
        let conn = self.conn.clone();
        let spawned = std::thread::Builder::new()
            .name("nomouse-tray".to_owned())
            .spawn(move || {
                block_on(async move {
                    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else {
                        return;
                    };
                    let changes = dbus
                        .receive_name_owner_changed_with_args(&[(0, WATCHER)])
                        .await;
                    register(&conn).await;
                    let Ok(mut changes) = changes else {
                        return;
                    };
                    while let Some(change) = changes.next().await {
                        if change.args().is_ok_and(|args| args.new_owner().is_some()) {
                            register(&conn).await;
                        }
                    }
                });
            });
        if let Err(error) = spawned {
            tracing::warn!("tray: cannot watch for a tray host: {error}");
        }
    }

    /// The oldest command waiting, without blocking.
    pub fn next_command(&self) -> Option<Command> {
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .try_recv()
            .ok()
    }

    /// The state the menu shows now.
    pub fn state(&self) -> MenuState {
        self.shared.state()
    }

    /// Changes the state and tells the tray host and the control interface's listeners.
    pub fn update(&self, change: impl FnOnce(&mut MenuState)) {
        let (before, after) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let before = state.clone();
            change(&mut state);
            (before, state.clone())
        };
        if before == after {
            return;
        }
        let revision = {
            let mut revision = self
                .shared
                .revision
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *revision = revision.wrapping_add(1);
            *revision
        };
        let result: zbus::Result<()> = block_on(async {
            let server = self.conn.object_server();
            let control = server.interface::<_, Control>(OBJECT_PATH).await?;
            let emitter = control.signal_emitter();
            if before.paused != after.paused {
                control.get().await.paused_changed(emitter).await?;
            }
            if before.active != after.active {
                control.get().await.active_changed(emitter).await?;
            }
            if before.warning != after.warning {
                control.get().await.warning_changed(emitter).await?;
            }
            let menu = SignalEmitter::new(&self.conn, MENU_PATH)?;
            menu::Menu::layout_updated(&menu, revision, 0).await?;
            if (before.paused, before.active) != (after.paused, after.active) {
                let item = SignalEmitter::new(&self.conn, ITEM_PATH)?;
                item::Item::new_icon(&item).await?;
                item::Item::new_status(&item, item::status(&after)).await?;
                item::Item::new_tool_tip(&item).await?;
            }
            Ok(())
        });
        if let Err(error) = result {
            tracing::debug!("tray: signalling a change: {error}");
        }
    }

    /// Whether a tray host shows the icon.
    pub fn tray_host(&self) -> TrayHost {
        block_on(watcher_host(&self.conn))
    }

    /// Shows a desktop notification (`org.freedesktop.Notifications`); failures are logged.
    pub fn notify(&self, summary: &str, body: &str) {
        let result: zbus::Result<u32> = block_on(async {
            let reply = self
                .conn
                .call_method(
                    Some("org.freedesktop.Notifications"),
                    "/org/freedesktop/Notifications",
                    Some("org.freedesktop.Notifications"),
                    "Notify",
                    &(
                        "nomouse",
                        0u32,
                        crate::control::ICON,
                        summary,
                        body,
                        Vec::<&str>::new(),
                        std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new(),
                        -1i32,
                    ),
                )
                .await?;
            reply.body().deserialize()
        });
        if let Err(error) = result {
            tracing::info!("notification {summary:?}: {error}");
        }
    }
}

/// Registers the item with the watcher, logging the outcome.
async fn register(conn: &zbus::Connection) {
    let Some(name) = conn.unique_name().map(ToString::to_string) else {
        return;
    };
    let result = conn
        .call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some(WATCHER),
            "RegisterStatusNotifierItem",
            &(name.as_str(),),
        )
        .await;
    match result {
        Ok(_) => match watcher_host(conn).await {
            TrayHost::Shown => tracing::info!("tray: icon shown"),
            TrayHost::Missing(why) => tracing::info!("tray: {why}"),
        },
        Err(error) => tracing::info!("tray: no tray host: {error}"),
    }
}

/// Asks the watcher whether a host is registered.
async fn watcher_host(conn: &zbus::Connection) -> TrayHost {
    let asked = async {
        let properties = zbus::fdo::PropertiesProxy::builder(conn)
            .destination(WATCHER)?
            .path(WATCHER_PATH)?
            .build()
            .await?;
        let value = properties
            .get(
                zbus::names::InterfaceName::from_static_str_unchecked(WATCHER),
                "IsStatusNotifierHostRegistered",
            )
            .await?;
        Ok::<bool, zbus::Error>(bool::try_from(value).unwrap_or(false))
    };
    let timeout = async {
        async_io::Timer::after(CALL_TIMEOUT).await;
        Err(zbus::Error::Failure("timed out".to_owned()))
    };
    match futures_util::future::select(Box::pin(asked), Box::pin(timeout))
        .await
        .factor_first()
        .0
    {
        Ok(true) => TrayHost::Shown,
        Ok(false) => TrayHost::Missing(
            "a StatusNotifierWatcher runs but no tray host is registered with it".to_owned(),
        ),
        Err(error) => TrayHost::Missing(format!(
            "no StatusNotifierWatcher on the session bus ({error})"
        )),
    }
}

/// The doctor's tray line: whether a tray host would show the icon, and what to do if not.
/// The icon is optional, so this is never a failure.
pub fn doctor_line() -> String {
    let checked = block_on(async {
        let conn = zbus::Connection::session().await?;
        Ok::<TrayHost, zbus::Error>(watcher_host(&conn).await)
    });
    let missing = |why: String| {
        format!(
            "no tray icon: {why}. GNOME needs the AppIndicator extension (Ubuntu ships it: \
             `gnome-extensions enable ubuntu-appindicators@ubuntu.com`); KDE, Xfce and waybar's \
             tray module show it. nomouse works without it: open its settings from the app launcher"
        )
    };
    match checked {
        Ok(TrayHost::Shown) => "a tray host shows the nomouse icon".to_owned(),
        Ok(TrayHost::Missing(why)) => missing(why),
        Err(error) => missing(format!("no session bus ({error})")),
    }
}
