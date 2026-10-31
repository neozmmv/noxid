//! Keeps the call fully connected: one task per other member, started and
//! stopped as the roster changes.
//!
//! Plays the part of Quix's `mesh::dialer` and `Peers::set_routes`, with one
//! deliberate difference: both sides dial. Quix let only the lower id dial, so
//! a pair where that side could not reach the other never linked. iroh-moq
//! reuses a live session to a peer whichever side opened it, and copes with a
//! simultaneous dial, so dialing from both ends costs nothing.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use iroh::EndpointId;
use iroh_live::{
    Subscription,
    media::subscribe::{AudioTrack, RemoteBroadcast, VideoTrack},
    moq::MoqSession,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use crate::media::Quality;
use crate::node::{Inner, PeerView, QualityMode};

/// Bounds each step of reaching a peer, so a peer that vanished mid-dial does
/// not hold its task forever.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Retry pacing for a peer we cannot reach. Starts quick, because the usual
/// reason is a race (they have not received the roster with us in it yet).
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(10);

/// How often a connected peer's tracks and stats are refreshed.
const REFRESH: Duration = Duration::from_millis(500);

/// Close code for a member's connections once the roster drops them.
const CLOSE_REMOVED: u32 = 4;

/// Every session we hold per peer, dialed or accepted, so all of them can be
/// closed when the peer leaves the roster.
type Sessions = Arc<Mutex<HashMap<EndpointId, Vec<MoqSession>>>>;

/// Aborts the peer's task when the peer leaves the roster.
struct PeerTask(JoinHandle<()>);

impl Drop for PeerTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) async fn run(inner: Arc<Inner>, mut roster: watch::Receiver<Vec<EndpointId>>) {
    let own_id = inner.own_id();
    let sessions: Sessions = Default::default();
    let mut tasks: HashMap<EndpointId, PeerTask> = HashMap::new();
    let mut incoming = inner.live.transport().incoming_sessions();
    let mut incoming_open = true;

    loop {
        tokio::select! {
            changed = roster.changed() => {
                if changed.is_err() {
                    return;
                }
                let wanted: HashSet<EndpointId> = roster
                    .borrow_and_update()
                    .iter()
                    .copied()
                    .filter(|id| *id != own_id)
                    .collect();

                let departed: Vec<EndpointId> =
                    tasks.keys().filter(|id| !wanted.contains(*id)).copied().collect();
                for id in departed {
                    tasks.remove(&id);
                    close_sessions(&sessions, &id);
                    inner.peers.lock().unwrap().remove(&id);
                    inner.tiles.lock().unwrap().remove(&id);
                    info!(peer = %id.fmt_short(), "peer left the call");
                }
                // Sessions from peers that never had a task (they dialed us and
                // were then dropped) are closed too.
                let strays: Vec<EndpointId> = sessions
                    .lock()
                    .unwrap()
                    .keys()
                    .filter(|id| !wanted.contains(*id))
                    .copied()
                    .collect();
                for id in strays {
                    close_sessions(&sessions, &id);
                }

                for id in wanted {
                    tasks.entry(id).or_insert_with(|| {
                        info!(peer = %id.fmt_short(), "peer joined the call");
                        PeerTask(tokio::spawn(peer_task(inner.clone(), id, sessions.clone())))
                    });
                }
                inner.bump();
            }
            session = incoming.next(), if incoming_open => match session {
                Some(session) => {
                    let peer = session.remote_id();
                    let member = inner.room.lock().unwrap().is_member(&peer.to_string());
                    if member {
                        remember(&sessions, session);
                    } else {
                        // The gate refuses these before they get here; a peer
                        // removed in between is caught now.
                        session.conn().close(CLOSE_REMOVED.into(), b"not a member");
                    }
                }
                None => incoming_open = false,
            },
        }
    }
}

/// Records a session, dropping any for the same peer that already closed so
/// the list does not grow with every reconnect.
fn remember(sessions: &Sessions, session: MoqSession) {
    let mut sessions = sessions.lock().unwrap();
    let list = sessions.entry(session.remote_id()).or_default();
    list.retain(|s| s.conn().close_reason().is_none());
    list.push(session);
}

fn close_sessions(sessions: &Sessions, id: &EndpointId) {
    if let Some(list) = sessions.lock().unwrap().remove(id) {
        for session in list {
            session.conn().close(CLOSE_REMOVED.into(), b"removed from the room");
        }
    }
}

