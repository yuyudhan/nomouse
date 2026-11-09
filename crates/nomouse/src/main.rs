//! The `nomouse` command line: the hotkey daemon, one-shot hint and scroll modes, `doctor`,
//! and configuration commands.
#![forbid(unsafe_code)]

#[cfg(unix)]
mod ipc;
mod lock;
mod run;

use std::fmt::Write as _;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use nomouse_core::config::{self, Config};
use nomouse_core::{ActionKind, Alphabet, InputEvent, Platform, generate_labels};

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
    /// Check the session, accessibility, input and configuration.
    Doctor,
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

/// `nomouse reload`: asks the running daemon to re-read its configuration.
fn reload_daemon() -> Result<String, String> {
    #[cfg(unix)]
    {
        let path = ipc::current_path().map_err(|error| format!("command socket: {error}"))?;
        match ipc::send(&path, Request::Reload) {
            Ok(()) => Ok("the daemon is reloading its configuration\n".to_owned()),
            Err(ipc::SendError::NoDaemon(_)) => Err(
                "no daemon is running (a daemon reads the configuration when it starts)".to_owned(),
            ),
            Err(error) => Err(error.to_string()),
        }
    }
    #[cfg(not(unix))]
    {
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
    platform: Box<dyn Platform>,
}

/// Creates the platform for `config` and registers its hotkeys (a chord that cannot be
/// registered is logged and skipped).
fn start(config: Config) -> Result<Running, String> {
    let settings = settings(&config)?;
    let mut platform = backend::platform(&config).map_err(|e| e.to_string())?;
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
    Ok(Running {
        config,
        settings,
        modes,
        platform,
    })
}

/// Re-reads the configuration. An unreadable or invalid file, or an unchanged one, keeps the
/// running state. Otherwise the old platform goes first (releasing its hotkeys and overlay)
/// and a new one starts with the new configuration, or with the old one again when the new
/// one fails to start.
///
/// # Errors
/// When neither configuration can start a platform.
fn reload(running: Running) -> Result<Running, String> {
    let config = match load_or_default() {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!("config reload: {error}; keeping the running configuration");
            return Ok(running);
        }
    };
    if config == running.config {
        tracing::info!("config reload: unchanged");
        return Ok(running);
    }
    let previous = running.config.clone();
    drop(running);
    match start(config) {
        Ok(running) => {
            tracing::info!(
                "config reloaded: {} hotkey(s) registered",
                running.modes.len()
            );
            Ok(running)
        }
        Err(error) => {
            tracing::warn!("config reload: {error}; restoring the previous configuration");
            start(previous)
        }
    }
}

/// How often the daemon looks at the configuration file's modification time.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// Notices changes to the configuration file (written, created or removed) by its
/// modification time, looked at no more than once per [`WATCH_INTERVAL`].
struct Watch {
    path: Option<PathBuf>,
    stamp: Option<std::time::SystemTime>,
    checked: Instant,
}

fn modified(path: Option<&std::path::Path>) -> Option<std::time::SystemTime> {
    std::fs::metadata(path?)
        .and_then(|meta| meta.modified())
        .ok()
}

impl Watch {
    fn new(path: Option<PathBuf>) -> Self {
        Self {
            stamp: modified(path.as_deref()),
            path,
            checked: Instant::now(),
        }
    }

    /// Whether the file changed since the last look.
    fn changed(&mut self) -> bool {
        if self.checked.elapsed() < WATCH_INTERVAL {
            return false;
        }
        self.checked = Instant::now();
        let stamp = modified(self.path.as_deref());
        if stamp == self.stamp {
            return false;
        }
        self.stamp = stamp;
        true
    }
}

