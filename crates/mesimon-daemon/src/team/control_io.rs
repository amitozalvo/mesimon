//! Bounded Mesophon network worker. No board state or authorization decisions.
use super::device::DeviceFile;
use mesimon_team::{
    control::{Auth, Wire, QUEUE},
    relay::RelayClient,
    wire::{Request, Response},
};
use std::{
    sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};
use tungstenite::Message;

pub enum Event {
    /// The browser origin, whether this relay keeps mail for the host while
    /// it is away (T-497), and whether it keeps a shelf for each paired
    /// browser (T-698).
    Online(String, bool, bool),
    Offline,
    Frame(Wire),
}
/// The relay pings every 15 s and drops a peer it has not heard from in 45 s;
/// the host keeps the same clock from its side (T-640). A socket the relay
/// dropped while the Mac slept stays open here with nothing to read, so
/// silence is the only sign: past `SILENT` the worker ends and the writer
/// reconnects, where it once sat on the dead socket and the browser read the
/// terminal as out of reach until the daemon restarted.
const PING_EVERY: Duration = Duration::from_secs(15);
const SILENT: Duration = Duration::from_secs(45);

#[derive(Debug, PartialEq)]
enum Beat {
    Wait,
    Ping,
    Dead,
}
fn beat(now: Instant, heard: Instant, pinged: Instant) -> Beat {
    if now.duration_since(heard) > SILENT {
        Beat::Dead
    } else if now.duration_since(pinged) >= PING_EVERY {
        Beat::Ping
    } else {
        Beat::Wait
    }
}

pub fn spawn(
    device: DeviceFile,
    report: impl Fn(Event) -> bool + Send + 'static,
) -> (SyncSender<Wire>, std::thread::JoinHandle<()>) {
    let (tx, rx) = sync_channel(QUEUE);
    let worker = std::thread::spawn(move || {
        // A worker owns one generation of the connection. The writer retries
        // by making another worker, so queued commands never cross reconnects.
        if run(device, rx, &report).is_err() {
            let _ = report(Event::Offline);
        }
    });
    (tx, worker)
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
    // The same for the shelf (T-698): an older relay keeps none, and is
    // never sent a `Shelve` it would drop the host for.
    let shelf = matches!(
        client.call(device.credential.as_ref(), Request::ControlShelf),
        Ok(Response::ControlShelf { version: 1 })
    );
    let mut ws = client.control_socket(&origin).map_err(|_| ())?;
    let auth = Auth { credential: device.credential, register: None };
    ws.send(Message::Text(serde_json::to_string(&auth).map_err(|_| ())?.into())).map_err(|_| ())?;
    let Message::Text(text) = ws.read().map_err(|_| ())? else { return Err(()) };
    if !matches!(serde_json::from_str::<Wire>(&text), Ok(Wire::Authenticated { .. })) {
        return Err(());
    }
    ws.get_mut().set_read_timeout(Some(Duration::from_millis(100))).map_err(|_| ())?;
    if !report(Event::Online(origin, mail, shelf)) {
        return Err(());
    }
    let (mut heard, mut pinged) = (Instant::now(), Instant::now());
    loop {
        match beat(Instant::now(), heard, pinged) {
            Beat::Dead => return Err(()),
            Beat::Ping => {
                ws.send(Message::Ping(Default::default())).map_err(|_| ())?;
                pinged = Instant::now();
            }
            Beat::Wait => {}
        }
        loop {
            match rx.try_recv() {
                Ok(wire) => ws
                    .send(Message::Text(serde_json::to_string(&wire).map_err(|_| ())?.into()))
                    .map_err(|_| ())?,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        let read = ws.read();
        if read.is_ok() {
            heard = Instant::now();
        }
        match read {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_link_is_pinged_and_a_silent_one_is_dead() {
        let t = Instant::now();
        let s = Duration::from_secs;
        assert_eq!(beat(t, t, t), Beat::Wait);
        assert_eq!(beat(t + s(14), t, t), Beat::Wait);
        assert_eq!(beat(t + s(15), t, t), Beat::Ping);
        // The relay's pings keep it alive though this side's own are recent.
        assert_eq!(beat(t + s(44), t + s(30), t + s(30)), Beat::Wait);
        // Silence outlasts any ping this side sends.
        assert_eq!(beat(t + s(46), t, t + s(45)), Beat::Dead);
        assert_eq!(beat(t + s(46), t, t), Beat::Dead);
    }
}
