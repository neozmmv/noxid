//! The door in front of the media.
//!
//! iroh-live's MoQ handler accepts any peer, and the node origin announces
//! every broadcast it holds to every session. Without this gate, anyone who
//! learned a participant's endpoint id could listen to them. Every MoQ ALPN is
//! registered through here instead, and only current members get through.
//!
//! A member removed later is cut off by the mesh, which closes their sessions
//! when the roster drops them; this only decides who gets in.

use std::sync::Arc;

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh_live::moq::MoqProtocolHandler;
use tracing::debug;

use crate::node::Inner;

/// Close code for a media connection from someone not in the room.
pub const CLOSE_NOT_MEMBER: u32 = 1;

#[derive(Clone)]
pub(crate) struct MemberGate {
    pub inner: Arc<Inner>,
    pub moq: MoqProtocolHandler,
}

impl std::fmt::Debug for MemberGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberGate").finish_non_exhaustive()
    }
}

impl ProtocolHandler for MemberGate {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer = connection.remote_id();
        let allowed = {
            let own_id = self.inner.own_id();
            let room = self.inner.room.lock().unwrap();
            peer != own_id && room.is_member(&peer.to_string())
        };
        if !allowed {
            debug!(%peer, "refused media connection from a non-member");
            connection.close(CLOSE_NOT_MEMBER.into(), b"not a member");
            return Ok(());
        }
        self.moq.accept(connection).await
    }
}
