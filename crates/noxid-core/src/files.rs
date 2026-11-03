//! Sending a file to another member, straight between the two machines.
//!
//! One connection per transfer, on an ALPN of its own, members only. The
//! sender opens a bidirectional stream and offers the file (its name and
//! size); the receiver asks its user and answers one byte, accept or decline.
//! On accept the bytes follow on the same stream, and once they are all on
//! disk the receiver answers one more byte, so "sent" means "saved".
//!
//! Either side can stop at any moment by closing the connection with
//! [`CLOSE_CANCELED`], which is how the other side tells "they cancelled"
//! from a dropped link.
//!
//! A received file is written next to its final name as `<name>.part`,
//! renamed when complete, and deleted if the transfer does not finish, so
//! Downloads never holds a half file under the real name.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::{Connection, ConnectionError, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, watch};
use tracing::{debug, info, warn};

use crate::node::Inner;

pub const FILES_ALPN: &[u8] = b"noxid-files/0";

/// Close codes. Zero is a finished transfer.
const CLOSE_DONE: u32 = 0;
const CLOSE_NOT_MEMBER: u32 = 1;
const CLOSE_DECLINED: u32 = 2;
pub const CLOSE_CANCELED: u32 = 3;
const CLOSE_FAILED: u32 = 4;

/// Largest offer accepted: a file name and a size.
const MAX_OFFER: u32 = 4096;

/// How long the sender has to make its offer once connected.
const OFFER_TIMEOUT: Duration = Duration::from_secs(10);

/// How long an offer waits for the receiver to decide.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(180);

/// How long the sender waits to reach the receiver at all.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the sender waits for "saved" after its last byte: the receiver
/// may still be flushing a large file to disk.
const SAVED_TIMEOUT: Duration = Duration::from_secs(60);

/// Bytes read from disk or the network at a time.
const CHUNK: usize = 256 * 1024;

/// How often progress reaches the frontend. Every chunk would be thousands
/// of redraws a second on a fast link.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);

/// Finished transfers kept for the frontend to show, beyond which the oldest
/// are forgotten.
const MAX_KEPT: usize = 20;

/// One byte answers.
const ACCEPT: u8 = 1;
const DECLINE: u8 = 0;
const SAVED: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Offer {
    name: String,
    size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Send,
    Receive,
}

/// Where a transfer is. The frontend draws each one from this alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "message", rename_all = "snake_case")]
pub enum TransferState {
    /// Sending: waiting for them to accept.
    Waiting,
    /// Receiving: waiting for us to accept.
    Offered,
    /// Bytes are moving.
    Active,
    Done,
    /// They (or, receiving, we) said no.
    Declined,
    /// We stopped it.
    Canceled,
    /// They stopped it.
    CanceledByPeer,
    Failed(String),
}

