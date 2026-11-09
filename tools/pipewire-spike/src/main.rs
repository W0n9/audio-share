mod cli;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod stub;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pipewire-spike: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "linux")]
fn run(cli: cli::Cli) -> anyhow::Result<()> {
    linux::run(cli)
}

#[cfg(not(target_os = "linux"))]
fn run(cli: cli::Cli) -> anyhow::Result<()> {
    stub::run(cli)
}
