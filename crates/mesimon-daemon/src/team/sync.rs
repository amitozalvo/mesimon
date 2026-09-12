//! The one thread that talks to the relay. It holds a client and a
//! credential, runs each job in order, and reports every result back to the
//! writer through `report`. It decides nothing: which jobs run, and what a
//! result means, is the writer's.
use mesimon_team::relay::{RelayClient, RelayEndpoint};
use mesimon_team::wire::{Credential, ErrorCode, Request, Response};
use std::sync::mpsc::{channel, Receiver, Sender};

/// What a job was for, so its result can be routed. `Put` carries the
/// operation id (hex) so the outbox entry can be found.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tag {
    SignIn,
    Share,
    Keys,
    Invite,
    Join,
    Revoke,
    Unshare,
    Leave,
    Boards,
    Members,
    Head,
    Sync,
    MyKeys,
    Put(String),
}

pub enum Job {
    /// (Re)build the client. `credential` is what later calls carry.
    Connect {
        endpoint: RelayEndpoint,
        credential: Option<Credential>,
    },
    Call {
        tag: Tag,
        request: Request,
    },
}

pub struct Done {
    pub tag: Tag,
    pub result: Result<Response, ErrorCode>,
}

/// Start the executor. `report` is called on the executor's thread with
/// every result; the caller forwards it to the writer as a message.
pub fn spawn(report: impl Fn(Done) + Send + 'static) -> Sender<Job> {
    let (tx, rx) = channel::<Job>();
    std::thread::spawn(move || run(rx, report));
    tx
}

fn run(rx: Receiver<Job>, report: impl Fn(Done)) {
    let mut client: Option<RelayClient> = None;
    let mut credential: Option<Credential> = None;
    for job in rx {
        match job {
            Job::Connect { endpoint, credential: c } => {
                client = RelayClient::new(endpoint).ok();
                credential = c;
            }
            Job::Call { tag, request } => {
                let result = match &client {
                    Some(client) => client.call(credential.as_ref(), request),
                    None => Err(ErrorCode::Unavailable),
                };
                report(Done { tag, result });
            }
        }
    }
}
