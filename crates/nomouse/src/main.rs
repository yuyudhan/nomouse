//! The `nomouse` command line: the hotkey daemon, one-shot hint and scroll modes, `doctor`,
//! the app launcher's `launch`, and configuration commands.
#![forbid(unsafe_code)]

mod desktop;
mod first_run;
#[cfg(unix)]
mod ipc;
#[cfg(target_os = "linux")]
mod launch;
mod lock;
mod run;

use std::fmt::Write as _;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use nomouse_core::config::{self, Config};
use nomouse_core::settings::{Change, Settings as ConfigFile};
use nomouse_core::{ActionKind, Alphabet, InputEvent, Platform, generate_labels};

use desktop::Desktop;
use run::{Mode, Outcome, Settings};

#[cfg(target_os = "linux")]
use nomouse_linux as backend;
#[cfg(target_os = "macos")]
use nomouse_macos as backend;
#[cfg(target_os = "windows")]
use nomouse_windows as backend;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod backend {
    use nomouse_core::{Config, Platform, PlatformError};

    pub(crate) const TARGET_OS: &str = std::env::consts::OS;
    pub(crate) const ACCESSIBILITY_REQUIREMENT: &str = "this OS has no nomouse backend";

    pub(crate) fn platform(_config: &Config) -> Result<Box<dyn Platform>, PlatformError> {
        Err(PlatformError::Unsupported(format!(
            "nomouse has no backend for {TARGET_OS}"
        )))
    }

    pub(crate) fn doctor(_config: &Config) -> Vec<(String, Result<String, String>)> {
        Vec::new()
    }

    pub(crate) fn first_run_permissions() -> Vec<nomouse_core::setup::Permission> {
        Vec::new()
    }

    pub(crate) fn first_run_probe() -> Box<dyn nomouse_core::setup::Probe> {
        Box::new(nomouse_core::setup::NothingToAsk)
    }

    pub(crate) fn first_run_dialogs() -> Option<Box<dyn nomouse_core::setup::Presenter>> {
        None
    }
}

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Command; `daemon` when omitted.
    #[command(subcommand)]
    command: Option<Command>,
    /// Run hints, continuous, scroll or search in this process even when a daemon is running
    /// (by default they are handed to the daemon over its socket).
    #[arg(long, global = true)]
    standalone: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Run the hotkey daemon (the default).
    Daemon,
    /// Show hints now and act on the picked one. Digits 1-4 toggle shift/ctrl/alt/super for
    /// the click (for example `2` then a label: ctrl+click).
    Hints {
        /// Keep re-labelling after every action until Escape.
        #[arg(long)]
        continuous: bool,
        /// Action instead of the configured one (click, right-click, double-click, hover, drag).
        #[arg(long)]
        action: Option<ActionKind>,
    },
    /// Continuous hints: keep re-labelling after every action until Escape (`hints
    /// --continuous`).
    Continuous {
        /// Action instead of the configured one (click, right-click, double-click, hover, drag).
        #[arg(long)]
        action: Option<ActionKind>,
    },
    /// Scroll the largest scroll area with j/k/h/l/d/u (Shift: faster), gg/G for top and
    /// bottom, Tab or a digit for another area, until Escape.
    Scroll,
    /// Search elements by name: type to filter, Enter clicks the best match, Tab then a label
    /// clicks another, Escape cancels.
    Search,
    /// Make the running daemon re-read its configuration file now (it also re-reads it on its
    /// own within a second or two of the file changing).
    Reload,
    /// Make the running daemon release its hotkeys (and ignore `nomouse hints` and the other
    /// modes) until `nomouse resume`.
    Pause,
    /// Make a paused daemon grab its hotkeys again.
    Resume,
    /// Stop the running daemon.
    Quit,
    /// Check the session, accessibility, input and configuration.
    Doctor,
    /// Walk through the permissions nomouse needs: explain each, ask the OS, wait for it.
    Setup,
    /// What the app launcher's nomouse entry runs: start the daemon unless it is running,
    /// then show a notification with its state and the hotkeys.
    #[cfg(target_os = "linux")]
    Launch {
        /// Open `nomouse doctor` in a terminal instead (shown as a notification without one).
        #[arg(long)]
        doctor: bool,
    },
    /// Draw the menu and every settings page, light and dark, as PNG files into DIR (the
    /// documentation's screenshots; needs no Screen Recording permission).
    #[cfg(target_os = "macos")]
    #[command(hide = true)]
    Screenshots {
        /// Directory to write into.
        dir: PathBuf,
    },
    /// Configuration commands.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Print the labels nomouse would assign to COUNT elements.
    Labels {
        /// Number of labels.
        count: usize,
        /// Alphabet to use instead of the configured one.
        #[arg(long)]
        alphabet: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the config file path.
    Path,
    /// Print the effective configuration (the file, or the defaults) as TOML.
    Show,
    /// Write the default configuration to the config path (refuses to overwrite).
    Init,
    /// Validate a config file (default: the config path).
    Check {
        /// File to check.
        path: Option<PathBuf>,
    },
}

