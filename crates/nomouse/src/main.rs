//! The `nomouse` command line: the hotkey daemon, one-shot hint and scroll modes, `doctor`,
//! and configuration commands.
#![forbid(unsafe_code)]

mod first_run;
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
    /// Pick a scroll target, then scroll with j/k/h/l/d/u until Escape.
    Scroll,
    /// Search elements by name: type to filter, Enter clicks the best match, Tab then a label
    /// clicks another, Escape cancels.
    Search,
    /// Check the session, accessibility, input and configuration.
    Doctor,
    /// Walk through the permissions nomouse needs: explain each, ask the OS, wait for it.
    Setup,
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
    })
}

fn describe(outcome: Outcome) -> String {
    match outcome {
        Outcome::Performed(count) => format!("performed {count} action(s)\n"),
        Outcome::Cancelled => "cancelled\n".to_owned(),
        Outcome::NoElements => "nothing to label in the focused window\n".to_owned(),
    }
}

/// Runs `mode`: hands it to the running daemon when there is one (unless `standalone`), so
/// only one overlay exists and compositor key bindings reach the daemon; else runs it here.
fn activate(mode: Mode, standalone: bool) -> Result<String, String> {
    #[cfg(unix)]
    if !standalone {
        let path = ipc::current_path().map_err(|error| format!("command socket: {error}"))?;
        match ipc::send(&path, mode) {
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
    let outcome =
        run::activate(platform.as_mut(), &settings, mode, started).map_err(|e| e.to_string())?;
    Ok(describe(outcome))
}

fn daemon() -> Result<String, String> {
    let path = lock::lock_path(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var("USER").ok(),
    );
    let _lock = lock::acquire(&path).map_err(|e| e.to_string())?;
    first_run::before_daemon()?;
    let config = load_or_default()?;
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
            Err(error) if !modes.is_empty() => {
                tracing::warn!("command socket: {error}");
                return serve(platform.as_mut(), &settings, &modes, &NoRequests);
            }
            Err(error) => {
                return Err(format!(
                    "no hotkey could be registered and the command socket failed: {error}"
                ));
            }
        };
        tracing::info!(
            "{} daemon ready with {} hotkey(s) and commands on {}",
            platform.name(),
            modes.len(),
            socket.display()
        );
        serve(platform.as_mut(), &settings, &modes, &server)
    }
    #[cfg(not(unix))]
    {
        if modes.is_empty() {
            return Err("no hotkey could be registered".to_owned());
        }
        tracing::info!(
            "{} daemon ready with {} hotkey(s)",
            platform.name(),
            modes.len()
        );
        serve(platform.as_mut(), &settings, &modes, &NoRequests)
    }
}

/// How often the daemon checks the command socket between hotkey events.
const COMMAND_POLL: Duration = Duration::from_millis(30);

/// Where the daemon's mode requests (besides its own hotkeys) come from.
trait Requests {
    /// The oldest request waiting.
    fn next(&self) -> Option<Mode>;
    /// Drops the requests made while a session ran; returns how many.
    fn discard(&self) -> usize;
}

/// No command socket.
struct NoRequests;

impl Requests for NoRequests {
    fn next(&self) -> Option<Mode> {
        None
    }

    fn discard(&self) -> usize {
        0
    }
}

#[cfg(unix)]
impl Requests for ipc::Server {
    fn next(&self) -> Option<Mode> {
        self.poll()
    }

    fn discard(&self) -> usize {
        self.discard_pending()
    }
}

fn serve(
    platform: &mut dyn Platform,
    settings: &Settings,
    modes: &[(nomouse_core::HotkeyId, Mode)],
    requests: &dyn Requests,
) -> Result<String, String> {
    loop {
        let event = platform
            .next_event(COMMAND_POLL)
            .map_err(|e| e.to_string())?;
        let started = Instant::now();
        let mode = match event {
            Some(InputEvent::Hotkey(id)) => modes
                .iter()
                .find(|(known, _)| *known == id)
                .map(|(_, mode)| *mode),
            _ => requests.next(),
        };
        let Some(mode) = mode else {
            continue;
        };
        match run::activate(platform, settings, mode, started) {
            Ok(outcome) => tracing::info!("{}", describe(outcome).trim_end()),
            // A lost display surfaces again from `next_event` and ends the daemon there.
            Err(error) => tracing::warn!("{error}"),
        }
        let ignored = requests.discard();
        if ignored > 0 {
            tracing::info!("ignored {ignored} request(s) made while the session ran");
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
        Command::Doctor => doctor(),
        Command::Setup => first_run::setup_command(),
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
