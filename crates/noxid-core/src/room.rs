//! Who is in the room: the coordinator's admission rules and every member's
//! copy of the roster.
//!
//! Ported from Quix's `membership.rs`, keeping its model:
//!
//! - The node that creates a room is its coordinator. It alone admits members
//!   and pushes the roster to everyone else.
//! - An invite is single-use and expires. The code carries the coordinator's id,
//!   its home relay and a random token, nothing else; which room the token is
//!   good for and until when are recorded only on the coordinator, which is
//!   the only party that checks them.
//! - `room_id` names one room apart from the next even when the same node
//!   creates both, so a token from an earlier room never admits anyone to a
//!   later one. It never goes on the wire.
//!
//! What changed from Quix:
//!
//! - Hostnames, DNS bindings and the name-pinning rules are gone. A member has a
//!   display name, which is a label for humans and grants nothing.
//! - A second way in: knocking. A room code (the coordinator's id and relay) lets
//!   someone ask to join, and the coordinator approves or denies them live.
//! - Redeeming an invite is idempotent for the same requester while the invite's
//!   window is open. In Quix a retried join whose first reply was lost found its
//!   own token already spent and was refused, though it had been admitted.
//! - Nothing is persisted. A room lives as long as its coordinator's app; a
//!   member that restarts knocks again and, still being on the roster, is let
//!   straight back in.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use iroh::{EndpointId, RelayUrl};
use serde::{Deserialize, Serialize};

/// How long an invite stays redeemable when nothing else is asked for.
pub const DEFAULT_INVITE_TTL: Duration = Duration::minutes(30);

/// The longest window an invite may be minted for.
pub const MAX_INVITE_TTL: Duration = Duration::days(7);

/// The longest display name, in characters.
pub const MAX_NAME_LEN: usize = 32;

/// One participant in the roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    /// What the participant asked to be called. Not unique and not trusted for
    /// anything: identity is the endpoint id.
    #[serde(default)]
    pub name: String,
    /// The hash of their profile picture, which they serve themselves (see
    /// `avatar`). Left out by versions without pictures, so it defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// Their home relay as they last told the coordinator, so members can
    /// dial each other without looking it up. Left out by older versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
}

impl Member {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            avatar: None,
            relay: None,
        }
    }
}

/// Cleans up a requested display name: trims it, drops control characters and
/// caps its length. Empty becomes the endpoint's short id, so every member has
/// something to show.
pub fn display_name(requested: &str, id: &str) -> String {
    let cleaned: String = requested
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_LEN)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        short_id(id)
    } else {
        cleaned
    }
}

/// The first ten characters of an endpoint id, which is how iroh abbreviates
/// them in its own logs.
pub fn short_id(id: &str) -> String {
    id.chars().take(10).collect()
}

/// What a code someone pasted turned out to be.
///
/// Two formats are read:
///
/// - Version 0, told apart by length: an invite is the coordinator's 32-byte
///   id followed by a 16-byte token, and a room code is the id alone. Nothing
///   says where the coordinator is, so joining depends on looking it up.
/// - Version 1, which starts with its version byte and also carries the
///   coordinator's home relay, so a joiner can dial it without any lookup.
///   Direct addresses are left out: they say where someone's PC is, and go
///   stale long before a code does.
///
/// Version 1 is laid out as
/// `[1] [kind] [id: 32] [relay tag] [relay..] [token: 16, invites only] [pad]`.
/// The relay is nothing (tag 0), one of n0's by its index in [`N0_RELAYS`]
/// (tag 1, one byte), or any other URL (tag 2, a length byte, the URL). One
/// zero byte is padded on in the single case where the code would otherwise
/// be 48 bytes long and read as a version 0 invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Code {
    /// A single-use invite: admitted without asking anyone.
    Invite {
        coordinator: EndpointId,
        relay: Option<RelayUrl>,
        token: [u8; 16],
    },
    /// A room code: knock, and wait for the coordinator to let us in.
    Knock {
        coordinator: EndpointId,
        relay: Option<RelayUrl>,
    },
}

/// The version byte of codes that carry the coordinator's relay.
const CODE_V1: u8 = 1;
const KIND_KNOCK: u8 = 0;
const KIND_INVITE: u8 = 1;
const RELAY_NONE: u8 = 0;
const RELAY_N0: u8 = 1;
const RELAY_URL: u8 = 2;
/// The shortest version 1 code: a knock with no relay.
const MIN_V1_LEN: usize = 1 + 1 + 32 + 1;
/// The lengths of version 0 codes, which a version 1 code never has.
const V0_INVITE_LEN: usize = 48;
const V0_KNOCK_LEN: usize = 32;

const UPDATE_NOXID: &str = "this code was made by a newer version of Noxid; update Noxid to use it";

/// n0's relays, which a code names by index to stay short. Part of the code
/// format: entries are only ever appended, never reordered or removed. A relay
/// missing here still works; it is just written out in full.
pub const N0_RELAYS: [&str; 4] = [
    "https://use1-1.relay.n0.iroh.link.",
    "https://usw1-1.relay.n0.iroh.link.",
    "https://euc1-1.relay.n0.iroh.link.",
    "https://aps1-1.relay.n0.iroh.link.",
];

