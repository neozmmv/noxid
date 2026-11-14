//! What a frontend shows of the friends list, in one serializable value.
//! Times are unix milliseconds.

use serde::Serialize;

/// Whether a friend can be reached right now, as far as we know. Never
/// guessed: `Unknown` until we have tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Online,
    Offline,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct FriendsView {
    /// Friends, and those we said yes to that are not confirmed yet
    /// (`finishing`).
    pub friends: Vec<FriendView>,
    /// Requests waiting for our answer.
    pub incoming: Vec<IncomingView>,
    /// Our requests waiting for theirs.
    pub outgoing: Vec<OutgoingView>,
    /// Friends ringing us now.
    pub rings: Vec<RingView>,
    /// The friend we are ringing.
    pub calling: Option<CallingView>,
    pub missed: Vec<MissedView>,
    /// Why the list is read-only, if it is.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FriendView {
    pub id: String,
    /// What they call themselves; a label, not proof of anything.
    pub name: String,
    pub alias: Option<String>,
    pub presence: Presence,
    /// When we last reached them or heard from them.
    pub last_seen: Option<i64>,
    /// We said yes and are waiting for them to confirm.
    pub finishing: bool,
    /// They answered our ping without knowing of the friendship.
    pub unaware: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IncomingView {
    pub id: String,
    /// The name sent with the request.
    pub name: String,
    pub received_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingView {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    /// When they confirmed they have it.
    pub delivered_at: Option<i64>,
    /// Not delivered for so long that it is no longer retried on its own.
    pub stalled: bool,
    /// Why the last attempt failed.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RingView {
    pub id: String,
    pub name: String,
    pub room_name: Option<String>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallingView {
    pub id: String,
    pub name: String,
    pub since: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissedView {
    pub id: String,
    pub name: String,
    pub at: i64,
}
