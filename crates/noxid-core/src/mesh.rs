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

/// What a voice at 100% plays at. iroh-live's playback cannot amplify (its
/// volume stops at 1.0), so voices are played at half scale and turning
/// someone up to `MAX_VOLUME` is what reaches full scale. The sender makes up
/// the difference: its mic goes out 6 dB hotter (see `audio::voice`).
const VOICE_HEADROOM: f32 = 1.0 / crate::MAX_VOLUME;

/// The gain a "muted" voice plays at: -80 dB, inaudible, but enough for the
/// level meter to keep working so the speaking indicator still shows.
const MUTED_GAIN: f32 = 1e-4;

/// How often a broadcast the peer has not published yet is asked for again.
const RESUBSCRIBE: Duration = Duration::from_secs(1);

/// How often a voice's peak is sampled. The track reports the peak of its
/// latest output buffer, so sampling much faster than the level is published
/// is what keeps short syllables from slipping between two reads.
const LEVEL_SAMPLE: Duration = Duration::from_millis(20);

/// How often the highest sampled peak is published for the speaking indicator.
const LEVEL_PUBLISH: Duration = Duration::from_millis(100);

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

/// Keeps one peer connected for as long as it is on the roster. One session
/// carries two independent subscriptions: their voice, and their screen share
/// whenever they have one. Either ending on its own is normal (a share stops,
/// a voice broadcast restarts); the session ending starts everything over.
async fn peer_task(inner: Arc<Inner>, id: EndpointId, sessions: Sessions) {
    let mut backoff = BACKOFF_MIN;
    loop {
        match connect(&inner, id).await {
            Ok(session) => {
                backoff = BACKOFF_MIN;
                remember(&sessions, session.clone());
                info!(peer = %id.fmt_short(), "connected");
                tokio::select! {
                    _ = voice_loop(&inner, id, &session) => {}
                    _ = screen_loop(&inner, id, &session) => {}
                    _ = session.closed() => {}
                }
                info!(peer = %id.fmt_short(), "disconnected");
            }
            Err(e) => debug!(peer = %id.fmt_short(), "could not reach peer: {e:#}"),
        }

        inner.tiles.lock().unwrap().remove(&id);
        inner.levels.lock().unwrap().remove(&id);
        inner.peers.lock().unwrap().insert(id, PeerView::default());
        inner.bump();

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

async fn connect(inner: &Inner, id: EndpointId) -> Result<MoqSession> {
    tokio::time::timeout(STEP_TIMEOUT, inner.live.transport().connect(id))
        .await
        .map_err(|_| anyhow!("dial timed out"))?
        .map_err(|e| anyhow!("{e:#}"))
}

/// Subscribes to `path` on `session` once the peer publishes it, retrying
/// until it does: a voice appears a moment after joining, and a screen share
/// may not exist for an hour. Only the session closing ends the wait, and that
/// is the caller's `select!` to notice, so this never returns `None` on its own
/// account.
async fn subscribe(session: &MoqSession, path: &str) -> Subscription {
    loop {
        // The default policy hands decoded frames over in CPU memory, which is
        // what the frontend renderer reads.
        let attempt = async {
            let consumer = session.subscribe(path).await.map_err(|e| anyhow!("{e:#}"))?;
            tokio::time::timeout(STEP_TIMEOUT, RemoteBroadcast::new(path, consumer))
                .await
                .map_err(|_| anyhow!("no catalog in time"))?
                .map_err(|e| anyhow!("{e:#}"))
        };
        match attempt.await {
            Ok(broadcast) => return Subscription::new(session.clone(), broadcast),
            Err(e) => {
                tracing::trace!(%path, "not available yet: {e:#}");
                tokio::time::sleep(RESUBSCRIBE).await;
            }
        }
    }
}

/// Updates this peer's view in place and tells the frontend.
fn update_view(inner: &Inner, id: EndpointId, change: impl FnOnce(&mut PeerView)) {
    change(inner.peers.lock().unwrap().entry(id).or_default());
    inner.bump();
}

/// Their voice: follows mute and unmute, applies our volume for them, and
/// samples their level often enough for a speaking indicator.
async fn voice_loop(inner: &Inner, id: EndpointId, session: &MoqSession) {
    let path = crate::voice_path(&id);
    update_view(inner, id, |v| v.connected = true);
    let slow_every = (REFRESH.as_millis() / LEVEL_SAMPLE.as_millis()) as u32;
    let publish_every = (LEVEL_PUBLISH.as_millis() / LEVEL_SAMPLE.as_millis()) as u32;
    loop {
        let sub = subscribe(session, &path).await;
        let broadcast = sub.broadcast();
        let mut audio: Option<AudioTrack> = None;
        let mut applied_gain = f32::NAN;
        let mut tick = tokio::time::interval(LEVEL_SAMPLE);
        let mut ticks = 0u32;
        let mut held = 0.0f32;

        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = broadcast.closed() => break,
            }

            // Track changes and stats move slower; check them less often.
            if ticks.is_multiple_of(slow_every) {
                match (broadcast.has_audio(), audio.is_some()) {
                    (true, false) => match broadcast.audio().await {
                        Ok(track) => {
                            audio = Some(track);
                            applied_gain = f32::NAN;
                        }
                        Err(e) => debug!(peer = %id.fmt_short(), "audio would not open: {e:#}"),
                    },
                    (false, true) => audio = None,
                    _ => {}
                }
                let signals = *sub.signals().borrow();
                let has_audio = audio.is_some();
                update_view(inner, id, |v| {
                    v.connected = true;
                    v.has_audio = has_audio;
                    v.rtt_ms = (!signals.rtt.is_zero()).then_some(signals.rtt.as_millis() as u64);
                });
            }
            ticks = ticks.wrapping_add(1);

            let gain = if inner.deafened.load(std::sync::atomic::Ordering::Relaxed) {
                0.0
            } else {
                inner
                    .audio_prefs
                    .lock()
                    .unwrap()
                    .get(&id)
                    .map(|p| p.voice_gain())
                    .unwrap_or(1.0)
            };
            // The track's peak is measured after its volume, so a muted
            // participant would stop showing as speaking. Muting plays at an
            // inaudible gain instead of zero, and the meter divides it back out.
            let gain = (gain * VOICE_HEADROOM).max(MUTED_GAIN);
            let level = match &audio {
                Some(track) => {
                    if gain != applied_gain {
                        track.set_volume(gain);
                        applied_gain = gain;
                    }
                    (track.peak() / gain).clamp(0.0, 1.0)
                }
                None => 0.0,
            };
            held = held.max(level);
            if ticks.is_multiple_of(publish_every) {
                inner.levels.lock().unwrap().insert(id, held);
                held = 0.0;
            }
        }

        inner.levels.lock().unwrap().remove(&id);
        update_view(inner, id, |v| v.has_audio = false);
    }
}