fn n0_relay(index: u8) -> Option<RelayUrl> {
    N0_RELAYS.get(usize::from(index))?.parse().ok()
}

fn n0_index(relay: &RelayUrl) -> Option<u8> {
    N0_RELAYS
        .iter()
        .position(|url| url.parse::<RelayUrl>().is_ok_and(|url| url == *relay))
        .and_then(|i| u8::try_from(i).ok())
}

/// Reads a version 1 code front to back.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        anyhow::ensure!(self.0.len() >= n, "not a valid code (cut short)");
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
}

impl Code {
    /// Decodes either kind, in either format. Version 0 codes are recognized
    /// by their length alone, which no version 1 code has.
    pub fn decode(code: &str) -> Result<Self> {
        let bytes = bs58::decode(code.trim())
            .into_vec()
            .context("not a valid code")?;
        match bytes.len() {
            V0_INVITE_LEN => Ok(Code::Invite {
                coordinator: endpoint_id(&bytes[..32])?,
                relay: None,
                token: bytes[32..].try_into().expect("length checked"),
            }),
            V0_KNOCK_LEN => Ok(Code::Knock {
                coordinator: endpoint_id(&bytes)?,
                relay: None,
            }),
            n if n < MIN_V1_LEN => anyhow::bail!("not a valid code (wrong length)"),
            _ if bytes[0] == CODE_V1 => Self::decode_v1(&bytes),
            _ => anyhow::bail!(UPDATE_NOXID),
        }
    }

    fn decode_v1(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader(&bytes[1..]);
        let kind = reader.byte()?;
        if kind != KIND_KNOCK && kind != KIND_INVITE {
            anyhow::bail!(UPDATE_NOXID);
        }
        let coordinator = endpoint_id(reader.take(32)?)?;
        let relay = match reader.byte()? {
            RELAY_NONE => None,
            // An index added in a later version: dial by id alone, as before.
            RELAY_N0 => n0_relay(reader.byte()?),
            RELAY_URL => {
                let len = usize::from(reader.byte()?);
                let text = std::str::from_utf8(reader.take(len)?).context("not a valid code (relay)")?;
                Some(text.parse().context("not a valid code (relay)")?)
            }
            _ => anyhow::bail!(UPDATE_NOXID),
        };
        let code = if kind == KIND_INVITE {
            Code::Invite {
                coordinator,
                relay,
                token: reader.take(16)?.try_into().expect("length checked"),
            }
        } else {
            Code::Knock { coordinator, relay }
        };
        // The one zero byte `encode` adds to keep clear of the version 0 length.
        let padded = bytes.len() == V0_INVITE_LEN + 1 && reader.0 == [0];
        anyhow::ensure!(reader.0.is_empty() || padded, "not a valid code (trailing bytes)");
        Ok(code)
    }

    /// Writes the code out in the current format, version 1.
    pub fn encode(&self) -> String {
        let (kind, coordinator, relay, token) = match self {
            Code::Invite {
                coordinator,
                relay,
                token,
            } => (KIND_INVITE, coordinator, relay, Some(token)),
            Code::Knock { coordinator, relay } => (KIND_KNOCK, coordinator, relay, None),
        };
        let mut blob = Vec::with_capacity(64);
        blob.extend_from_slice(&[CODE_V1, kind]);
        blob.extend_from_slice(coordinator.as_bytes());
        let url = relay.as_ref().map(ToString::to_string);
        match (relay.as_ref().and_then(n0_index), url) {
            (Some(index), _) => blob.extend_from_slice(&[RELAY_N0, index]),
            (None, Some(url)) => match u8::try_from(url.len()) {
                Ok(len) => {
                    blob.extend_from_slice(&[RELAY_URL, len]);
                    blob.extend_from_slice(url.as_bytes());
                }
                Err(_) => {
                    tracing::warn!(%url, "relay URL too long for a code; leaving it out");
                    blob.push(RELAY_NONE);
                }
            },
            (None, None) => blob.push(RELAY_NONE),
        }
        if let Some(token) = token {
            blob.extend_from_slice(token);
        }
        // Read back as a version 0 invite otherwise; see `decode_v1`.
        if blob.len() == V0_INVITE_LEN {
            blob.push(0);
        }
        bs58::encode(blob).into_string()
    }

    pub fn coordinator(&self) -> EndpointId {
        match self {
            Code::Invite { coordinator, .. } | Code::Knock { coordinator, .. } => *coordinator,
        }
    }

    /// The coordinator's home relay when the code was made, if it says.
    pub fn relay(&self) -> Option<&RelayUrl> {
        match self {
            Code::Invite { relay, .. } | Code::Knock { relay, .. } => relay.as_ref(),
        }
    }
}

fn endpoint_id(bytes: &[u8]) -> Result<EndpointId> {
    let array: [u8; 32] = bytes.try_into().expect("length checked");
    EndpointId::from_bytes(&array).context("code names an invalid endpoint")
}