fn init_logging() {
    let level = std::env::var("NOMOUSE_LOG")
        .ok()
        .and_then(|value| {
            value
                .parse::<tracing_subscriber::filter::LevelFilter>()
                .ok()
        })
        .unwrap_or(tracing_subscriber::filter::LevelFilter::WARN);
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging();
    match run_command(cli.command.unwrap_or(Command::Daemon), cli.standalone) {
        Ok(text) => {
            let mut stdout = std::io::stdout().lock();
            if stdout.write_all(text.as_bytes()).is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            // A failed stderr write leaves nothing else to report to.
            let _ = writeln!(std::io::stderr(), "nomouse: {message}");
            ExitCode::FAILURE
        }
    }
}

fn config_path(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    explicit
        .or_else(config::default_path)
        .ok_or_else(|| "no config path: set HOME (or APPDATA on Windows)".to_owned())
}

fn load_or_default() -> Result<Config, String> {
    match config::default_path() {
        Some(path) if path.exists() => Config::load(&path).map_err(|e| e.to_string()),
        _ => Ok(Config::default()),
    }
}

fn settings(config: &Config) -> Result<Settings, String> {
    Ok(Settings {
        alphabet: config.alphabet().map_err(|e| e.to_string())?,
        scope: config.hints.scope,
        scroll_step: i32::try_from(config.scroll.step).unwrap_or(3),
        dash_step: i32::try_from(config.scroll.dash_step).unwrap_or(15),
        keys: config.hints.keys,
        ignore: config.ignore.apps.clone(),
    })
}

fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Performed(count) => format!("performed {count} action(s)\n"),
        Outcome::Cancelled => "cancelled\n".to_owned(),
        Outcome::NoElements => "nothing to label in the focused window\n".to_owned(),
        Outcome::Ignored(entry) => {
            format!("the focused app is ignored (ignore.apps entry {entry:?})\n")
        }
    }
}

/// What the daemon is asked for besides its own hotkeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request {
    /// Run an activation.
    Run(Mode),
    /// Re-read the configuration file.
    Reload,
    /// Release the hotkeys and ignore activations until resumed.
    Pause,
    /// Grab the hotkeys again.
    Resume,
    /// Stop the daemon.
    Quit,
}