impl TransferState {
    fn finished(&self) -> bool {
        !matches!(self, Self::Waiting | Self::Offered | Self::Active)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferView {
    pub id: u64,
    /// The other member's endpoint id, and their name at the time.
    pub peer: String,
    pub peer_name: String,
    pub direction: Direction,
    pub file_name: String,
    /// Total size in bytes, and how much has moved.
    pub size: u64,
    pub done: u64,
    #[serde(flatten)]
    pub state: TransferState,
    /// Receiving, once done: where the file was saved.
    pub saved_to: Option<String>,
}

struct Entry {
    view: TransferView,
    /// Set to true to stop the transfer.
    cancel: watch::Sender<bool>,
    /// Receiving, while offered: the user's answer.
    answer: Option<oneshot::Sender<bool>>,
}

/// Every transfer this node knows about, in and out.
#[derive(Default)]
pub(crate) struct Transfers {
    next: AtomicU64,
    list: Mutex<Vec<Entry>>,
}

impl Transfers {
    pub fn views(&self) -> Vec<TransferView> {
        self.list.lock().unwrap().iter().map(|e| e.view.clone()).collect()
    }

    fn add(&self, view: TransferView, answer: Option<oneshot::Sender<bool>>) -> (u64, watch::Receiver<bool>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (cancel, canceled) = watch::channel(false);
        let mut list = self.list.lock().unwrap();
        list.push(Entry {
            view: TransferView { id, ..view },
            cancel,
            answer,
        });
        // Forget the oldest finished ones once too many have piled up.
        while list.len() > MAX_KEPT {
            match list.iter().position(|e| e.view.state.finished()) {
                Some(oldest) => {
                    list.remove(oldest);
                }
                None => break,
            }
        }
        (id, canceled)
    }

    fn update(&self, id: u64, change: impl FnOnce(&mut TransferView)) {
        if let Some(entry) = self.list.lock().unwrap().iter_mut().find(|e| e.view.id == id) {
            change(&mut entry.view);
        }
    }

    /// Accepts or declines an offer we received.
    pub fn answer(&self, id: u64, accept: bool) -> Result<()> {
        let mut list = self.list.lock().unwrap();
        let entry = list.iter_mut().find(|e| e.view.id == id).context("no such transfer")?;
        let reply = entry.answer.take().context("that transfer is not waiting for an answer")?;
        let _ = reply.send(accept);
        Ok(())
    }

    /// Stops a transfer in either direction, at whatever stage it is.
    pub fn cancel(&self, id: u64) {
        if let Some(entry) = self.list.lock().unwrap().iter().find(|e| e.view.id == id) {
            entry.cancel.send_replace(true);
        }
    }

    pub fn cancel_all(&self) {
        for entry in self.list.lock().unwrap().iter() {
            if !entry.view.state.finished() {
                entry.cancel.send_replace(true);
            }
        }
    }

    /// Forgets a finished transfer, so the frontend stops showing it.
    pub fn dismiss(&self, id: u64) {
        self.list
            .lock()
            .unwrap()
            .retain(|e| e.view.id != id || !e.view.state.finished());
    }

    /// Where a received file was saved.
    pub fn saved_to(&self, id: u64) -> Option<PathBuf> {
        self.list
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.view.id == id)
            .and_then(|e| e.view.saved_to.clone())
            .map(PathBuf::from)
    }
}

/// Paces progress reports.
struct Progress<'a> {
    inner: &'a Inner,
    id: u64,
    last: Instant,
}

impl<'a> Progress<'a> {
    fn new(inner: &'a Inner, id: u64) -> Self {
        Self {
            inner,
            id,
            last: Instant::now(),
        }
    }

    fn report(&mut self, done: u64) {
        self.inner.transfers.update(self.id, |v| v.done = done);
        if self.last.elapsed() >= PROGRESS_EVERY {
            self.last = Instant::now();
            self.inner.bump();
        }
    }
}

/// How a transfer ended, from the connection's close reason when it was the
/// other side that ended it.
fn ending(connection: &Connection, error: anyhow::Error) -> TransferState {
    if let Some(ConnectionError::ApplicationClosed(close)) = connection.close_reason() {
        match u64::from(close.error_code) {
            code if code == u64::from(CLOSE_CANCELED) => return TransferState::CanceledByPeer,
            code if code == u64::from(CLOSE_DECLINED) => return TransferState::Declined,
            code if code == u64::from(CLOSE_NOT_MEMBER) => {
                return TransferState::Failed("they are not in this room any more".into());
            }
            _ => {}
        }
    }
    TransferState::Failed(format!("{error:#}"))
}

/// Waits until `canceled` turns true.
async fn canceled(mut canceled: watch::Receiver<bool>) {
    let _ = canceled.wait_for(|c| *c).await;
}

/// The other member's name, as the roster has it.
fn member_name(inner: &Inner, id: &EndpointId) -> String {
    let id = id.to_string();
    inner
        .room
        .lock()
        .unwrap()
        .roster()
        .into_iter()
        .find(|m| m.id == id)
        .map(|m| m.name)
        .unwrap_or_else(|| id.chars().take(10).collect())
}

fn is_member(inner: &Inner, id: &EndpointId) -> bool {
    *id != inner.own_id() && inner.room.lock().unwrap().is_member(&id.to_string())
}

// ---------------------------------------------------------------- sending

