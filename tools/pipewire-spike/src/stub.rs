//! Non-Linux stub: the workspace must build and type-check on Windows, but
//! PipeWire capture is Linux-only.

use anyhow::{bail, Result};

use crate::cli::{Cli, Command};

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::List => bail!(
            "listing PipeWire Audio/Sink endpoints is only supported on Linux; \
             this build has no PipeWire client"
        ),
        Command::Capture(_) => bail!(
            "PipeWire capture is only supported on Linux; \
             this build has no PipeWire client"
        ),
        Command::Measure(_) => bail!(
            "PipeWire measurement is only supported on Linux; \
             this build has no PipeWire client"
        ),
    }
}