/// Runs `mode`: hands it to the running daemon when there is one (unless `standalone`), so
/// only one overlay exists and compositor key bindings reach the daemon; else runs it here.
fn activate(mode: Mode, standalone: bool) -> Result<String, String> {
    #[cfg(unix)]
    if !standalone {
        let path = ipc::current_path().map_err(|error| format!("command socket: {error}"))?;
        match ipc::send(&path, Request::Run(mode)) {
            Ok(()) => return Ok(String::new()),
            Err(ipc::SendError::NoDaemon(_)) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    #[cfg(not(unix))]
    let _ = standalone;
    let started = Instant::now();
    let config = load_or_default()?;
    let settings = settings(&config)?;
    let mut platform = backend::platform(&config).map_err(|e| e.to_string())?;
    // A one-shot run registers no hotkey and has no command socket: Escape or an action ends it.
    let outcome = run::activate(platform.as_mut(), &settings, mode, started, &|| false)
        .map_err(|e| e.to_string())?;
    Ok(describe(&outcome))
}

/// `nomouse reload`, `pause`, `resume` and `quit`: hands `request` to the running daemon and
/// returns `done`.
fn tell_daemon(request: Request, done: &str) -> Result<String, String> {
    #[cfg(unix)]
    {
        let path = ipc::current_path().map_err(|error| format!("command socket: {error}"))?;
        match ipc::send(&path, request) {
            Ok(()) => Ok(format!("{done}\n")),
            Err(ipc::SendError::NoDaemon(_)) => Err(if request == Request::Reload {
                "no daemon is running (a daemon reads the configuration when it starts)".to_owned()
            } else {
                "no daemon is running".to_owned()
            }),
            Err(error) => Err(error.to_string()),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (request, done);
        Err(
            "this system has no command socket; the daemon re-reads the configuration file \
             within two seconds of it changing"
                .to_owned(),
        )
    }
}

/// The daemon's live state: what a configuration reload replaces.
struct Running {
    config: Config,
    settings: Settings,
    modes: Vec<(nomouse_core::HotkeyId, Mode)>,
    /// `None` when the backend cannot start in this session: the daemon then runs only its
    /// tray menu and control interface, `problem` says why, and it retries now and then.
    platform: Option<Box<dyn Platform>>,
    problem: Option<String>,
    /// Hotkeys released and mode requests ignored (the tray's Pause).
    paused: bool,
}

/// Registers `config`'s hotkeys (a chord that cannot be registered is logged and skipped).
fn register(platform: &mut dyn Platform, config: &Config) -> Vec<(nomouse_core::HotkeyId, Mode)> {
    let action = config.hints.action;
    let bindings = [
        (config.hotkeys.hints, Mode::Once(action)),
        (config.hotkeys.continuous, Mode::Continuous(action)),
        (config.hotkeys.scroll, Mode::Scroll),
        (config.hotkeys.search, Mode::Search),
    ];
    let mut modes = Vec::new();
    for (hotkey, mode) in bindings {
        match platform.register(hotkey) {
            Ok(id) => modes.push((id, mode)),
            Err(error) => tracing::warn!("{hotkey}: {error}"),
        }
    }
    modes
}

/// Creates the platform for `config` and, unless `paused`, registers its hotkeys.
fn start(config: Config, paused: bool) -> Result<Running, String> {
    let settings = settings(&config)?;
    let mut platform = backend::platform(&config).map_err(|e| e.to_string())?;
    let modes = if paused {
        Vec::new()
    } else {
        register(platform.as_mut(), &config)
    };
    Ok(Running {
        config,
        settings,
        modes,
        platform: Some(platform),
        problem: None,
        paused,
    })
}

/// The state without a backend: only the tray menu and control interface run.
fn without_backend(config: Config, problem: String, paused: bool) -> Result<Running, String> {
    Ok(Running {
        settings: settings(&config)?,
        config,
        modes: Vec::new(),
        platform: None,
        problem: Some(problem),
        paused,
    })
}

impl Running {
    /// Releases the hotkeys; mode requests are ignored until [`Self::resume`].
    fn pause(&mut self) {
        if let Some(platform) = &mut self.platform {
            for (id, _) in self.modes.drain(..) {
                if let Err(error) = platform.unregister(id) {
                    tracing::warn!("pause: {error}");
                }
            }
        }
        self.paused = true;
        tracing::info!("paused");
    }

    /// Grabs the hotkeys again.
    fn resume(&mut self) {
        if let Some(platform) = &mut self.platform
            && self.paused
        {
            self.modes = register(platform.as_mut(), &self.config);
        }
        self.paused = false;
        tracing::info!("resumed with {} hotkey(s)", self.modes.len());
    }
}

/// Re-reads the configuration file and puts a changed configuration in effect. An unreadable
/// or invalid file, or an unchanged configuration with the backend running, keeps the running
/// state. Otherwise the old platform goes first (releasing
/// its hotkeys and overlay) and a new one starts with the new configuration, or with the old
/// one again when the new one fails to start. With `menu_only_ok` (the tray is up) a backend
/// that cannot start leaves the daemon running its menu only.
///
/// # Errors
/// When neither configuration can start a platform and the menu cannot run alone.
fn reload(running: Running, file: &mut ConfigFile, menu_only_ok: bool) -> Result<Running, String> {
    if let Change::Invalid(error) = file.reload() {
        tracing::warn!("config reload: {error}; keeping the running configuration");
        return Ok(running);
    }
    let config = file.config().clone();
    if config == running.config && running.platform.is_some() {
        tracing::info!("config reload: unchanged");
        return Ok(running);
    }
    let previous = running.config.clone();
    let paused = running.paused;
    let had_backend = running.platform.is_some();
    drop(running);
    match start(config.clone(), paused) {
        Ok(running) => {
            tracing::info!(
                "config reloaded: {} hotkey(s) registered",
                running.modes.len()
            );
            Ok(running)
        }
        Err(error) if !had_backend && menu_only_ok => {
            tracing::debug!("the backend still cannot start: {error}");
            without_backend(config, error, paused)
        }
        Err(error) => {
            tracing::warn!("config reload: {error}; restoring the previous configuration");
            match start(previous, paused) {
                Err(error) if menu_only_ok => {
                    tracing::warn!("{error}; running only the tray menu");
                    without_backend(config, error, paused)
                }
                other => other,
            }
        }
    }
}

/// Whether the configuration file changed into a new valid configuration since the last look
/// (at most once per [`nomouse_core::settings::WATCH_INTERVAL`]). An invalid edit, from a
/// settings window or a text editor, is logged once and the running configuration stays.
fn file_changed(file: &mut ConfigFile) -> bool {
    match file.watch() {
        Change::Unchanged => false,
        Change::Reloaded => {
            tracing::info!("the config file changed");
            true
        }
        Change::Invalid(error) => {
            tracing::warn!("config file: {error}; keeping the running configuration");
            false
        }
    }
}

fn daemon() -> Result<String, String> {
    let _lock = lock::acquire(&lock::current_path()).map_err(|e| e.to_string())?;
    first_run::before_daemon()?;
    let mut file = ConfigFile::open_default().map_err(|e| e.to_string())?;
    if let Some(problem) = file.problem() {
        return Err(problem.to_owned());
    }
    let config = file.config().clone();
    let mut desktop = Desktop::start(&config, None);
    let running = match start(config.clone(), false) {
        Ok(running) => running,
        // With the tray (or the settings window) able to say why, the daemon stays up.
        Err(error) if desktop.is_up() => {
            tracing::warn!("{error}; running only the tray menu until the session allows more");
            without_backend(config, error, false)?
        }
        Err(error) => return Err(error),
    };
    desktop.show(&running.config, running.paused, running.problem.as_deref());
    if !desktop.tray_shown() {
        tracing::info!(
            "no tray icon (`nomouse doctor` says why); settings open from the app launcher"
        );
    }
    let name = running
        .platform
        .as_ref()
        .map_or("tray-only", |platform| platform.name());
    #[cfg(unix)]
    {
        let bound = ipc::current_path().and_then(|socket| match ipc::Server::bind(&socket) {
            Ok(server) => Ok((socket, server)),
            Err(error) => Err(std::io::Error::new(
                error.kind(),
                format!("{}: {error}", socket.display()),
            )),
        });
        let (socket, server) = match bound {
            Ok(bound) => bound,
            Err(error) if !running.modes.is_empty() || desktop.is_up() => {
                tracing::warn!("command socket: {error}");
                return serve(running, &NoRequests, &mut file, &mut desktop);
            }
            Err(error) => {
                return Err(format!(
                    "no hotkey could be registered and the command socket failed: {error}"
                ));
            }
        };
        tracing::info!(
            "{name} daemon ready with {} hotkey(s) and commands on {}",
            running.modes.len(),
            socket.display()
        );
        serve(running, &server, &mut file, &mut desktop)
    }
    #[cfg(not(unix))]
    {
        if running.modes.is_empty() {
            return Err("no hotkey could be registered".to_owned());
        }
        tracing::info!("{name} daemon ready with {} hotkey(s)", running.modes.len());
        serve(running, &NoRequests, &mut file, &mut desktop)
    }
}

/// How often the daemon checks the command socket between hotkey events.
const COMMAND_POLL: Duration = Duration::from_millis(30);

/// How often a daemon without a backend tries to start one again.
const RETRY_BACKEND: Duration = Duration::from_secs(30);

/// How often the daemon looks for desktop changes made elsewhere (start at login).
const DESKTOP_REFRESH: Duration = Duration::from_secs(2);

/// Where the daemon's requests (besides its own hotkeys) come from.
trait Requests {
    /// The oldest request waiting.
    fn next(&self) -> Option<Request>;
    /// Drops the activations requested while a session ran; returns how many, and the other
    /// requests made meanwhile.
    fn discard(&self) -> (usize, Vec<Request>);
}

/// No command socket.
struct NoRequests;

impl Requests for NoRequests {
    fn next(&self) -> Option<Request> {
        None
    }

    fn discard(&self) -> (usize, Vec<Request>) {
        (0, Vec::new())
    }
}

#[cfg(unix)]
impl Requests for ipc::Server {
    fn next(&self) -> Option<Request> {
        self.poll()
    }

    fn discard(&self) -> (usize, Vec<Request>) {
        self.discard_pending()
    }
}

fn serve(
    mut running: Running,
    requests: &dyn Requests,
    file: &mut ConfigFile,
    desktop: &mut Desktop,
) -> Result<String, String> {
    let mut retried = Instant::now();
    let mut refreshed = Instant::now();
    // Requests made while a session ran, other than activations: carried out after it.
    let mut pending = std::collections::VecDeque::new();
    loop {
        let event = if let Some(platform) = running.platform.as_mut() {
            platform
                .next_event(COMMAND_POLL)
                .map_err(|e| e.to_string())?
        } else {
            std::thread::sleep(COMMAND_POLL);
            None
        };
        let started = Instant::now();
        if refreshed.elapsed() >= DESKTOP_REFRESH {
            refreshed = Instant::now();
            desktop.refresh();
        }
        // The daemon's own hotkeys first, then what waited for a session to end, the command
        // socket, the tray menu and D-Bus, the file watch, and a daemon without a backend
        // trying again.
        let request = match event {
            Some(InputEvent::Hotkey(id)) => running
                .modes
                .iter()
                .find(|(known, _)| *known == id)
                .map(|(_, mode)| Request::Run(*mode)),
            _ => pending.pop_front().or_else(|| requests.next()),
        }
        .or_else(|| desktop.next())
        .or_else(|| file_changed(file).then_some(Request::Reload))
        .or_else(|| {
            (running.platform.is_none() && retried.elapsed() >= RETRY_BACKEND).then(|| {
                retried = Instant::now();
                Request::Reload
            })
        });
        let mode = match request {
            Some(Request::Run(_)) if running.paused => {
                tracing::info!("paused: ignoring a mode request");
                continue;
            }
            Some(Request::Run(mode)) => mode,
            Some(Request::Reload) => {
                running = reload(running, file, desktop.is_up())?;
                desktop.show(&running.config, running.paused, running.problem.as_deref());
                continue;
            }
            Some(Request::Pause) => {
                running.pause();
                desktop.show(&running.config, running.paused, running.problem.as_deref());
                continue;
            }
            Some(Request::Resume) => {
                running.resume();
                desktop.show(&running.config, running.paused, running.problem.as_deref());
                continue;
            }
            Some(Request::Quit) => {
                tracing::info!("quitting on request");
                return Ok(String::new());
            }
            None => continue,
        };
        if running.platform.is_none() {
            tracing::info!(
                "cannot run a mode: {}",
                running.problem.as_deref().unwrap_or("no backend")
            );
            continue;
        }
        let Running {
            platform: Some(platform),
            settings,
            ..
        } = &mut running
        else {
            continue;
        };
        // An activation requested on the command socket while the labels are up (the
        // compositor binding pressed again) toggles them off; other requests wait for the end.
        let waiting = std::cell::RefCell::new(Vec::new());
        let dismissed = || match requests.next() {
            Some(Request::Run(_)) => true,
            Some(other) => {
                waiting.borrow_mut().push(other);
                false
            }
            None => false,
        };
        match run::activate(platform.as_mut(), settings, mode, started, &dismissed) {
            Ok(outcome) => tracing::info!("{}", describe(&outcome).trim_end()),
            // A lost display surfaces again from `next_event` and ends the daemon there.
            Err(error) => tracing::warn!("{error}"),
        }
        let (ignored, kept) = requests.discard();
        if ignored > 0 {
            tracing::info!("ignored {ignored} request(s) made while the session ran");
        }
        pending.extend(waiting.into_inner());
        pending.extend(kept);
    }
}

fn doctor() -> Result<String, String> {
    let (out, failed) = doctor_report()?;
    if failed {
        // Print the report and still exit non-zero.
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(out.as_bytes())
            .map_err(|e| e.to_string())?;
        return Err("some checks failed".to_owned());
    }
    Ok(out)
}

/// The `nomouse doctor` report, and whether any check failed.
pub(crate) fn doctor_report() -> Result<(String, bool), String> {
    let path = config_path(None)?;
    let (config, status) = if path.exists() {
        match Config::load(&path) {
            Ok(config) => (config, Ok(format!("{} is valid", path.display()))),
            Err(error) => (Config::default(), Err(error.to_string())),
        }
    } else {
        (
            Config::default(),
            Ok(format!("{} absent (defaults apply)", path.display())),
        )
    };
    let mut checks = backend::doctor(&config);
    #[cfg(target_os = "linux")]
    checks.push(desktop::doctor_line());
    checks.push(("config".to_owned(), status));
    let ignored = if config.ignore.apps.is_empty() {
        "none".to_owned()
    } else {
        config.ignore.apps.join(", ")
    };
    checks.push(("ignored apps".to_owned(), Ok(ignored)));
    let mut out = format!("platform: {}\n", backend::TARGET_OS);
    let mut failed = false;
    for (name, result) in &checks {
        let (mark, text) = match result {
            Ok(detail) => ("ok  ", detail),
            Err(problem) => {
                failed = true;
                ("FAIL", problem)
            }
        };
        let _ = writeln!(out, "[{mark}] {name}: {text}");
    }
    if failed {
        let _ = writeln!(out, "hint: {}", backend::ACCESSIBILITY_REQUIREMENT);
    }
    Ok((out, failed))
}

fn run_command(command: Command, standalone: bool) -> Result<String, String> {
    match command {
        Command::Daemon => daemon(),
        Command::Hints { continuous, action } => {
            let kind = match action {
                Some(ActionKind::Scroll) => return activate(Mode::Scroll, standalone),
                Some(kind) => kind,
                None => load_or_default()?.hints.action,
            };
            activate(
                if continuous {
                    Mode::Continuous(kind)
                } else {
                    Mode::Once(kind)
                },
                standalone,
            )
        }
        Command::Continuous { action } => {
            let kind = match action {
                Some(kind) => kind,
                None => load_or_default()?.hints.action,
            };
            activate(Mode::Continuous(kind), standalone)
        }
        Command::Scroll => activate(Mode::Scroll, standalone),
        Command::Search => activate(Mode::Search, standalone),
        Command::Reload => {
            tell_daemon(Request::Reload, "the daemon is reloading its configuration")
        }
        Command::Pause => tell_daemon(Request::Pause, "the daemon released its hotkeys"),
        Command::Resume => tell_daemon(Request::Resume, "the daemon grabs its hotkeys again"),
        Command::Quit => tell_daemon(Request::Quit, "the daemon is quitting"),
        Command::Doctor => doctor(),
        Command::Setup => first_run::setup_command(),
        #[cfg(target_os = "linux")]
        Command::Launch { doctor: false } => launch::launch(load_or_default()),
        #[cfg(target_os = "linux")]
        Command::Launch { doctor: true } => launch::doctor(),
        #[cfg(target_os = "macos")]
        Command::Screenshots { dir } => nomouse_macos::menubar::screenshots(&dir).map(|written| {
            written.iter().fold(String::new(), |mut out, path| {
                let _ = writeln!(out, "{}", path.display());
                out
            })
        }),
        Command::Config { command } => config_command(command),
        Command::Labels { count, alphabet } => {
            let alphabet = match alphabet {
                Some(chars) => Alphabet::new(&chars).map_err(|e| e.to_string())?,
                None => load_or_default()?.alphabet().map_err(|e| e.to_string())?,
            };
            let mut out = generate_labels(&alphabet, count).join("\n");
            out.push('\n');
            Ok(out)
        }
    }
}

fn config_command(command: ConfigCommand) -> Result<String, String> {
    match command {
        ConfigCommand::Path => Ok(format!("{}\n", config_path(None)?.display())),
        ConfigCommand::Show => Ok(load_or_default()?.to_toml()),
        ConfigCommand::Init => {
            let path = config_path(None)?;
            init_config(&path)?;
            Ok(format!("wrote {}\n", path.display()))
        }
        ConfigCommand::Check { path } => {
            let path = config_path(path)?;
            Config::load(&path).map_err(|e| e.to_string())?;
            Ok(format!("{}: ok\n", path.display()))
        }
    }
}

fn init_config(path: &std::path::Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(Config::default().to_toml().as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_with_explicit_alphabet() {
        let out = run_command(
            Command::Labels {
                count: 3,
                alphabet: Some("ab".into()),
            },
            false,
        )
        .unwrap();
        assert_eq!(out, "b\naa\nab\n");
    }

    #[test]
    fn bad_alphabet_is_an_error() {
        assert!(
            run_command(
                Command::Labels {
                    count: 1,
                    alphabet: Some("l".into())
                },
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn init_writes_valid_defaults_once() {
        let dir = std::env::temp_dir().join(format!("nomouse-init-{}", std::process::id()));
        let path = dir.join("nomouse").join("config.toml");
        init_config(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), Config::default());
        assert!(init_config(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cli_defaults_to_daemon() {
        assert!(Cli::try_parse_from(["nomouse"]).unwrap().command.is_none());
        let cli = Cli::try_parse_from([
            "nomouse",
            "hints",
            "--continuous",
            "--action",
            "right-click",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Hints {
                continuous: true,
                action: Some(ActionKind::RightClick)
            })
        ));
    }
}
