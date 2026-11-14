//! The control plane: joining, knocking, leaving and roster sync, on its own
//! ALPN beside the media.
//!
//! Ported from Quix's `admin.rs` and `connect.rs`: one JSON request and one JSON
//! response per bidirectional stream, authenticated by the connection itself
//! (iroh's handshake proves the peer's key). Additions: knocking, a room-closed
//! notice, and timeouts on every wait, since anyone can open this ALPN and
//! Quix's handler would wait on a silent peer forever.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointId};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::node::{Inner, Notice};
use crate::rpc;
use crate::room::{Code, Member};

pub const ADMIN_ALPN: &[u8] = b"noxid-admin/0";

/// Largest request or response accepted. A roster of a few members is a few
/// kilobytes.
const MAX_MESSAGE: usize = 64 * 1024;

/// How long a peer gets to send its request once connected.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a knock waits for the coordinator's answer before giving up.
pub const KNOCK_TIMEOUT: Duration = Duration::from_secs(120);

/// How long an ordinary call waits for its answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the coordinator re-sends the roster to everyone. Pushes are
/// fire-and-forget, so this is what catches up a member that missed one,
/// which in Quix waited until that member happened to link with the
/// coordinator again.
const ROSTER_HEARTBEAT: Duration = Duration::from_secs(20);

/// Attempts at reaching the coordinator with an invite. Safe to retry now that
/// redeeming is idempotent for the same requester.
const JOIN_ATTEMPTS: u32 = 3;
const JOIN_RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AdminRequest {
    /// Joiner to coordinator: redeem an invite token.
    Join {
        token_hex: String,
        name: String,
        /// Their picture's hash; older versions send none.
        #[serde(default)]
        avatar: Option<String>,
    },
    /// Joiner to coordinator: ask to be let in. The reply comes once the
    /// coordinator decides, or the knock times out.
    Knock {
        name: String,
        #[serde(default)]
        avatar: Option<String>,
    },
    /// Member to coordinator: take me off the roster.
    Leave,
    /// Coordinator to member: the roster changed, here it is.
    Roster {
        room_name: Option<String>,
        members: Vec<Member>,
    },
    /// Coordinator to member: you have been removed.
    Kicked,
    /// Coordinator to member: the room is over.
    Closed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AdminResponse {
    Joined {
        room_name: Option<String>,
        members: Vec<Member>,
    },
    Denied { message: String },
    Ack,
    Error { message: String },
}

fn error(message: impl Into<String>) -> AdminResponse {
    AdminResponse::Error {
        message: message.into(),
    }
}

#[derive(Clone)]
pub(crate) struct AdminHandler {
    pub inner: Arc<Inner>,
}

impl std::fmt::Debug for AdminHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminHandler").finish_non_exhaustive()
    }
}

impl ProtocolHandler for AdminHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let requester = connection.remote_id().to_string();
        let (send, response) = match rpc::read_request(&connection, MAX_MESSAGE, REQUEST_TIMEOUT).await? {
            rpc::Incoming::Request(send, request) => (send, self.dispatch(request, &requester).await),
            rpc::Incoming::Malformed(send, message) => (send, error(message)),
            rpc::Incoming::TimedOut => return Ok(()),
        };
        rpc::reply(&connection, send, &response).await
    }
}

impl AdminHandler {
    async fn dispatch(&self, request: AdminRequest, requester: &str) -> AdminResponse {
        match request {
            AdminRequest::Join {
                token_hex,
                name,
                avatar,
            } => self.join(&token_hex, &name, avatar.as_deref(), requester),
            AdminRequest::Knock { name, avatar } => self.knock(name, avatar, requester).await,
            AdminRequest::Leave => self.leave(requester),
            AdminRequest::Roster { room_name, members } => {
                self.roster(room_name, members, requester)
            }
            AdminRequest::Kicked => self.removed(requester, Notice::Kicked).await,
            AdminRequest::Closed => self.removed(requester, Notice::RoomClosed).await,
        }
    }

    fn is_coordinator(&self) -> bool {
        let own_id = self.inner.own_id().to_string();
        self.inner.room.lock().unwrap().is_coordinator(&own_id)
    }

    fn admitted(&self) -> AdminResponse {
        let room = self.inner.room.lock().unwrap();
        AdminResponse::Joined {
            room_name: room.name.clone(),
            members: room.roster(),
        }
    }

