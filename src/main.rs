//! busk — a keyboard-driven D-Bus explorer and debugger TUI.

mod app;
mod bus;
mod model;
mod ui;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "busk", version, about = "A D-Bus explorer and debugger TUI")]
struct Cli {
    /// Connect to the system bus instead of the session bus.
    #[arg(short, long)]
    system: bool,

    /// List names on the bus and exit (no TUI). Useful for scripting.
    #[arg(short, long)]
    list: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = bus::BusClient::connect(cli.system).await?;

    if cli.list {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        for s in client.list_services().await? {
            let res = if s.well_known {
                writeln!(out, "{:<52} {}", s.name, s.owner.unwrap_or_default())
            } else {
                writeln!(out, "{}", s.name)
            };
            // e.g. `busk --list | head`: reader went away, not an error.
            if res.is_err() {
                break;
            }
        }
        return Ok(());
    }

    app::run(client).await
}