/// Their screen share, each time they start one: the picture for the renderer
/// and the PC sound at our volume for it.
async fn screen_loop(inner: &Inner, id: EndpointId, session: &MoqSession) {
    let path = crate::screen_path(&id);
    loop {
        let sub = subscribe(session, &path).await;
        serve_screen(inner, id, &sub).await;
        inner.tiles.lock().unwrap().remove(&id);
        update_view(inner, id, |v| {
            v.has_video = false;
            v.has_screen_audio = false;
            v.rendition = None;
            v.fps = 0;
            v.width = 0;
            v.height = 0;
        });
    }
}

async fn serve_screen(inner: &Inner, id: EndpointId, sub: &Subscription) {
    let broadcast = sub.broadcast();
    let mut audio: Option<AudioTrack> = None;
    let mut video: Option<VideoTrack> = None;
    let mut applied: Option<QualityMode> = None;
    let mut applied_gain = f32::NAN;
    let mut counted = (Instant::now(), 0u64);
    let mut fps = 0u32;

    let mut tick = tokio::time::interval(REFRESH);
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = broadcast.closed() => return,
        }

        match (broadcast.has_audio(), audio.is_some()) {
            (true, false) => match broadcast.audio().await {
                Ok(track) => {
                    audio = Some(track);
                    applied_gain = f32::NAN;
                }
                Err(e) => debug!(peer = %id.fmt_short(), "share sound would not open: {e:#}"),
            },
            (false, true) => audio = None,
            _ => {}
        }
        if let Some(track) = &audio {
            let gain = if inner.deafened.load(std::sync::atomic::Ordering::Relaxed) {
                0.0
            } else {
                inner
                    .audio_prefs
                    .lock()
                    .unwrap()
                    .get(&id)
                    .map(|p| p.screen_gain())
                    .unwrap_or(1.0)
            };
            if gain != applied_gain {
                track.set_volume(gain);
                applied_gain = gain;
            }
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
        let has_video = video.is_some();
        let has_screen_audio = audio.is_some();
        update_view(inner, id, |v| {
            v.has_video = has_video;
            v.has_screen_audio = has_screen_audio;
            v.rendition = rendition;
            v.fps = if has_video { fps } else { 0 };
            v.width = width;
            v.height = height;
        });
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