    fn join(&self, token_hex: &str, name: &str, avatar: Option<&str>, requester: &str) -> AdminResponse {
        let Ok(Ok(token)) = hex::decode(token_hex).map(<[u8; 16]>::try_from) else {
            return error("invalid token");
        };
        let admitted = self.is_coordinator() && {
            let mut room = self.inner.room.lock().unwrap();
            let admitted = room.redeem_invite(&token, requester, name, chrono::Utc::now());
            if admitted {
                room.set_avatar(requester, avatar);
            }
            admitted
        };
        if !admitted {
            // One answer for every way a code can fail.
            return AdminResponse::Denied {
                message: "invalid, expired, or already used invite".to_string(),
            };
        }
        info!(%requester, "admitted with an invite");
        self.inner.roster_changed(Some(requester));
        self.admitted()
    }

    async fn knock(&self, name: String, avatar: Option<String>, requester: &str) -> AdminResponse {
        if !self.is_coordinator() {
            return AdminResponse::Denied {
                message: "this room is not open".to_string(),
            };
        }

        // Someone already on the roster (back after a restart) comes straight in.
        let already_in = self.inner.room.lock().unwrap().is_member(requester);
        if already_in {
            {
                let mut room = self.inner.room.lock().unwrap();
                room.admit(requester, &name);
                room.set_avatar(requester, avatar.as_deref());
            }
            self.inner.roster_changed(Some(requester));
            return self.admitted();
        }

        let name = crate::room::display_name(&name, requester);
        let Some(answer) = self.inner.add_knock(requester, name.clone()) else {
            return AdminResponse::Denied {
                message: "too many people are waiting; try again shortly".to_string(),
            };
        };
        info!(%requester, %name, "knocking");

        let approved = match tokio::time::timeout(KNOCK_TIMEOUT, answer).await {
            Ok(Ok(approved)) => approved,
            // Replaced by a newer knock from the same node, or the room went away.
            Ok(Err(_)) => false,
            Err(_) => {
                self.inner.forget_knock(requester);
                return AdminResponse::Denied {
                    message: "nobody answered".to_string(),
                };
            }
        };

        // The room may have closed while they waited.
        if !approved || !self.is_coordinator() {
            return AdminResponse::Denied {
                message: "the host did not let you in".to_string(),
            };
        }
        {
            let mut room = self.inner.room.lock().unwrap();
            room.admit(requester, &name);
            room.set_avatar(requester, avatar.as_deref());
        }
        info!(%requester, %name, "let in");
        self.inner.roster_changed(Some(requester));
        self.admitted()
    }

    fn leave(&self, requester: &str) -> AdminResponse {
        if !self.is_coordinator() {
            return error("only the coordinator keeps the roster");
        }
        let removed = self.inner.room.lock().unwrap().remove_member(requester);
        if removed {
            info!(%requester, "left");
            self.inner.roster_changed(Some(requester));
        }
        AdminResponse::Ack
    }

    fn sent_by_coordinator(&self, requester: &str) -> bool {
        self.inner.room.lock().unwrap().coordinator_id.as_deref() == Some(requester)
    }

    fn roster(&self, room_name: Option<String>, members: Vec<Member>, requester: &str) -> AdminResponse {
        // Only the coordinator rewrites our roster; otherwise anyone could add
        // themselves to the room.
        if !self.sent_by_coordinator(requester) || self.is_coordinator() {
            return error("only the coordinator can push a roster");
        }
        let own_id = self.inner.own_id().to_string();
        let name = self.inner.name();
        let changed = {
            let mut room = self.inner.room.lock().unwrap();
            let before = room.roster();
            room.set_roster(&own_id, &name, room_name, members);
            // Ours stays ours, whatever the push says about it.
            room.set_avatar(&own_id, self.inner.own_avatar().as_deref());
            room.roster() != before
        };
        // The heartbeat re-sends an unchanged roster every few seconds; only an
        // actual change is worth waking the mesh and the frontend for.
        if changed {
            self.inner.roster_changed(None);
        }
        AdminResponse::Ack
    }

    async fn removed(&self, requester: &str, notice: Notice) -> AdminResponse {
        // Only the coordinator may say so, or any peer could end our call.
        if !self.sent_by_coordinator(requester) || self.is_coordinator() {
            return error("only the coordinator can remove a member");
        }
        warn!(?notice, "removed from the room");
        self.inner.leave_local().await;
        self.inner.notice(notice);
        AdminResponse::Ack
    }
}