/// Offers the file at `path` to `peer` and, if they accept, sends it. Returns
/// the transfer's id at once; the transfer runs on its own task.
pub(crate) async fn send(inner: Arc<Inner>, peer: EndpointId, path: PathBuf) -> Result<u64> {
    if !is_member(&inner, &peer) {
        bail!("they are not in this room");
    }
    let meta = tokio::fs::metadata(&path).await.context("the file could not be read")?;
    if !meta.is_file() {
        bail!("only files can be sent, not folders");
    }
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .context("the file has no name")?;
    let view = TransferView {
        id: 0,
        peer: peer.to_string(),
        peer_name: member_name(&inner, &peer),
        direction: Direction::Send,
        file_name: file_name.clone(),
        size: meta.len(),
        done: 0,
        state: TransferState::Waiting,
        saved_to: None,
    };
    let (id, cancel) = inner.transfers.add(view, None);
    inner.bump();
    info!(%peer, file = %file_name, size = meta.len(), "offering a file");

    tokio::spawn(async move {
        let state = run_send(&inner, id, peer, &path, file_name, meta.len(), cancel).await;
        info!(id, ?state, "file transfer out ended");
        inner.transfers.update(id, |v| v.state = state);
        inner.bump();
    });
    Ok(id)
}

async fn run_send(
    inner: &Inner,
    id: u64,
    peer: EndpointId,
    path: &Path,
    name: String,
    size: u64,
    cancel: watch::Receiver<bool>,
) -> TransferState {
    let connect = tokio::time::timeout(CONNECT_TIMEOUT, inner.endpoint.connect(peer, FILES_ALPN));
    let connection = tokio::select! {
        result = connect => match result {
            Ok(Ok(connection)) => connection,
            Ok(Err(e)) => return TransferState::Failed(format!("could not reach them: {e:#}")),
            Err(_) => return TransferState::Failed("could not reach them in time".into()),
        },
        _ = canceled(cancel.clone()) => return TransferState::Canceled,
    };

    let work = sending(inner, id, &connection, path, Offer { name, size });
    let state = tokio::select! {
        result = work => match result {
            Ok(state) => state,
            Err(e) => ending(&connection, e),
        },
        _ = canceled(cancel) => {
            connection.close(CLOSE_CANCELED.into(), b"canceled");
            return TransferState::Canceled;
        }
    };
    let code = match state {
        TransferState::Done | TransferState::Declined => CLOSE_DONE,
        _ => CLOSE_FAILED,
    };
    connection.close(code.into(), b"done");
    state
}

async fn sending(inner: &Inner, id: u64, connection: &Connection, path: &Path, offer: Offer) -> Result<TransferState> {
    let (mut send, mut recv) = connection.open_bi().await.context("open stream")?;
    let header = serde_json::to_vec(&offer)?;
    send.write_all(&(header.len() as u32).to_le_bytes()).await?;
    send.write_all(&header).await?;

    let mut answer = [0u8; 1];
    tokio::time::timeout(ANSWER_TIMEOUT + OFFER_TIMEOUT, recv.read_exact(&mut answer))
        .await
        .map_err(|_| anyhow!("they did not answer"))?
        .context("no answer")?;
    if answer[0] != ACCEPT {
        return Ok(TransferState::Declined);
    }
    inner.transfers.update(id, |v| v.state = TransferState::Active);
    inner.bump();

    let mut file = tokio::fs::File::open(path).await.context("the file could not be opened")?;
    let mut buffer = vec![0u8; CHUNK];
    let mut done = 0u64;
    let mut progress = Progress::new(inner, id);
    while done < offer.size {
        let n = file.read(&mut buffer).await.context("reading the file")?;
        if n == 0 {
            bail!("the file got shorter while it was being sent");
        }
        // Never send more than offered, even if the file grew meanwhile.
        let n = n.min((offer.size - done) as usize);
        send.write_all(&buffer[..n]).await.context("sending")?;
        done += n as u64;
        progress.report(done);
    }
    send.finish()?;
    inner.transfers.update(id, |v| v.done = done);
    inner.bump();

    let mut saved = [0u8; 1];
    tokio::time::timeout(SAVED_TIMEOUT, recv.read_exact(&mut saved))
        .await
        .map_err(|_| anyhow!("they did not confirm the file was saved"))?
        .context("no confirmation")?;
    Ok(if saved[0] == SAVED {
        TransferState::Done
    } else {
        TransferState::Failed("they could not save the file".into())
    })
}

// -------------------------------------------------------------- receiving

#[derive(Clone)]
pub(crate) struct FilesHandler {
    pub inner: Arc<Inner>,
}

