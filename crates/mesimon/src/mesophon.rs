//! Browser setup uses the existing relay identity. Board enablement and
//! pairing remain explicit actions in the TUI; no trust store is changed.
use anyhow::{bail, Context, Result};
use mesimon_daemon::{team::device::DeviceFile, Paths};
use mesimon_team::{
    relay::RelayClient,
    wire::{Request, Response},
};

const USAGE: &str = "usage: mesimon mesophon setup [--check]\n\
Checks your signed-in relay and opens the browser app. --check only checks.\n\
For this Mac, configure the relay with WEB_ORIGIN=http://localhost:8444.\n\
In the TUI, open Esc → Remote Control, enable the board, and choose Pair a browser.";

pub fn run(args: &[String]) -> Result<()> {
    if matches!(args, [arg] if arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let check = match args {
        [command] if command == "setup" => false,
        [command, flag] if command == "setup" && flag == "--check" => true,
        _ => bail!("{USAGE}"),
    };
    let device = DeviceFile::load(&Paths::team_device_file()?)?
        .filter(|device| device.credential.is_some())
        .context("Sign in to your relay first: open Mesimon, Esc → Settings → Team")?;
    let relay = RelayClient::new(device.relay)?;
    let origin = match relay.call(device.credential.as_ref(), Request::ControlInfo) {
        Ok(Response::ControlInfo { version: 1, origin: Some(origin) }) => origin,
        Ok(Response::ControlInfo { origin: None, .. }) => {
            bail!("Enable the relay's browser listener. For this Mac, set WEB_ORIGIN=http://localhost:8444 and restart the relay.");
        }
        Ok(_) => bail!("This relay does not support this Remote Control version; upgrade the relay."),
        Err(error) => bail!("Could not discover Remote Control: {error}. Check that your relay is running and supports Remote Control."),
    };
    let mut socket = relay
        .control_socket(&origin)
        .context("The relay advertised a browser endpoint that could not be reached safely")?;
    let _ = socket.close(None);
    println!("Remote Control is ready at {origin}");
    if origin.starts_with("http:") {
        println!("This Mac only. No certificate setup is needed.");
    }
    println!("In Mesimon: Esc → Remote Control → Enable this board → Pair a browser.");
    if !check {
        let opener = if cfg!(target_os = "macos") { "/usr/bin/open" } else { "xdg-open" };
        let status = std::process::Command::new(opener)
            .arg(&origin)
            .status()
            .with_context(|| format!("Could not open the browser; open {origin} manually"))?;
        if !status.success() {
            bail!("Could not open the browser; open {origin} manually");
        }
    }
    Ok(())
}