/// Keeps one peer connected for as long as it is on the roster: dial,
/// subscribe, follow its tracks, and start over whenever that fails.
async fn peer_task(inner: Arc<Inner>, id: EndpointId, sessions: Sessions) {
    let path = crate::broadcast_path(&id);
    let mut backoff = BACKOFF_MIN;
    loop {
        match connect(&inner, id, &path).await {
            Ok(sub) => {
                backoff = BACKOFF_MIN;
                remember(&sessions, sub.session().clone());
                info!(peer = %id.fmt_short(), "connected");
                serve(&inner, id, &sub).await;
                info!(peer = %id.fmt_short(), "disconnected");
            }
            Err(e) => debug!(peer = %id.fmt_short(), "could not reach peer: {e:#}"),
        }

        inner.tiles.lock().unwrap().remove(&id);
        inner.peers.lock().unwrap().insert(id, PeerView::default());
        inner.bump();

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

async fn connect(inner: &Inner, id: EndpointId, path: &str) -> Result<Subscription> {
    let session = tokio::time::timeout(STEP_TIMEOUT, inner.live.transport().connect(id))
        .await
        .map_err(|_| anyhow!("dial timed out"))?
        .map_err(|e| anyhow!("{e:#}"))?;
    let consumer = tokio::time::timeout(STEP_TIMEOUT, session.subscribe(path))
        .await
        .map_err(|_| anyhow!("not announced in time"))?
        .map_err(|e| anyhow!("{e:#}"))?;
    // The default policy hands decoded frames over in CPU memory, which is
    // what the frontend's renderer reads.
    let broadcast = tokio::time::timeout(STEP_TIMEOUT, RemoteBroadcast::new(path, consumer))
        .await
        .map_err(|_| anyhow!("no catalog in time"))?
        .map_err(|e| anyhow!("{e:#}"))?;
    Ok(Subscription::new(session, broadcast))
}

/// Follows one connected peer until its broadcast or session ends.
///
/// Both tracks follow the catalog rather than being opened once: a participant
/// mutes, unmutes, starts and stops sharing mid-call.
async fn serve(inner: &Inner, id: EndpointId, sub: &Subscription) {
    let broadcast = sub.broadcast();
    let mut audio: Option<AudioTrack> = None;
    let mut video: Option<VideoTrack> = None;
    let mut applied: Option<QualityMode> = None;
    let mut counted = (Instant::now(), 0u64);
    let mut fps = 0u32;

    let mut tick = tokio::time::interval(REFRESH);
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = broadcast.closed() => return,
            _ = sub.session().closed() => return,
        }

        match (broadcast.has_audio(), audio.is_some()) {
            (true, false) => match broadcast.audio().await {
                Ok(track) => audio = Some(track),
                Err(e) => debug!(peer = %id.fmt_short(), "audio would not open: {e:#}"),
            },
            (false, true) => audio = None,
            _ => {}
        }

        match (broadcast.has_video(), video.is_some()) {
            (true, false) => match broadcast.video().await {
                Ok(track) => {
                    inner.tiles.lock().unwrap().insert(id, track.frame_slot());
                    counted = (Instant::now(), track.frames().produced());
                    applied = None;
                    video = Some(track);
                }
                Err(e) => debug!(peer = %id.fmt_short(), "screen share would not open: {e:#}"),
            },
            (false, true) => {
                video = None;
                inner.tiles.lock().unwrap().remove(&id);
            }
            _ => {}
        }

        if let Some(track) = &video {
            let mode = inner
                .quality_modes
                .lock()
                .unwrap()
                .get(&id)
                .copied()
                .unwrap_or_default();
            if applied != Some(mode) {
                match mode {
                    QualityMode::Auto => track.enable_adaptation(sub.signals().clone()),
                    QualityMode::Pinned(wanted) => {
                        track.disable_adaptation();
                        let available: Vec<String> =
                            broadcast.catalog().video().keys().cloned().collect();
                        if let Some(name) = closest(wanted, &available) {
                            track.set_rendition(name);
                        }
                    }
                }
                applied = Some(mode);
            }

            let elapsed = counted.0.elapsed();
            if elapsed >= Duration::from_secs(1) {
                let produced = track.frames().produced();
                fps = ((produced - counted.1) as f64 / elapsed.as_secs_f64()).round() as u32;
                counted = (Instant::now(), produced);
            }
        }

        let rendition = video.as_ref().map(|t| t.rendition());
        let (width, height) = rendition
            .as_ref()
            .and_then(|r| broadcast.catalog().video().get(r).cloned())
            .map(|c| (c.coded_width.unwrap_or(0), c.coded_height.unwrap_or(0)))
            .unwrap_or((0, 0));
        let signals = *sub.signals().borrow();
        let view = PeerView {
            connected: true,
            has_audio: audio.is_some(),
            has_video: video.is_some(),
            level: audio.as_ref().map(|a| a.peak().clamp(0.0, 1.0)).unwrap_or(0.0),
            rendition,
            fps: if video.is_some() { fps } else { 0 },
            width,
            height,
            rtt_ms: (!signals.rtt.is_zero()).then_some(signals.rtt.as_millis() as u64),
            quality: QualityMode::default(),
        };
        inner.peers.lock().unwrap().insert(id, view);
        inner.bump();
    }
}

/// The published rendition nearest to `wanted` without going over it, or the
/// smallest one if every rung is above it. A sender capped below what the
/// receiver asked for simply does not have the larger rungs.
fn closest(wanted: Quality, available: &[String]) -> Option<&'static str> {
    let published: Vec<Quality> = available
        .iter()
        .filter_map(|name| Quality::from_rendition(name))
        .collect();
    published
        .iter()
        .copied()
        .filter(|q| *q <= wanted)
        .max()
        .or_else(|| published.iter().copied().min())
        .map(Quality::rendition)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(qualities: &[Quality]) -> Vec<String> {
        qualities.iter().map(|q| q.rendition().to_string()).collect()
    }

    #[test]
    fn a_pinned_rendition_falls_back_to_what_was_published() {
        let all = names(&Quality::ALL);
        assert_eq!(closest(Quality::Mid, &all), Some("screen/mid"));

        let capped = names(&[Quality::Low, Quality::Mid]);
        assert_eq!(closest(Quality::High, &capped), Some("screen/mid"), "sender capped at mid");

        let only_mid = names(&[Quality::Mid]);
        assert_eq!(closest(Quality::Low, &only_mid), Some("screen/mid"), "nothing smaller exists");

        assert_eq!(closest(Quality::High, &["video".to_string()]), None);
    }
}