impl std::fmt::Debug for FilesHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesHandler").finish_non_exhaustive()
    }
}

impl ProtocolHandler for FilesHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer = connection.remote_id();
        if !is_member(&self.inner, &peer) {
            debug!(%peer, "refused a file from a non-member");
            connection.close(CLOSE_NOT_MEMBER.into(), b"not a member");
            return Ok(());
        }
        let offered = tokio::time::timeout(OFFER_TIMEOUT, read_offer(&connection)).await;
        let (send, recv, offer) = match offered {
            Ok(Ok(offered)) => offered,
            Ok(Err(e)) => {
                debug!(%peer, "bad file offer: {e:#}");
                connection.close(CLOSE_FAILED.into(), b"bad offer");
                return Ok(());
            }
            Err(_) => {
                connection.close(CLOSE_FAILED.into(), b"offer timed out");
                return Ok(());
            }
        };
        receive(&self.inner, &connection, peer, offer, send, recv).await;
        Ok(())
    }
}

async fn read_offer(connection: &Connection) -> Result<(SendStream, RecvStream, Offer)> {
    let (send, mut recv) = connection.accept_bi().await?;
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len);
    if len > MAX_OFFER {
        bail!("offer of {len} bytes is too large");
    }
    let mut header = vec![0u8; len as usize];
    recv.read_exact(&mut header).await?;
    let offer: Offer = serde_json::from_slice(&header)?;
    Ok((send, recv, offer))
}

async fn receive(inner: &Inner, connection: &Connection, peer: EndpointId, offer: Offer, send: SendStream, recv: RecvStream) {
    let file_name = safe_name(&offer.name);
    let (reply, answer) = oneshot::channel();
    let view = TransferView {
        id: 0,
        peer: peer.to_string(),
        peer_name: member_name(inner, &peer),
        direction: Direction::Receive,
        file_name: file_name.clone(),
        size: offer.size,
        done: 0,
        state: TransferState::Offered,
        saved_to: None,
    };
    let (id, cancel) = inner.transfers.add(view, Some(reply));
    inner.bump();
    info!(%peer, file = %file_name, size = offer.size, "offered a file");

    let state = receiving(inner, id, connection, file_name, offer.size, answer, send, recv, cancel).await;
    info!(id, ?state, "file transfer in ended");
    // Declining is our own choice, already made: nothing left worth showing.
    if state == TransferState::Declined {
        inner.transfers.update(id, |v| v.state = state);
        inner.transfers.dismiss(id);
    } else {
        inner.transfers.update(id, |v| v.state = state);
    }
    inner.bump();
}

#[allow(clippy::too_many_arguments, reason = "one call site, each argument distinct")]
async fn receiving(
    inner: &Inner,
    id: u64,
    connection: &Connection,
    file_name: String,
    size: u64,
    answer: oneshot::Receiver<bool>,
    mut send: SendStream,
    mut recv: RecvStream,
    cancel: watch::Receiver<bool>,
) -> TransferState {
    let accepted = tokio::select! {
        answer = tokio::time::timeout(ANSWER_TIMEOUT, answer) => matches!(answer, Ok(Ok(true))),
        // They gave up waiting, or cancelled the offer.
        _ = connection.closed() => return ending(connection, anyhow!("they withdrew the file")),
        _ = canceled(cancel.clone()) => {
            connection.close(CLOSE_CANCELED.into(), b"canceled");
            return TransferState::Canceled;
        }
    };
    if !accepted {
        let _ = send.write_all(&[DECLINE]).await;
        let _ = send.finish();
        let _ = tokio::time::timeout(Duration::from_secs(2), connection.closed()).await;
        return TransferState::Declined;
    }

    let (part, target) = match destination(&file_name).await {
        Ok(paths) => paths,
        Err(e) => {
            connection.close(CLOSE_FAILED.into(), b"cannot save");
            return TransferState::Failed(format!("the file could not be saved: {e:#}"));
        }
    };
    let work = async {
        let mut file = tokio::fs::File::create(&part).await.context("creating the file")?;
        send.write_all(&[ACCEPT]).await?;
        inner.transfers.update(id, |v| v.state = TransferState::Active);
        inner.bump();

        let mut buffer = vec![0u8; CHUNK];
        let mut done = 0u64;
        let mut progress = Progress::new(inner, id);
        while let Some(n) = recv.read(&mut buffer).await.context("receiving")? {
            done += n as u64;
            if done > size {
                bail!("they sent more than they offered");
            }
            file.write_all(&buffer[..n]).await.context("writing the file")?;
            progress.report(done);
        }
        if done != size {
            bail!("the file ended early");
        }
        file.flush().await?;
        file.sync_all().await.ok();
        drop(file);
        tokio::fs::rename(&part, &target).await.context("naming the file")?;
        let _ = send.write_all(&[SAVED]).await;
        let _ = send.finish();
        // Let "saved" reach them before the connection goes.
        let _ = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await;
        anyhow::Ok(())
    };
    // Stopping ends the work, then falls through to the same cleanup as a
    // failure, so a half file never stays behind.
    let result = tokio::select! {
        result = work => result,
        _ = canceled(cancel.clone()) => Err(anyhow!("canceled")),
    };

    match result {
        Ok(()) => {
            let saved = target.to_string_lossy().into_owned();
            inner.transfers.update(id, |v| v.saved_to = Some(saved));
            TransferState::Done
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            if *cancel.borrow() {
                connection.close(CLOSE_CANCELED.into(), b"canceled");
                return TransferState::Canceled;
            }
            let state = ending(connection, e);
            connection.close(CLOSE_FAILED.into(), b"failed");
            state
        }
    }
}

