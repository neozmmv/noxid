//! What goes over `noxid-friends/0`: one JSON request and one JSON reply per
//! connection, as on the admin ALPN. Nothing here names the sender; the
//! sender is always the connection's authenticated remote id.

use serde::{Deserialize, Serialize};

pub const FRIENDS_ALPN: &[u8] = b"noxid-friends/0";

/// Largest request or reply accepted. A name and a code fit in a few hundred
/// bytes.
pub const MAX_MESSAGE: usize = 4 * 1024;

/// Names one friend request, and once it is accepted, the friendship it
/// became. Random, so a message about an old request or friendship can
/// never match a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub [u8; 16]);

impl RequestId {
    pub fn random() -> Self {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("failed to get random bytes");
        Self(bytes)
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl std::str::FromStr for RequestId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s)?;
        let array: [u8; 16] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("a request id is 16 bytes"))?;
        Ok(Self(array))
    }
}

impl Serialize for RequestId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Why a request or friendship ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    Declined,
    Canceled,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FriendMsg {
    /// Asks to be friends. `name` is a label for the request, nothing more.
    Request {
        id: RequestId,
        #[serde(default)]
        name: String,
    },
    /// Says yes to the request `id`. `name` is the accepter's label, so the
    /// requester has something better to show than an id.
    Accept {
        id: RequestId,
        #[serde(default)]
        name: String,
    },
    /// Says no to the request `id`.
    Decline { id: RequestId },
    /// Withdraws our request `id`.
    Cancel { id: RequestId },
    /// Ends the friendship `id`.
    Remove { id: RequestId },
    /// Presence and reconciliation between friends.
    Ping {
        id: RequestId,
        #[serde(default)]
        name: String,
    },
    /// Rings a friend into our room. Held open until they answer, decline, or
    /// the ring times out; never stored.
    Ring {
        call_id: RequestId,
        code: String,
        #[serde(default)]
        room_name: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FriendReply {
    /// The request is stored on their side, waiting for an answer.
    Received,
    /// They had already consented (crossing requests, or they still count us
    /// as a friend): the friendship is active, with this id.
    Accepted { id: RequestId },
    /// Applied, or already applied.
    Ack,
    Pong {
        #[serde(default)]
        name: String,
    },
    /// That id is closed on their side.
    Gone { reason: CloseReason },
    /// They have no record matching that id.
    Unknown,
    /// Not now: rate limited, too many pending, or their list is unavailable.
    Busy,
    /// Ring outcomes.
    Answered,
    Declined,
    NoAnswer,
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_go_over_the_wire_as_hex() {
        let id = RequestId([0xab; 16]);
        let json = serde_json::to_string(&FriendMsg::Cancel { id }).unwrap();
        assert_eq!(json, format!(r#"{{"op":"cancel","id":"{}"}}"#, "ab".repeat(16)));
        assert_eq!(serde_json::from_str::<FriendMsg>(&json).unwrap(), FriendMsg::Cancel { id });
    }

    #[test]
    fn a_request_id_of_the_wrong_length_is_refused() {
        assert!(serde_json::from_str::<FriendMsg>(r#"{"op":"accept","id":"abcd"}"#).is_err());
    }

    #[test]
    fn a_request_without_a_name_still_parses() {
        let json = format!(r#"{{"op":"request","id":"{}"}}"#, "01".repeat(16));
        let msg: FriendMsg = serde_json::from_str(&json).unwrap();
        assert!(matches!(msg, FriendMsg::Request { name, .. } if name.is_empty()));
    }
}
