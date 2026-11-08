//! The `nomouse` command line: the hotkey daemon, one-shot hint and scroll modes, `doctor`,
//! and configuration commands.
#![forbid(unsafe_code)]

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
}

#[derive(Subcommand)]
enum Command {
    /// Run the hotkey daemon (the default).
    Daemon,
    /// Show hints now and act on the picked one.
    Hints {
        /// Keep re-labelling after every action until Escape.
        #[arg(long)]
        continuous: bool,
        /// Action instead of the configured one (click, right-click, double-click, hover, drag).
        #[arg(long)]
        action: Option<ActionKind>,
    },
    /// Pick a scroll target, then scroll with j/k/h/l/d/u until Escape.
    Scroll,
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
    match run_command(cli.command.unwrap_or(Command::Daemon)) {
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

fn activate_now(mode: Mode) -> Result<String, String> {
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
    let config = load_or_default()?;
    let settings = settings(&config)?;
    let mut platform = backend::platform(&config).map_err(|e| e.to_string())?;
    let action = config.hints.action;
    let bindings = [
        (config.hotkeys.hints, Mode::Once(action)),
        (config.hotkeys.continuous, Mode::Continuous(action)),
        (config.hotkeys.scroll, Mode::Scroll),
    ];
    let mut modes = Vec::new();
    for (hotkey, mode) in bindings {
        match platform.register(hotkey) {
            Ok(id) => modes.push((id, mode)),
            Err(error) => tracing::warn!("{hotkey}: {error}"),
        }
    }
    if modes.is_empty() {
        return Err("no hotkey could be registered".to_owned());
    }
    tracing::info!(
        "{} daemon ready with {} hotkey(s)",
        platform.name(),
        modes.len()
    );
    serve(platform.as_mut(), &settings, &modes)
}

fn serve(
    platform: &mut dyn Platform,
    settings: &Settings,
    modes: &[(nomouse_core::HotkeyId, Mode)],
) -> Result<String, String> {
    loop {
        let event = platform
            .next_event(Duration::from_secs(1))
            .map_err(|e| e.to_string())?;
        let Some(InputEvent::Hotkey(id)) = event else {
            continue;
        };
        let started = Instant::now();
        let Some((_, mode)) = modes.iter().find(|(known, _)| *known == id) else {
            continue;
        };
        match run::activate(platform, settings, *mode, started) {
            Ok(outcome) => tracing::info!("{}", describe(outcome).trim_end()),
            // A lost display surfaces again from `next_event` and ends the daemon there.
            Err(error) => tracing::warn!("{error}"),
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

fn run_command(command: Command) -> Result<String, String> {
    match command {
        Command::Daemon => daemon(),
        Command::Hints { continuous, action } => {
            let kind = match action {
                Some(ActionKind::Scroll) => return activate_now(Mode::Scroll),
                Some(kind) => kind,
                None => load_or_default()?.hints.action,
            };
            activate_now(if continuous {
                Mode::Continuous(kind)
            } else {
                Mode::Once(kind)
            })
        }
        Command::Scroll => activate_now(Mode::Scroll),
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
        let out = run_command(Command::Labels {
            count: 3,
            alphabet: Some("ab".into()),
        })
        .unwrap();
        assert_eq!(out, "b\naa\nab\n");
    }

    #[test]
    fn bad_alphabet_is_an_error() {
        assert!(
            run_command(Command::Labels {
                count: 1,
                alphabet: Some("l".into())
            })
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
