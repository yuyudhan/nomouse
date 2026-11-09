//! `nomouse-settings`: the nomouse settings window (Linux). On macOS the settings are part of
//! nomouse.app.
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    nomouse_settings::app::run()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    use std::io::Write;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version") {
        // Nothing else to do when stdout is closed.
        writeln!(
            std::io::stdout(),
            "{}",
            nomouse_settings::args::version_line()
        )
        .ok();
        return std::process::ExitCode::SUCCESS;
    }
    // Nothing else to do when stderr is closed.
    writeln!(
        std::io::stderr(),
        "the settings window is part of nomouse.app on macOS"
    )
    .ok();
    std::process::ExitCode::FAILURE
}