/// The code anyone can use to knock on a room this node coordinates, naming
/// its home relay when it has one.
pub fn room_code(coordinator: &EndpointId, relay: Option<&RelayUrl>) -> String {
    Code::Knock {
        coordinator: *coordinator,
        relay: relay.cloned(),
    }
    .encode()
}

/// Reads a relay URL from a roster or a request. Anything but a well-formed
/// https URL counts as none, so a bad entry never sends anyone anywhere odd.
pub fn parse_relay(text: &str) -> Option<RelayUrl> {
    crate::relay::parse(text).ok()
}

/// An outstanding invite, and the two things that decide whether it still counts.
#[derive(Debug, Clone)]
struct PendingInvite {
    room_id: String,
    expires_at: DateTime<Utc>,
}

/// An invite that was redeemed, kept until its window closes so the same
/// requester retrying gets the same answer.
#[derive(Debug, Clone)]
struct Redeemed {
    room_id: String,
    requester: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct Room {
    pub name: Option<String>,
    pub coordinator_id: Option<String>,
    /// Set only on the coordinator; see the module docs.
    room_id: Option<String>,
    /// In joining order: the host first, then everyone in the order they were
    /// let in. Every member's copy is in the host's order, so all agree on who
    /// is next to host (see [`Room::next_coordinator`]).
    members: Vec<Member>,
    /// The host before the current one, after a hand-off, so the same
    /// hand-off arriving again is recognized.
    previous_coordinator: Option<String>,
    pending_invites: HashMap<[u8; 16], PendingInvite>,
    redeemed: HashMap<[u8; 16], Redeemed>,
}

fn new_room_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("failed to get random bytes");
    hex::encode(bytes)
}

impl Room {
    /// Whether this node is in a room at all.
    pub fn is_active(&self) -> bool {
        self.coordinator_id.is_some()
    }

    pub fn create(&mut self, name: Option<String>, own_id: &str, own_name: &str) {
        self.leave();
        self.name = name;
        self.coordinator_id = Some(own_id.to_string());
        self.room_id = Some(new_room_id());
        self.members = vec![Member::new(own_id, own_name)];
    }

    /// Mints a one-time invite, returning the shareable code and the moment it
    /// stops being redeemable. `ttl` is clamped to [`MAX_INVITE_TTL`].
    pub fn generate_invite(
        &mut self,
        own_id: EndpointId,
        own_relay: Option<&RelayUrl>,
        ttl: Duration,
        now: DateTime<Utc>,
    ) -> Result<(String, DateTime<Utc>)> {
        let room_id = self
            .room_id
            .clone()
            .context("only the room's coordinator can invite")?;
        self.prune_expired(now);

        let mut token = [0u8; 16];
        getrandom::fill(&mut token).expect("failed to get random bytes");
        let expires_at = now + ttl.min(MAX_INVITE_TTL);
        self.pending_invites
            .insert(token, PendingInvite { room_id, expires_at });

        let code = Code::Invite {
            coordinator: own_id,
            relay: own_relay.cloned(),
            token,
        };
        Ok((code.encode(), expires_at))
    }

    fn prune_expired(&mut self, now: DateTime<Utc>) {
        self.pending_invites.retain(|_, i| i.expires_at > now);
        self.redeemed.retain(|_, r| r.expires_at > now);
    }

    /// Spends an invite and admits the bearer, if it is still good for anything.
    ///
    /// Unknown, spent by someone else, expired, or minted for a different room
    /// all give the same answer: a joiner learns only that the code did not
    /// work. The same requester presenting a token it already redeemed, while
    /// the window is open, is admitted again; that is a retry, not a reuse.
    pub fn redeem_invite(
        &mut self,
        token: &[u8; 16],
        requester: &str,
        name: &str,
        now: DateTime<Utc>,
    ) -> bool {
        let Some(room_id) = self.room_id.clone() else {
            return false;
        };

        if let Some(redeemed) = self.redeemed.get(token) {
            let retry = redeemed.requester == requester
                && redeemed.room_id == room_id
                && redeemed.expires_at > now;
            if retry {
                self.admit(requester, name);
            }
            return retry;
        }

        // Removed before it is judged: a token presented once is spent whatever
        // the verdict.
        let Some(invite) = self.pending_invites.remove(token) else {
            return false;
        };
        self.prune_expired(now);
        if invite.room_id != room_id || invite.expires_at <= now {
            return false;
        }

        self.redeemed.insert(
            *token,
            Redeemed {
                room_id,
                requester: requester.to_string(),
                expires_at: invite.expires_at,
            },
        );
        self.admit(requester, name);
        true
    }

    /// Adds a member, or refreshes the name of one already listed.
    pub fn admit(&mut self, id: &str, name: &str) {
        let name = display_name(name, id);
        match self.members.iter_mut().find(|m| m.id == id) {
            Some(member) => member.name = name,
            None => self.members.push(Member::new(id, name)),
        }
    }

    /// Sets a listed member's picture. Anything but a well-formed hash counts
    /// as no picture, so a bad roster entry can never name a path or a URL.
    pub fn set_avatar(&mut self, id: &str, avatar: Option<&str>) {
        if let Some(member) = self.members.iter_mut().find(|m| m.id == id) {
            member.avatar = avatar.filter(|a| crate::avatar::is_hash(a)).map(str::to_string);
        }
    }