fn daemon() -> Result<String, String> {
    let path = lock::lock_path(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var("USER").ok(),
    );
    let _lock = lock::acquire(&path).map_err(|e| e.to_string())?;
    let mut watch = Watch::new(config::default_path());
    let running = start(load_or_default()?)?;
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
            Err(error) if !running.modes.is_empty() => {
                tracing::warn!("command socket: {error}");
                return serve(running, &NoRequests, &mut watch);
            }
            Err(error) => {
                return Err(format!(
                    "no hotkey could be registered and the command socket failed: {error}"
                ));
            }
        };
        tracing::info!(
            "{} daemon ready with {} hotkey(s) and commands on {}",
            running.platform.name(),
            running.modes.len(),
            socket.display()
        );
        serve(running, &server, &mut watch)
    }
    #[cfg(not(unix))]
    {
        if running.modes.is_empty() {
            return Err("no hotkey could be registered".to_owned());
        }
        tracing::info!(
            "{} daemon ready with {} hotkey(s)",
            running.platform.name(),
            running.modes.len()
        );
        serve(running, &NoRequests, &mut watch)
    }
}

/// How often the daemon checks the command socket between hotkey events.
const COMMAND_POLL: Duration = Duration::from_millis(30);

/// Where the daemon's requests (besides its own hotkeys) come from.
trait Requests {
    /// The oldest request waiting.
    fn next(&self) -> Option<Request>;
    /// Drops the activations requested while a session ran; returns how many, and whether a
    /// reload was requested meanwhile.
    fn discard(&self) -> (usize, bool);
}

/// No command socket.
struct NoRequests;

impl Requests for NoRequests {
    fn next(&self) -> Option<Request> {
        None
    }

    fn discard(&self) -> (usize, bool) {
        (0, false)
    }
}

#[cfg(unix)]
impl Requests for ipc::Server {
    fn next(&self) -> Option<Request> {
        self.poll()
    }

    fn discard(&self) -> (usize, bool) {
        self.discard_pending()
    }
}

fn serve(
    mut running: Running,
    requests: &dyn Requests,
    watch: &mut Watch,
) -> Result<String, String> {
    loop {
        let event = running
            .platform
            .next_event(COMMAND_POLL)
            .map_err(|e| e.to_string())?;
        let started = Instant::now();
        // The daemon's own hotkeys first, then the command socket, then the file watch.
        let request = match event {
            Some(InputEvent::Hotkey(id)) => running
                .modes
                .iter()
                .find(|(known, _)| *known == id)
                .map(|(_, mode)| Request::Run(*mode)),
            _ => requests.next(),
        }
        .or_else(|| {
            watch.changed().then(|| {
                tracing::info!("the config file changed");
                Request::Reload
            })
        });
        let mode = match request {
            Some(Request::Run(mode)) => mode,
            Some(Request::Reload) => {
                running = reload(running)?;
                continue;
            }
            None => continue,
        };
        // An activation requested on the command socket while the labels are up (the
        // compositor binding pressed again) toggles them off; a reload waits for the end.
        let reload_requested = std::cell::Cell::new(false);
        let dismissed = || match requests.next() {
            Some(Request::Run(_)) => true,
            Some(Request::Reload) => {
                reload_requested.set(true);
                false
            }
            None => false,
        };
        let Running {
            platform, settings, ..
        } = &mut running;
        match run::activate(platform.as_mut(), settings, mode, started, &dismissed) {
            Ok(outcome) => tracing::info!("{}", describe(&outcome).trim_end()),
            // A lost display surfaces again from `next_event` and ends the daemon there.
            Err(error) => tracing::warn!("{error}"),
        }
        let (ignored, reload_pending) = requests.discard();
        if ignored > 0 {
            tracing::info!("ignored {ignored} request(s) made while the session ran");
        }
        if reload_pending || reload_requested.get() {
            running = reload(running)?;
        }
    }
}

fn doctor() -> Result<String, String> {
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
        // Print the report and still exit non-zero.
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(out.as_bytes())
            .map_err(|e| e.to_string())?;
        return Err("some checks failed".to_owned());
    }
    Ok(out)
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
        Command::Reload => reload_daemon(),
        Command::Doctor => doctor(),
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
