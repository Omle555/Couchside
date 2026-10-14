//! Solidfin: a lightweight, self-hosted movie and TV server for Windows.

mod api;
mod app;
mod boot;
mod lan;
mod logging;
mod manager;
mod ocr;
mod range;
mod recycle;
#[cfg(windows)]
mod service;
mod subtitles;
mod title_scan;
mod web;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Solidfin media server

USAGE:
    solidfin [COMMAND] [OPTIONS]

COMMANDS:
    run                  Run in this console window (default)
    install-service      Install and start Solidfin as a Windows service (run as Administrator)
    uninstall-service    Stop and remove the Windows service (run as Administrator)
    help                 Show this help
    version              Show the version

OPTIONS:
    --config <PATH>      Use a specific config.toml
    --no-browser         Don't open the web browser on start
";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run,
    Service,
    InstallService,
    UninstallService,
    Help,
    Version,
}

#[derive(Debug)]
struct Cli {
    command: Command,
    config: Option<PathBuf>,
    open_browser: bool,
}

fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli {
        command: Command::Run,
        config: None,
        open_browser: true,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "run" => cli.command = Command::Run,
            "service" => cli.command = Command::Service,
            "install-service" => cli.command = Command::InstallService,
            "uninstall-service" => cli.command = Command::UninstallService,
            "help" | "--help" | "-h" | "/?" => cli.command = Command::Help,
            "version" | "--version" | "-V" => cli.command = Command::Version,
            "--no-browser" => cli.open_browser = false,
            "--config" => {
                let path = iter.next().ok_or("--config needs a file path")?;
                cli.config = Some(PathBuf::from(path));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(cli)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse_args(&args) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let result = match cli.command {
        Command::Help => {
            print!("{USAGE}");
            Ok(())
        }
        Command::Version => {
            println!("solidfin {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Run => app::run_console(cli.config, cli.open_browser),
        #[cfg(windows)]
        Command::Service => service::run_dispatcher(),
        #[cfg(windows)]
        Command::InstallService => service::install(cli.config),
        #[cfg(windows)]
        Command::UninstallService => service::uninstall(),
        #[cfg(not(windows))]
        Command::Service | Command::InstallService | Command::UninstallService => Err(
            anyhow::anyhow!("Windows services are only available on Windows"),
        ),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Logging may not be initialised (e.g. a bad config file), so print too.
            eprintln!("Solidfin error: {err:#}");
            tracing::error!(error = %format!("{err:#}"), "fatal error");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parses_commands() {
        let cli = parse_args(&args(&[])).expect("ok");
        assert_eq!(cli.command, Command::Run);
        assert!(cli.open_browser);

        let cli =
            parse_args(&args(&["install-service", "--config", r"D:\s\config.toml"])).expect("ok");
        assert_eq!(cli.command, Command::InstallService);
        assert_eq!(cli.config, Some(PathBuf::from(r"D:\s\config.toml")));

        assert!(parse_args(&args(&["--config"])).is_err());
        assert!(parse_args(&args(&["bogus"])).is_err());
    }
}