    /// Sets a listed member's home relay. Anything but a well-formed https
    /// URL counts as none.
    pub fn set_relay(&mut self, id: &str, relay: Option<&str>) {
        if let Some(member) = self.members.iter_mut().find(|m| m.id == id) {
            member.relay = relay.and_then(parse_relay).map(|url| url.to_string());
        }
    }

    /// Every member's home relay that we know, as endpoint ids.
    pub fn relays(&self) -> Vec<(EndpointId, RelayUrl)> {
        self.members
            .iter()
            .filter_map(|m| Some((m.id.parse().ok()?, parse_relay(m.relay.as_deref()?)?)))
            .collect()
    }

    /// Records the room we were admitted to, with the roster the coordinator
    /// handed back. Without that roster a joiner would only trust the
    /// coordinator, and members would reject each other.
    pub fn set_joined(
        &mut self,
        coordinator_id: &str,
        own_id: &str,
        own_name: &str,
        name: Option<String>,
        roster: Vec<Member>,
    ) {
        self.leave();
        self.coordinator_id = Some(coordinator_id.to_string());
        self.name = name;
        self.apply_roster(own_id, own_name, roster);
    }

    /// Applies a roster pushed by the coordinator.
    pub fn set_roster(&mut self, own_id: &str, own_name: &str, name: Option<String>, roster: Vec<Member>) {
        if name.is_some() {
            self.name = name;
        }
        self.apply_roster(own_id, own_name, roster);
    }

    fn apply_roster(&mut self, own_id: &str, own_name: &str, roster: Vec<Member>) {
        self.members.clear();
        for member in roster {
            if !self.is_member(&member.id) {
                let name = display_name(&member.name, &member.id);
                let avatar = member.avatar.filter(|a| crate::avatar::is_hash(a));
                let relay = member.relay.as_deref().and_then(parse_relay).map(|url| url.to_string());
                self.members.push(Member {
                    avatar,
                    relay,
                    ..Member::new(member.id, name)
                });
            }
        }
        // We and the coordinator stay in our own roster whatever arrives, so a
        // malformed push cannot evict us from our own room.
        if !self.is_member(own_id) {
            self.members.push(Member::new(own_id, own_name));
        }
        if let Some(coordinator) = self.coordinator_id.clone()
            && !self.is_member(&coordinator)
        {
            let name = short_id(&coordinator);
            self.members.push(Member::new(coordinator, name));
        }
    }

    /// Forgets the room entirely, invites included.
    pub fn leave(&mut self) {
        *self = Self::default();
    }

    /// Coordinator side of someone leaving. Returns whether they were listed.
    pub fn remove_member(&mut self, id: &str) -> bool {
        let before = self.members.len();
        self.members.retain(|m| m.id != id);
        before != self.members.len()
    }

