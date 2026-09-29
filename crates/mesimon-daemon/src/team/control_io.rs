//! Bounded Mesophon network worker. No board state or authorization decisions.
use super::device::DeviceFile;
use mesimon_team::{
    control::{Auth, Wire, QUEUE},
    relay::RelayClient,
    wire::{Request, Response},
};
use std::{
    sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError},
    time::Duration,
};
use tungstenite::Message;

pub enum Event {
    /// The browser origin, and whether this relay keeps mail for the host
    /// while it is away (T-497).
    Online(String, bool),
    Offline,
    Frame(Wire),
}
pub fn spawn(
    device: DeviceFile,
    report: impl Fn(Event) -> bool + Send + 'static,
) -> SyncSender<Wire> {
    let (tx, rx) = sync_channel(QUEUE);
    std::thread::spawn(move || {
        // A worker owns one generation of the connection. The writer retries
        // by making another worker, so queued commands never cross reconnects.
        if run(device, rx, &report).is_err() {
            let _ = report(Event::Offline);
        }
    });
    tx
}
fn run(device: DeviceFile, rx: Receiver<Wire>, report: &impl Fn(Event) -> bool) -> Result<(), ()> {
    let client = RelayClient::new(device.relay).map_err(|_| ())?;
    let origin =
        match client.call(device.credential.as_ref(), Request::ControlInfo).map_err(|_| ())? {
            Response::ControlInfo { version: 1, origin: Some(origin) } => origin,
            _ => return Err(()),
        };
    // An older relay answers `InvalidRequest`: it keeps no mail, and this
    // host never sends it a frame it cannot read.
    let mail = matches!(
        client.call(device.credential.as_ref(), Request::ControlMail),
        Ok(Response::ControlMail { version: 1 })
    );
    let mut ws = client.control_socket(&origin).map_err(|_| ())?;
    let auth = Auth { credential: device.credential, register: None };
    ws.send(Message::Text(serde_json::to_string(&auth).map_err(|_| ())?.into())).map_err(|_| ())?;
    let Message::Text(text) = ws.read().map_err(|_| ())? else { return Err(()) };
    if !matches!(serde_json::from_str::<Wire>(&text), Ok(Wire::Authenticated { .. })) {
        return Err(());
    }
    ws.get_mut().set_read_timeout(Some(Duration::from_millis(100))).map_err(|_| ())?;
    if !report(Event::Online(origin, mail)) {
        return Err(());
    }
    loop {
        loop {
            match rx.try_recv() {
                Ok(wire) => ws
                    .send(Message::Text(serde_json::to_string(&wire).map_err(|_| ())?.into()))
                    .map_err(|_| ())?,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if !report(Event::Frame(serde_json::from_str(&text).map_err(|_| ())?)) {
                    return Err(());
                }
            }
            Ok(Message::Ping(_)) => {
                ws.flush().map_err(|_| ())?;
            }
            Ok(Message::Pong(_)) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            _ => return Err(()),
        }
    }
}
