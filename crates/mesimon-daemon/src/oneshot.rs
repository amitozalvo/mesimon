//! One request to a daemon over its `orch.sock`: connect, one line out, one
//! line back. The TUI's board scan, `doctor`, the `mesimon mcp` shim and a
//! linked board's daemon (T-733) each spelled this for themselves; the
//! words each says about a failure stay its own, so `Fault` says where.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use mesimon_core::command::{Envelope, Response};

/// Where a one-shot call failed.
#[derive(Debug)]
pub enum Fault {
    /// No connection: nobody listening, or the socket could not be set up.
    Connect(io::Error),
    /// The request did not go.
    Send(io::Error),
    /// No answer in time.
    Receive(io::Error),
    /// The daemon closed the connection with no line.
    Closed,
    /// A line this build cannot read.
    Unreadable(serde_json::Error),
}

/// Send `env` to the daemon at `sock` and read its one line back. A `None`
/// timeout waits as the socket does by default.
pub fn one_shot(
    sock: &Path,
    env: &Envelope,
    write: Option<Duration>,
    read: Option<Duration>,
) -> Result<Response, Fault> {
    exchange(sock, env, write, read)
}

/// MCP states the response contract on every call, including --call's
/// short-lived process. All policy remains at the daemon's dispatch.
pub fn one_shot_mcp(
    sock: &Path,
    env: &Envelope,
    read: Option<Duration>,
) -> Result<Response, Fault> {
    let request = mesimon_core::command::WireRequest {
        envelope: env,
        mcp_protocol: mesimon_core::command::MCP_PROTOCOL,
    };
    exchange(sock, &request, None, read)
}

fn exchange(
    sock: &Path,
    env: &impl serde::Serialize,
    write: Option<Duration>,
    read: Option<Duration>,
) -> Result<Response, Fault> {
    let stream = UnixStream::connect(sock).map_err(Fault::Connect)?;
    if write.is_some() {
        stream.set_write_timeout(write).map_err(Fault::Connect)?;
    }
    if read.is_some() {
        stream.set_read_timeout(read).map_err(Fault::Connect)?;
    }
    let json = serde_json::to_string(env).map_err(|e| Fault::Send(io::Error::other(e)))?;
    writeln!(&mut &stream, "{json}").map_err(Fault::Send)?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(Fault::Receive)?;
    if line.trim().is_empty() {
        return Err(Fault::Closed);
    }
    serde_json::from_str(&line).map_err(Fault::Unreadable)
}