/// Coordinator: sends the current roster to every member but us and `subject`.
pub(crate) fn broadcast_roster(inner: &Inner, subject: Option<&str>) {
    let (room_name, members) = {
        let room = inner.room.lock().unwrap();
        (room.name.clone(), room.roster())
    };
    let own_id = inner.own_id().to_string();
    for member in &members {
        if member.id == own_id || Some(member.id.as_str()) == subject {
            continue;
        }
        let Ok(to) = member.id.parse::<EndpointId>() else {
            continue;
        };
        let endpoint = inner.endpoint.clone();
        let request = AdminRequest::Roster {
            room_name: room_name.clone(),
            members: members.clone(),
        };
        tokio::spawn(async move {
            if let Err(e) = expect_ack(call(&endpoint, to, request, CALL_TIMEOUT).await) {
                // The heartbeat will try again; a member that is offline is
                // the common case and not worth more than debug.
                tracing::debug!(%to, "roster push failed: {e:#}");
            }
        });
    }
}

/// Coordinator: re-sends the roster periodically while a room is open.
pub(crate) async fn roster_heartbeat(inner: Arc<Inner>) {
    let mut tick = tokio::time::interval(ROSTER_HEARTBEAT);
    loop {
        tick.tick().await;
        let own_id = inner.own_id().to_string();
        if inner.room.lock().unwrap().is_coordinator(&own_id) {
            broadcast_roster(&inner, None);
        }
    }
}

/// What a successful join returned.
pub(crate) struct Admission {
    pub room_name: Option<String>,
    pub members: Vec<Member>,
}

/// Joins with an invite, or knocks and waits for the coordinator's answer.
pub(crate) async fn join(
    endpoint: &Endpoint,
    code: Code,
    name: String,
    avatar: Option<String>,
) -> Result<Admission> {
    let response = match code {
        Code::Knock { coordinator } => {
            // A little longer than the coordinator waits, so its "nobody
            // answered" arrives rather than our own timeout.
            call(
                endpoint,
                coordinator,
                AdminRequest::Knock { name, avatar },
                KNOCK_TIMEOUT + Duration::from_secs(10),
            )
            .await
            .context("could not reach the room's host; is it online?")?
        }
        Code::Invite { coordinator, token } => {
            let mut last_error = None;
            let mut response = None;
            for attempt in 0..JOIN_ATTEMPTS {
                if attempt > 0 {
                    tokio::time::sleep(JOIN_RETRY_DELAY).await;
                }
                let request = AdminRequest::Join {
                    token_hex: hex::encode(token),
                    name: name.clone(),
                    avatar: avatar.clone(),
                };
                match call(endpoint, coordinator, request, CALL_TIMEOUT).await {
                    Ok(r) => {
                        response = Some(r);
                        break;
                    }
                    Err(e) => last_error = Some(e),
                }
            }
            match response {
                Some(r) => r,
                None => {
                    return Err(last_error
                        .unwrap_or_else(|| anyhow::anyhow!("no response"))
                        .context("could not reach the room's host; is it online?"));
                }
            }
        }
    };

    match response {
        AdminResponse::Joined { room_name, members } => Ok(Admission { room_name, members }),
        AdminResponse::Denied { message } | AdminResponse::Error { message } => {
            anyhow::bail!(message)
        }
        other => anyhow::bail!("unexpected response to join: {other:?}"),
    }
}

pub(crate) async fn notify_leave(endpoint: &Endpoint, coordinator: EndpointId) -> Result<()> {
    expect_ack(call(endpoint, coordinator, AdminRequest::Leave, CALL_TIMEOUT).await)
}

/// How long the coordinator spends telling a removed member. The removal has
/// already happened, and the member may well be offline.
const NOTICE_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn notify_kicked(endpoint: &Endpoint, member: EndpointId) -> Result<()> {
    expect_ack(call(endpoint, member, AdminRequest::Kicked, NOTICE_TIMEOUT).await)
}

pub(crate) async fn notify_closed(endpoint: Endpoint, member: EndpointId) {
    if let Err(e) = expect_ack(call(&endpoint, member, AdminRequest::Closed, NOTICE_TIMEOUT).await) {
        tracing::debug!(%member, "close notice failed: {e:#}");
    }
}

fn expect_ack(response: Result<AdminResponse>) -> Result<()> {
    match response? {
        AdminResponse::Ack => Ok(()),
        AdminResponse::Error { message } | AdminResponse::Denied { message } => {
            anyhow::bail!(message)
        }
        other => anyhow::bail!("unexpected response: {other:?}"),
    }
}

/// One request/response exchange on its own connection, bounded by `timeout`.
async fn call(
    endpoint: &Endpoint,
    to: EndpointId,
    request: AdminRequest,
    timeout: Duration,
) -> Result<AdminResponse> {
    rpc::call(endpoint, to, ADMIN_ALPN, &request, MAX_MESSAGE, timeout).await
}
