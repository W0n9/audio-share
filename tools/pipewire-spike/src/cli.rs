use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use pipewire_spike::PcmEncoding;

#[derive(Debug, Parser)]
#[command(
    name = "pipewire-spike",
    version,
    about = "Linux PipeWire capture spike for the Audio Share server rewrite"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List Audio/Sink endpoints and mark the default (highest priority.session)
    List,
    /// Capture monitor PCM from an Audio/Sink
    Capture(CaptureArgs),
    /// Measure callback/quantum stats, CPU, glitches, and repeated start/stop
    Measure(MeasureArgs),
}

#[derive(Debug, Args)]
pub struct CaptureArgs {
    /// object.serial of the sink, or "default"
    #[arg(long, default_value = "default", value_name = "SERIAL|default")]
    pub endpoint: String,

    /// Requested PCM encoding
    #[arg(long, value_enum, default_value = "f32")]
    pub encoding: PcmEncoding,

    /// Requested channels: 0 = endpoint default, 1, or 2
    #[arg(long, default_value_t = 0, value_name = "0|1|2")]
    pub channels: u32,

    /// Requested sample rate: 0 = endpoint default, 44100, or 48000
    #[arg(long, default_value_t = 0, value_name = "0|44100|48000")]
    pub sample_rate: u32,

    /// Capture duration in seconds
    #[arg(long, value_name = "N")]
    pub seconds: f64,

    /// Output file for raw interleaved PCM
    #[arg(long, value_name = "FILE")]
    pub out: PathBuf,
}

#[derive(Debug, Args)]
pub struct MeasureArgs {
    /// object.serial of the sink, or "default"
    #[arg(long, default_value = "default", value_name = "SERIAL|default")]
    pub endpoint: String,

    /// Requested PCM encoding
    #[arg(long, value_enum, default_value = "f32")]
    pub encoding: PcmEncoding,

    /// Requested channels: 0 = endpoint default, 1, or 2
    #[arg(long, default_value_t = 0, value_name = "0|1|2")]
    pub channels: u32,

    /// Requested sample rate: 0 = endpoint default, 44100, or 48000
    #[arg(long, default_value_t = 0, value_name = "0|44100|48000")]
    pub sample_rate: u32,

    /// Measured capture duration in seconds
    #[arg(long, value_name = "N")]
    pub seconds: f64,

    /// Number of start/stop cycles after the measured session
    #[arg(long, default_value_t = 10, value_name = "N")]
    pub cycles: usize,

    /// Duration of each start/stop cycle in seconds
    #[arg(long, default_value_t = 0.25, value_name = "N")]
    pub cycle_seconds: f64,
}
