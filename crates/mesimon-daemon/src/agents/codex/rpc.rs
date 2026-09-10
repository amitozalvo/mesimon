//! Bounded local Codex app-server transport. Unix sockets carry WebSocket
//! frames, not Mesimon's newline-delimited board protocol.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use tungstenite::protocol::{Message, WebSocketConfig};

pub struct Client {
    socket: tungstenite::WebSocket<UnixStream>,
    next_id: u64,
}

impl Client {
    pub fn connect(path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path).context("connect Codex observer")?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(4 * 1024 * 1024))
            .max_frame_size(Some(4 * 1024 * 1024));
        let (socket, _) =
            tungstenite::client::client_with_config("ws://localhost/", stream, Some(config))
                .map_err(|e| anyhow!("Codex WebSocket handshake: {e}"))?;
        let mut client = Self { socket, next_id: 0 };
        client.call(
            "initialize",
            json!({"clientInfo": {"name": "mesimon", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": true}}),
            &mut |_| {},
        )?;
        client.send(json!({"method": "initialized", "params": {}}))?;
        Ok(client)
    }

    pub fn send(&mut self, value: Value) -> Result<()> {
        self.socket.send(Message::Text(serde_json::to_string(&value)?.into()))?;
        Ok(())
    }

    /// Requests from the server are returned to the observer, never answered.
    /// The native TUI exclusively owns user interactions.
    pub fn receive(&mut self, timeout: Duration) -> Result<Option<Value>> {
        self.socket.get_mut().set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        match self.socket.read() {
            Ok(Message::Text(text)) => Ok(Some(serde_json::from_str(&text)?)),
            Ok(Message::Close(_)) => bail!("Codex observer connection closed"),
            Ok(Message::Ping(_) | Message::Pong(_)) => {
                self.socket.flush()?;
                Ok(None)
            }
            Ok(_) => bail!("unexpected Codex WebSocket message"),
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Preserve notifications arriving between a request and its response.
    /// An RPC timeout invalidates this connection: callers reconnect and
    /// reconcile instead of trusting a late response as current evidence.
    pub fn call(
        &mut self,
        method: &str,
        params: Value,
        observe: &mut impl FnMut(Value),
    ) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let Some(value) = self.receive(Duration::from_millis(250))? else { continue };
            if value.get("method").is_none() && value.get("id") == Some(&json!(id)) {
                if let Some(error) = value.get("error") {
                    bail!("Codex {method}: {error}");
                }
                return value.get("result").cloned().context("Codex response has no result");
            }
            observe(value);
        }
        bail!("Codex {method} timed out")
    }
}
