//! Scripted Wire Protocol client, fixture recorder/replayer, and fake v0.3.4
//! server for hardware-free tests.

mod fake_server;
mod fixture;
mod process;
mod record;
mod replay;
mod synth;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "protocol-harness",
    version,
    about = "Wire Protocol harness for the Audio Share Server rewrite"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Launch an external server, record a fixture set from the live protocol.
    Record(record::RecordArgs),
    /// Write the synthesized Windows v0.3.4 fixture sets.
    Synthesize(synth::SynthesizeArgs),
    /// Validate fixtures offline, or drive a live server and compare framing.
    Replay(replay::ReplayArgs),
    /// Run a v0.3.4-compatible fake server with synthetic PCM.
    ServeFake(fake_server::ServeFakeArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Record(args) => record::run(&args).await,
        Command::Synthesize(args) => synth::run(&args),
        Command::Replay(args) => replay::run(&args).await,
        Command::ServeFake(args) => fake_server::run(&args).await,
    }
}
