//! Who is in the room: the coordinator's admission rules and every member's
//! copy of the roster.
//!
//! Ported from Quix's `membership.rs`, keeping its model:
//!
//! - The node that creates a room is its coordinator. It alone admits members
//!   and pushes the roster to everyone else.
//! - An invite is single-use and expires. The code carries the coordinator's id
//!   and a random token, nothing else; which room the token is good for and
//!   until when are recorded only on the coordinator, which is the only party
//!   that checks them.
//! - `room_id` names one room apart from the next even when the same node
//!   creates both, so a token from an earlier room never admits anyone to a
//!   later one. It never goes on the wire.
//!
//! What changed from Quix:
//!
//! - Hostnames, DNS bindings and the name-pinning rules are gone. A member has a
//!   display name, which is a label for humans and grants nothing.
//! - A second way in: knocking. A room code (the coordinator's id alone) lets
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
use iroh::EndpointId;
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
}

impl Member {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            avatar: None,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// A single-use invite: admitted without asking anyone.
    Invite { coordinator: EndpointId, token: [u8; 16] },
    /// A room code: knock, and wait for the coordinator to let us in.
    Knock { coordinator: EndpointId },
}

impl Code {
    /// Decodes either kind. They are told apart by length: an invite is the
    /// coordinator's 32-byte id followed by a 16-byte token, and a room code is
    /// the id alone.
    pub fn decode(code: &str) -> Result<Self> {
        let bytes = bs58::decode(code.trim())
            .into_vec()
            .context("not a valid code")?;
        let id = |bytes: &[u8]| -> Result<EndpointId> {
            let array: [u8; 32] = bytes.try_into().expect("length checked");
            EndpointId::from_bytes(&array).context("code names an invalid endpoint")
        };
        match bytes.len() {
            48 => Ok(Code::Invite {
                coordinator: id(&bytes[..32])?,
                token: bytes[32..].try_into().expect("length checked"),
            }),
            32 => Ok(Code::Knock {
                coordinator: id(&bytes)?,
            }),
            _ => anyhow::bail!("not a valid code (wrong length)"),
        }
    }

    pub fn coordinator(&self) -> EndpointId {
        match self {
            Code::Invite { coordinator, .. } | Code::Knock { coordinator } => *coordinator,
        }
    }
}

/// The code anyone can use to knock on a room this node coordinates.
pub fn room_code(coordinator: &EndpointId) -> String {
    bs58::encode(coordinator.as_bytes()).into_string()
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

        let mut blob = Vec::with_capacity(48);
        blob.extend_from_slice(own_id.as_bytes());
        blob.extend_from_slice(&token);
        Ok((bs58::encode(blob).into_string(), expires_at))
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
                self.members.push(Member {
                    avatar,
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
        let (code, _) = room.generate_invite(key(1), ttl, minted_at()).unwrap();
        (room, token_of(&code))
    }

    #[test]
    fn invite_code_round_trips() {
        let mut room = Room::default();
        room.create(None, &id(1), "host");
        let (code, expires_at) = room
            .generate_invite(key(1), Duration::minutes(5), minted_at())
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
            Code::decode(&room_code(&key(1))).unwrap(),
            Code::Knock { coordinator: key(1) }
        );
        assert!(Code::decode("not-base58!").is_err());
        assert!(Code::decode("abc").is_err(), "too short");
    }

    #[test]
    fn only_a_coordinator_mints_invites() {
        let mut room = Room::default();
        room.set_joined(&id(1), &id(2), "me", None, vec![]);
        assert!(room.generate_invite(key(2), Duration::minutes(5), minted_at()).is_err());
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
            .generate_invite(key(1), Duration::days(365), minted_at())
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
        assert!(member.generate_invite(key(2), DEFAULT_INVITE_TTL, minted_at()).is_ok());
    }

    #[test]
    fn the_old_hosts_invites_do_not_work_for_the_new_one() {
        let mut host = host_with(&[2]);
        let (code, _) = host.generate_invite(key(1), DEFAULT_INVITE_TTL, minted_at()).unwrap();
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