    /// Finds the member a human means: the full id, its short form, or the
    /// display name (case-insensitive). A name shared by two members finds
    /// neither, rather than guessing.
    pub fn find_member(&self, query: &str) -> Option<&Member> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        if let Some(m) = self
            .members
            .iter()
            .find(|m| m.id == query || short_id(&m.id) == query)
        {
            return Some(m);
        }
        let mut by_name = self
            .members
            .iter()
            .filter(|m| m.name.eq_ignore_ascii_case(query));
        match (by_name.next(), by_name.next()) {
            (Some(m), None) => Some(m),
            _ => None,
        }
    }

    /// Coordinator side of removing someone who did not ask to go.
    pub fn kick(&mut self, query: &str, own_id: &str) -> Result<Member, String> {
        let member = self
            .find_member(query)
            .cloned()
            .ok_or_else(|| format!("{} is not in this room", query.trim()))?;
        if member.id == own_id {
            return Err("that is you; to go, leave the room".to_string());
        }
        self.remove_member(&member.id);
        Ok(member)
    }

    pub fn is_coordinator(&self, own_id: &str) -> bool {
        self.room_id.is_some() && self.coordinator_id.as_deref() == Some(own_id)
    }

    pub fn is_member(&self, id: &str) -> bool {
        self.members.iter().any(|m| m.id == id)
    }

    /// The roster in joining order, which every member sees the same way.
    pub fn roster(&self) -> Vec<Member> {
        self.members.clone()
    }

    /// Who hosts if the host leaves: the first to have joined after it.
    pub fn next_coordinator(&self) -> Option<String> {
        let coordinator = self.coordinator_id.as_deref()?;
        self.members
            .iter()
            .find(|m| m.id != coordinator)
            .map(|m| m.id.clone())
    }

    /// Takes over the room the host handed us: we admit people from now on,
    /// under a new `room_id`, so nothing the old host minted lets anyone in.
    pub fn promote(&mut self, own_id: &str, own_name: &str, name: Option<String>, roster: Vec<Member>) {
        self.previous_coordinator = self.coordinator_id.take();
        self.coordinator_id = Some(own_id.to_string());
        self.room_id = Some(new_room_id());
        self.pending_invites.clear();
        self.redeemed.clear();
        if name.is_some() {
            self.name = name;
        }
        self.apply_roster(own_id, own_name, roster);
    }

    /// Follows the room to its new host, `successor`.
    pub fn hand_over(&mut self, successor: &str, own_id: &str, own_name: &str, name: Option<String>, roster: Vec<Member>) {
        self.previous_coordinator = self.coordinator_id.replace(successor.to_string());
        if name.is_some() {
            self.name = name;
        }
        self.apply_roster(own_id, own_name, roster);
    }

    /// Whether a hand-off from `requester` to `successor` is one to follow:
    /// it comes from the host, or it is the hand-off we already followed,
    /// arriving again.
    pub fn accepts_hand_over(&self, requester: &str, successor: &str) -> bool {
        let from_host = self.coordinator_id.as_deref() == Some(requester);
        let again = self.previous_coordinator.as_deref() == Some(requester)
            && self.coordinator_id.as_deref() == Some(successor);
        from_host || again
    }

    /// The roster as endpoint ids, skipping any entry that does not parse.
    pub fn member_ids(&self) -> Vec<EndpointId> {
        self.members.iter().filter_map(|m| m.id.parse().ok()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    fn id(n: u8) -> String {
        key(n).to_string()
    }

    /// Fixed, because expiry is under test and the clock has to be an input.
    fn minted_at() -> DateTime<Utc> {
        "2000-01-01T00:00:00Z".parse().unwrap()
    }

    fn token_of(code: &str) -> [u8; 16] {
        match Code::decode(code).unwrap() {
            Code::Invite { token, .. } => token,
            other => panic!("not an invite: {other:?}"),
        }
    }

    fn with_invite(ttl: Duration) -> (Room, [u8; 16]) {
        let mut room = Room::default();
        room.create(Some("room".into()), &id(1), "host");
        let (code, _) = room.generate_invite(key(1), None, ttl, minted_at()).unwrap();
        (room, token_of(&code))
    }

    #[test]
    fn invite_code_round_trips() {
        let mut room = Room::default();
        room.create(None, &id(1), "host");
        let (code, expires_at) = room
            .generate_invite(key(1), None, Duration::minutes(5), minted_at())
            .unwrap();

        let decoded = Code::decode(&code).unwrap();
        assert_eq!(decoded.coordinator(), key(1));
        assert_eq!(expires_at, minted_at() + Duration::minutes(5));
        assert!(room.redeem_invite(&token_of(&code), &id(2), "ana", minted_at()));
        assert!(room.is_member(&id(2)));
    }

    #[test]
    fn room_codes_and_invites_are_told_apart() {
        assert_eq!(
            Code::decode(&room_code(&key(1), None)).unwrap(),
            Code::Knock {
                coordinator: key(1),
                relay: None
            }
        );
        assert!(Code::decode("not-base58!").is_err());
        assert!(Code::decode("abc").is_err(), "too short");
    }

    // ------------------------------------------------------------ code format

    fn relay(url: &str) -> RelayUrl {
        url.parse().unwrap()
    }

    fn raw(code: &str) -> Vec<u8> {
        bs58::decode(code).into_vec().unwrap()
    }

    #[test]
    fn version_0_codes_still_decode() {
        let token = [7u8; 16];
        let mut invite = key(1).as_bytes().to_vec();
        invite.extend_from_slice(&token);
        let invite = bs58::encode(invite).into_string();
        let knock = bs58::encode(key(1).as_bytes()).into_string();

        assert_eq!(
            Code::decode(&invite).unwrap(),
            Code::Invite {
                coordinator: key(1),
                relay: None,
                token
            }
        );
        assert_eq!(
            Code::decode(&knock).unwrap(),
            Code::Knock {
                coordinator: key(1),
                relay: None
            }
        );
    }

    #[test]
    fn n0_relays_are_written_as_an_index() {
        for (index, url) in N0_RELAYS.iter().enumerate() {
            let code = Code::Invite {
                coordinator: key(1),
                relay: Some(relay(url)),
                token: [3; 16],
            };
            let encoded = code.encode();
            let bytes = raw(&encoded);
            assert_eq!(bytes[0], CODE_V1);
            assert_eq!(&bytes[34..36], &[RELAY_N0, index as u8]);
            assert_eq!(bytes.len(), 52, "two bytes for the relay");
            assert_eq!(Code::decode(&encoded).unwrap(), code);
        }
        let knock = Code::Knock {
            coordinator: key(2),
            relay: Some(relay(N0_RELAYS[2])),
        };
        assert_eq!(Code::decode(&knock.encode()).unwrap(), knock);
    }

    #[test]
    fn every_relay_iroh_defaults_to_has_an_index() {
        // A new n0 relay would only make codes longer, but it should be noticed.
        for url in iroh::defaults::prod::default_relay_map().urls::<Vec<RelayUrl>>() {
            assert!(n0_index(&url).is_some(), "{url} has no index");
        }
    }

    #[test]
    fn custom_relays_are_written_out_in_full() {
        let custom = relay("https://relay.neozmmv.dev");
        let invite = Code::Invite {
            coordinator: key(1),
            relay: Some(custom.clone()),
            token: [9; 16],
        };
        let knock = Code::Knock {
            coordinator: key(1),
            relay: Some(custom.clone()),
        };
        assert_eq!(Code::decode(&invite.encode()).unwrap(), invite);
        assert_eq!(Code::decode(&knock.encode()).unwrap(), knock);
        assert_eq!(Code::decode(&knock.encode()).unwrap().relay(), Some(&custom));
    }

    #[test]
    fn a_code_without_a_relay_round_trips() {
        let knock = Code::Knock {
            coordinator: key(1),
            relay: None,
        };
        assert_eq!(raw(&knock.encode()).len(), MIN_V1_LEN);
        assert_eq!(Code::decode(&knock.encode()).unwrap(), knock);
    }

    #[test]
    fn a_new_code_never_reads_as_an_old_invite() {
        // 1 + 1 + 32 + 2 + 12 = 48: a twelve-character URL would make a knock
        // exactly as long as a version 0 invite, without the pad.
        let short = relay("https://a.b");
        assert_eq!(short.to_string().len(), 12, "{short}");
        let knock = Code::Knock {
            coordinator: key(1),
            relay: Some(short),
        };
        assert_eq!(raw(&knock.encode()).len(), V0_INVITE_LEN + 1);
        assert_eq!(Code::decode(&knock.encode()).unwrap(), knock);
    }

    #[test]
    fn unknown_versions_say_to_update() {
        let mut bytes = raw(&room_code(&key(1), Some(&relay(N0_RELAYS[0]))));
        bytes[0] = 2;
        let err = Code::decode(&bs58::encode(bytes).into_string()).unwrap_err();
        assert!(err.to_string().contains("update Noxid"), "{err}");
    }

    #[test]
    fn broken_version_1_codes_are_refused() {
        let encoded = Code::Invite {
            coordinator: key(1),
            relay: Some(relay("https://relay.example.com")),
            token: [1; 16],
        }
        .encode();
        let bytes = raw(&encoded);

        let cut = bs58::encode(&bytes[..bytes.len() - 1]).into_string();
        assert!(Code::decode(&cut).is_err(), "cut short");
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Code::decode(&bs58::encode(longer).into_string()).is_err(), "trailing bytes");
    }

    #[test]
    fn invites_name_the_relay_they_were_minted_with() {
        let mut room = Room::default();
        room.create(None, &id(1), "host");
        let own = relay(N0_RELAYS[1]);
        let (code, _) = room
            .generate_invite(key(1), Some(&own), Duration::minutes(5), minted_at())
            .unwrap();
        let decoded = Code::decode(&code).unwrap();
        assert_eq!(decoded.relay(), Some(&own));
        assert!(room.redeem_invite(&token_of(&code), &id(2), "ana", minted_at()));
    }

    #[test]
    fn a_roster_keeps_relays_but_only_https_ones() {
        let mut room = Room::default();
        room.set_joined(&id(1), &id(2), "me", None, vec![]);
        let with = |n: u8, relay: &str| Member {
            relay: Some(relay.to_string()),
            ..Member::new(id(n), "x")
        };

        room.set_roster(
            &id(2),
            "me",
            None,
            vec![with(3, "https://relay.example.com"), with(4, "http://10.0.0.1"), with(5, "nonsense")],
        );

        assert_eq!(room.relays(), vec![(key(3), relay("https://relay.example.com"))]);
    }

    #[test]
    fn members_without_relays_read_as_before() {
        let old: Member = serde_json::from_str(&format!(r#"{{"id":"{}","name":"ana"}}"#, id(2))).unwrap();
        assert_eq!(old.relay, None);
        let json = serde_json::to_string(&Member::new(id(2), "ana")).unwrap();
        assert!(!json.contains("relay"), "{json}");
    }

    #[test]
    fn only_a_coordinator_mints_invites() {
        let mut room = Room::default();
        room.set_joined(&id(1), &id(2), "me", None, vec![]);
        assert!(room.generate_invite(key(2), None, Duration::minutes(5), minted_at()).is_err());
    }

    #[test]
    fn an_invite_only_admits_one_requester() {
        let (mut room, token) = with_invite(Duration::minutes(5));

        assert!(room.redeem_invite(&token, &id(2), "ana", minted_at()));
        assert!(!room.redeem_invite(&token, &id(3), "bo", minted_at()), "token is spent");
        assert!(!room.is_member(&id(3)));
    }

    #[test]
    fn the_same_requester_retrying_is_admitted_again() {
        // Quix refused this: a join whose reply was lost retried, found its own
        // token spent, and was told the code did not work though it was in.
        let (mut room, token) = with_invite(Duration::minutes(5));

        assert!(room.redeem_invite(&token, &id(2), "ana", minted_at()));
        assert!(room.redeem_invite(&token, &id(2), "ana", minted_at() + Duration::seconds(3)));
        assert_eq!(room.roster().len(), 2, "admitted once, not twice");
    }

    #[test]
    fn a_retry_after_the_window_closes_is_refused() {
        let (mut room, token) = with_invite(Duration::minutes(5));

        assert!(room.redeem_invite(&token, &id(2), "ana", minted_at()));
        assert!(!room.redeem_invite(&token, &id(2), "ana", minted_at() + Duration::minutes(6)));
    }

    #[test]
    fn an_invite_expires_once_its_window_passes() {
        let (mut room, token) = with_invite(Duration::minutes(30));

        assert!(!room.redeem_invite(&token, &id(2), "ana", minted_at() + Duration::minutes(30)));
        assert!(!room.is_member(&id(2)));
    }

    #[test]
    fn a_failed_redemption_still_spends_the_token() {
        let (mut room, token) = with_invite(Duration::minutes(30));

        assert!(!room.redeem_invite(&token, &id(2), "ana", minted_at() + Duration::hours(1)));
        assert!(!room.redeem_invite(&token, &id(2), "ana", minted_at()), "gone even if time rewinds");
    }

    #[test]
    fn an_invite_does_not_survive_creating_another_room() {
        let (mut room, token) = with_invite(Duration::days(2));

        room.create(Some("room".into()), &id(1), "host");

        assert!(!room.redeem_invite(&token, &id(9), "x", minted_at()));
        assert!(!room.is_member(&id(9)));
    }

    #[test]
    fn an_invite_does_not_survive_joining_someone_elses_room() {
        let (mut room, token) = with_invite(Duration::days(2));

        room.set_joined(&id(5), &id(1), "me", None, vec![Member::new(id(5), "them")]);

        assert!(!room.redeem_invite(&token, &id(9), "x", minted_at()));
    }

    #[test]
    fn invite_windows_are_capped() {
        let mut room = Room::default();
        room.create(None, &id(1), "host");
        let (_, expires_at) = room
            .generate_invite(key(1), None, Duration::days(365), minted_at())
            .unwrap();
        assert_eq!(expires_at, minted_at() + MAX_INVITE_TTL);
    }

    #[test]
    fn joining_trusts_the_whole_roster_not_just_the_coordinator() {
        let mut room = Room::default();
        room.set_joined(
            &id(1),
            &id(2),
            "me",
            Some("standup".into()),
            vec![Member::new(id(1), "host"), Member::new(id(3), "bo")],
        );

        assert!(room.is_member(&id(3)), "members admitted before us must be trusted");
        assert!(room.is_member(&id(1)) && room.is_member(&id(2)));
        assert_eq!(room.name.as_deref(), Some("standup"));
        assert!(!room.is_coordinator(&id(2)));
    }

    #[test]
    fn a_roster_push_keeps_pictures_but_only_well_formed_ones() {
        let mut room = Room::default();
        room.set_joined(&id(1), &id(2), "me", None, vec![]);
        let good = crate::avatar::hash(b"picture");
        let with = |n: u8, avatar: &str| Member {
            avatar: Some(avatar.to_string()),
            ..Member::new(id(n), "x")
        };

        room.set_roster(&id(2), "me", None, vec![with(3, &good), with(4, "../../key")]);

        let avatar_of = |n: u8| room.roster().into_iter().find(|m| m.id == id(n)).unwrap().avatar;
        assert_eq!(avatar_of(3), Some(good));
        assert_eq!(avatar_of(4), None, "a path is never taken for a hash");
    }

    #[test]
    fn a_roster_push_cannot_evict_us_or_the_coordinator() {
        let mut room = Room::default();
        room.set_joined(&id(1), &id(2), "me", None, vec![]);

        room.set_roster(&id(2), "me", None, vec![Member::new(id(3), "bo")]);

        assert!(room.is_member(&id(2)), "we stay in our own room");
        assert!(room.is_member(&id(1)), "the coordinator stays");
        assert!(room.is_member(&id(3)));
    }

    #[test]
    fn a_roster_push_replaces_rather_than_merges() {
        let mut room = Room::default();
        room.set_joined(&id(1), &id(2), "me", None, vec![Member::new(id(3), "bo")]);

        room.set_roster(&id(2), "me", None, vec![Member::new(id(4), "cy")]);

        assert!(!room.is_member(&id(3)), "someone who left is dropped");
        assert!(room.is_member(&id(4)));
    }

    #[test]
    fn display_names_are_cleaned_up() {
        assert_eq!(display_name("  Ana  ", &id(2)), "Ana");
        assert_eq!(display_name("a\u{7}b", &id(2)), "ab");
        assert_eq!(display_name("", &id(2)), short_id(&id(2)));
        assert_eq!(display_name(&"x".repeat(100), &id(2)).len(), MAX_NAME_LEN);
    }

    #[test]
    fn readmitting_a_member_only_refreshes_its_name() {
        let mut room = Room::default();
        room.create(None, &id(1), "host");
        room.admit(&id(2), "ana");
        room.admit(&id(2), "Ana B");

        assert_eq!(room.roster().len(), 2);
        assert_eq!(room.find_member(&id(2)).unwrap().name, "Ana B");
    }

    #[test]
    fn members_are_found_by_id_short_id_or_unique_name() {
        let mut room = Room::default();
        room.create(None, &id(1), "host");
        room.admit(&id(2), "ana");
        room.admit(&id(3), "twin");
        room.admit(&id(4), "twin");

        assert_eq!(room.find_member(&id(2)).unwrap().id, id(2));
        assert_eq!(room.find_member(&short_id(&id(2))).unwrap().id, id(2));
        assert_eq!(room.find_member("ANA").unwrap().id, id(2));
        assert!(room.find_member("twin").is_none(), "ambiguous names find nobody");
        assert!(room.find_member("").is_none());
    }

    #[test]
    fn kicking_removes_the_member_and_says_who_it_was() {
        let mut room = Room::default();
        room.create(None, &id(1), "host");
        room.admit(&id(2), "ana");

        let kicked = room.kick("ana", &id(1)).unwrap();

        assert_eq!(kicked.id, id(2));
        assert!(!room.is_member(&id(2)));
        assert!(room.kick("host", &id(1)).is_err(), "the coordinator cannot kick itself");
        assert!(room.kick("nobody", &id(1)).is_err());
    }

    #[test]
    fn leaving_forgets_everything() {
        let (mut room, token) = with_invite(Duration::days(2));
        room.leave();

        assert!(!room.is_active());
        assert!(room.roster().is_empty());
        assert!(!room.redeem_invite(&token, &id(9), "x", minted_at()));
    }

    // ------------------------------------------------------------ hand-off

    fn host_with(members: &[u8]) -> Room {
        let mut room = Room::default();
        room.create(Some("room".into()), &id(1), "host");
        for n in members {
            room.admit(&id(*n), &format!("m{n}"));
        }
        room
    }

    fn ids(room: &Room) -> Vec<String> {
        room.roster().into_iter().map(|m| m.id).collect()
    }

    #[test]
    fn the_roster_keeps_joining_order() {
        let room = host_with(&[3, 2]);
        assert_eq!(ids(&room), vec![id(1), id(3), id(2)]);
    }

    #[test]
    fn a_members_copy_keeps_the_hosts_order() {
        let host = host_with(&[3, 2]);
        let mut member = Room::default();
        member.set_joined(&id(1), &id(2), "m2", None, host.roster());
        assert_eq!(ids(&member), ids(&host));
    }

    #[test]
    fn the_next_host_is_the_first_to_have_joined() {
        let host = host_with(&[3, 2]);
        assert_eq!(host.next_coordinator(), Some(id(3)));
        let mut member = Room::default();
        member.set_joined(&id(1), &id(2), "m2", None, host.roster());
        assert_eq!(member.next_coordinator(), Some(id(3)));
    }

    #[test]
    fn being_handed_the_room_makes_us_its_host() {
        let host = host_with(&[2, 3]);
        let mut member = Room::default();
        member.set_joined(&id(1), &id(2), "m2", Some("room".into()), host.roster());
        let rest: Vec<Member> = host.roster().into_iter().filter(|m| m.id != id(1)).collect();

        member.promote(&id(2), "m2", Some("room".into()), rest);

        assert!(member.is_coordinator(&id(2)));
        assert_eq!(ids(&member), vec![id(2), id(3)]);
        assert_eq!(member.name.as_deref(), Some("room"));
        assert!(member.generate_invite(key(2), None, DEFAULT_INVITE_TTL, minted_at()).is_ok());
    }

    #[test]
    fn the_old_hosts_invites_do_not_work_for_the_new_one() {
        let mut host = host_with(&[2]);
        let (code, _) = host.generate_invite(key(1), None, DEFAULT_INVITE_TTL, minted_at()).unwrap();
        let mut member = Room::default();
        member.set_joined(&id(1), &id(2), "m2", None, host.roster());
        member.promote(&id(2), "m2", None, vec![Member::new(id(2), "m2")]);
        assert!(!member.redeem_invite(&token_of(&code), &id(5), "x", minted_at()));
    }

    #[test]
    fn the_others_follow_the_new_host() {
        let host = host_with(&[2, 3]);
        let mut member = Room::default();
        member.set_joined(&id(1), &id(3), "m3", None, host.roster());
        let rest: Vec<Member> = host.roster().into_iter().filter(|m| m.id != id(1)).collect();

        member.hand_over(&id(2), &id(3), "m3", None, rest);

        assert_eq!(member.coordinator_id, Some(id(2)));
        assert!(!member.is_coordinator(&id(3)));
        assert_eq!(ids(&member), vec![id(2), id(3)]);
        assert!(!member.is_member(&id(1)), "the old host is gone");
    }

    #[test]
    fn a_hand_off_is_taken_only_from_the_host_and_repeats_are_harmless() {
        let host = host_with(&[2, 3]);
        let mut member = Room::default();
        member.set_joined(&id(1), &id(3), "m3", None, host.roster());
        assert!(!member.accepts_hand_over(&id(2), &id(2)), "only the host hands the room over");
        assert!(member.accepts_hand_over(&id(1), &id(2)));
        let rest: Vec<Member> = host.roster().into_iter().filter(|m| m.id != id(1)).collect();
        member.hand_over(&id(2), &id(3), "m3", None, rest);
        assert!(member.accepts_hand_over(&id(1), &id(2)), "the same hand-off again");
        assert!(!member.accepts_hand_over(&id(1), &id(3)), "but not a different one");
    }
}