/// Where a received file goes: the Downloads folder, under its own name or,
/// if that is taken, `name (2).ext` and so on. Returns the temporary path to
/// write and the final one.
async fn destination(file_name: &str) -> Result<(PathBuf, PathBuf)> {
    let folder = dirs::download_dir()
        .or_else(dirs::home_dir)
        .context("there is no Downloads folder")?;
    tokio::fs::create_dir_all(&folder).await?;
    let path = Path::new(file_name);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = path.extension().map(|e| format!(".{}", e.to_string_lossy()));
    for n in 1..10_000 {
        let name = match n {
            1 => file_name.to_string(),
            n => format!("{stem} ({n}){}", extension.as_deref().unwrap_or("")),
        };
        let target = folder.join(&name);
        let part = folder.join(format!("{name}.part"));
        let taken = tokio::fs::try_exists(&target).await.unwrap_or(true)
            || tokio::fs::try_exists(&part).await.unwrap_or(true);
        if !taken {
            return Ok((part, target));
        }
    }
    bail!("too many files called {file_name:?} already")
}

/// A name that is safe to create in Downloads whatever the sender put in the
/// offer: no folders, nothing Windows refuses, and never empty or hidden.
fn safe_name(offered: &str) -> String {
    // Only the last component: "../../evil" and "C:\\x\\y" become "evil" and "y".
    let last = offered.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim_end_matches(['.', ' ']);
    let mut name: String = trimmed.chars().take(200).collect();
    if name.is_empty() {
        name = "file".to_string();
    }
    // Device names Windows will not create as files, with or without an extension.
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem.as_str()) {
        name = format!("_{name}");
    }
    name
}

/// Ends every transfer still running, for leaving the room.
pub(crate) fn stop_all(inner: &Inner) {
    inner.transfers.cancel_all();
    if inner.transfers.views().iter().any(|v| !v.state.finished()) {
        warn!("stopping file transfers on the way out");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offered_names_cannot_escape_downloads() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\Windows\\system32\\evil.dll"), "evil.dll");
        assert_eq!(safe_name("..."), "file");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name(".bashrc"), "bashrc");
        assert_eq!(safe_name("a<b>c:d\"e|f?g*h.txt"), "a_b_c_d_e_f_g_h.txt");
        assert_eq!(safe_name("con.txt"), "_con.txt");
        assert_eq!(safe_name("report final.pdf. "), "report final.pdf");
        assert_eq!(safe_name("photo.jpg"), "photo.jpg");
    }

    #[test]
    fn finished_states() {
        assert!(!TransferState::Waiting.finished());
        assert!(!TransferState::Offered.finished());
        assert!(!TransferState::Active.finished());
        assert!(TransferState::Done.finished());
        assert!(TransferState::CanceledByPeer.finished());
    }
}
