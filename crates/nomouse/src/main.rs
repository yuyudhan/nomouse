//! The `nomouse` command line.
//!
//! Until the first backend lands (roadmap M1) the binary offers the platform-independent
//! commands: config inspection and validation, label preview, and `doctor`.
#![forbid(unsafe_code)]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nomouse_core::config::{self, Config};
use nomouse_core::{Alphabet, generate_labels};

#[cfg(target_os = "linux")]
use nomouse_linux as backend;
#[cfg(target_os = "macos")]
use nomouse_macos as backend;
#[cfg(target_os = "windows")]
use nomouse_windows as backend;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod backend {
    pub(crate) const TARGET_OS: &str = std::env::consts::OS;
    pub(crate) const ACCESSIBILITY_REQUIREMENT: &str = "this OS has no nomouse backend";
}

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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
    /// Report the platform, config path and accessibility requirement.
    Doctor,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the default configuration as TOML.
    Default,
    /// Print the config file path.
    Path,
    /// Validate a config file (default: the config path).
    Check {
        /// File to check.
        path: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
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

fn run(command: Command) -> Result<String, String> {
    match command {
        Command::Config {
            command: ConfigCommand::Default,
        } => Ok(Config::default().to_toml()),
        Command::Config {
            command: ConfigCommand::Path,
        } => Ok(format!("{}\n", config_path(None)?.display())),
        Command::Config {
            command: ConfigCommand::Check { path },
        } => {
            let path = config_path(path)?;
            Config::load(&path).map_err(|e| e.to_string())?;
            Ok(format!("{}: ok\n", path.display()))
        }
        Command::Labels { count, alphabet } => {
            let alphabet = match alphabet {
                Some(chars) => Alphabet::new(&chars).map_err(|e| e.to_string())?,
                None => load_or_default()?.alphabet().map_err(|e| e.to_string())?,
            };
            let mut out = generate_labels(&alphabet, count).join("\n");
            out.push('\n');
            Ok(out)
        }
        Command::Doctor => {
            let path = config_path(None)?;
            let status = if path.exists() {
                Config::load(&path).map_or_else(|e| e.to_string(), |_| "ok".to_owned())
            } else {
                "absent (defaults apply)".to_owned()
            };
            Ok(format!(
                "platform: {}\nconfig: {} ({status})\naccessibility: {}\n",
                backend::TARGET_OS,
                path.display(),
                backend::ACCESSIBILITY_REQUIREMENT
            ))
        }
    }
}

fn load_or_default() -> Result<Config, String> {
    match config::default_path() {
        Some(path) if path.exists() => Config::load(&path).map_err(|e| e.to_string()),
        _ => Ok(Config::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_with_explicit_alphabet() {
        let out = run(Command::Labels {
            count: 3,
            alphabet: Some("ab".into()),
        })
        .unwrap();
        assert_eq!(out, "b\naa\nab\n");
    }

    #[test]
    fn default_config_validates() {
        let text = run(Command::Config {
            command: ConfigCommand::Default,
        })
        .unwrap();
        assert!(Config::from_toml(&text).is_ok());
    }

    #[test]
    fn bad_alphabet_is_an_error() {
        assert!(
            run(Command::Labels {
                count: 1,
                alphabet: Some("l".into())
            })
            .is_err()
        );
    }
}
