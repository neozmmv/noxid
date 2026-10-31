//! Gets decoded screen share frames into the webview.
//!
//! A WebSocket on 127.0.0.1 carries one binary message per frame: a fixed
//! header followed by the I420 planes, which the frontend uploads as three
//! textures and converts to RGB in a shader. I420 is 1.5 bytes per pixel
//! against RGBA's 4, and the colour conversion runs on the GPU.
//!
//! Why a socket rather than Tauri's own IPC: on WebKitGTK every IPC payload is
//! a custom-protocol fetch, which is the slow path for tens of megabytes a
//! second. A loopback socket behaves the same on both webviews.
//!
//! The port is random and the socket answers only a client presenting this
//! launch's random token, which the frontend gets through a Tauri command, so
//! another local process cannot watch the call.
//!
//! Frames are dropped, never queued: each tile keeps only its newest picture
//! (iroh-live's frame slot already works that way) and a full send queue drops
//! the frame rather than delaying the next one.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures_util::SinkExt;
use iroh::EndpointId;
use noxid_core::Node;
use noxid_core::media::Quality;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{ErrorResponse, Request, Response},
    http::StatusCode,
};
use tracing::{debug, info, warn};

type Frame = noxid_core::media::Frame;

/// Bytes before the planes: endpoint id (32), width and height (u32 LE each),
/// colour space (1), padding (3).
const HEADER_LEN: usize = 44;

/// Frames waiting to be written across all tiles. Small on purpose: a frame
/// that has to wait is already stale.
const SEND_QUEUE: usize = 4;

#[derive(Debug, Clone, serde::Serialize)]
pub struct FrameServer {
    pub port: u16,
    pub token: String,
}

pub async fn start(node: watch::Receiver<Option<Node>>) -> Result<FrameServer> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("failed to get random bytes");
    let token = hex::encode(bytes);
    info!(port, "frame server listening");

    let expected = token.clone();
    tokio::spawn(async move {
        // One webview at a time; a reload replaces the previous connection.
        let mut current: Option<JoinHandle<()>> = None;
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let Some(ws) = handshake(stream, &expected).await else {
                continue;
            };
            if let Some(previous) = current.take() {
                previous.abort();
            }
            current = Some(tokio::spawn(serve(ws, node.clone())));
        }
    });

    Ok(FrameServer { port, token })
}

async fn handshake(
    stream: TcpStream,
    token: &str,
) -> Option<tokio_tungstenite::WebSocketStream<TcpStream>> {
    let wanted = format!("token={token}");
    // The signature is tungstenite's handshake callback, so the large error
    // type is theirs to choose, not ours to box.
    #[allow(clippy::result_large_err, reason = "signature fixed by tungstenite's Callback")]
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let ok = request
            .uri()
            .query()
            .is_some_and(|q| q.split('&').any(|pair| pair == wanted));
        if ok {
            Ok(response)
        } else {
            let mut refusal = ErrorResponse::new(None);
            *refusal.status_mut() = StatusCode::FORBIDDEN;
            Err(refusal)
        }
    };
    match tokio_tungstenite::accept_hdr_async(stream, check).await {
        Ok(ws) => Some(ws),
        Err(e) => {
            debug!("frame socket refused: {e}");
            None
        }
    }
}

/// What a tile forwarder is reading, so a replaced slot (a share that stopped
/// and restarted) gets a fresh forwarder.
#[derive(PartialEq, Eq)]
enum Source {
    Remote(usize),
    Preview(usize),
}

async fn serve(mut ws: tokio_tungstenite::WebSocketStream<TcpStream>, mut node: watch::Receiver<Option<Node>>) {
    let node = loop {
        if let Some(node) = node.borrow_and_update().clone() {
            break node;
        }
        if node.changed().await.is_err() {
            return;
        }
    };

    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(SEND_QUEUE);
    let mut forwarders: HashMap<EndpointId, (Source, JoinHandle<()>)> = HashMap::new();
    let mut changes = node.changes();

    loop {
        sync_forwarders(&node, &tx, &mut forwarders);
        tokio::select! {
            changed = changes.changed() => if changed.is_err() { break },
            message = rx.recv() => {
                let Some(message) = message else { break };
                if ws.send(Message::binary(message)).await.is_err() {
                    break;
                }
            }
        }
    }
    for (_, (_, task)) in forwarders {
        task.abort();
    }
}

/// Starts a forwarder for every tile that has frames, and stops the rest.
fn sync_forwarders(
    node: &Node,
    tx: &mpsc::Sender<Vec<u8>>,
    forwarders: &mut HashMap<EndpointId, (Source, JoinHandle<()>)>,
) {
    let mut wanted: HashMap<EndpointId, Source> = HashMap::new();
    let snapshot = node.snapshot();
    let mut remote = HashMap::new();
    for member in snapshot.room.iter().flat_map(|r| &r.members) {
        let Ok(id) = member.id.parse::<EndpointId>() else { continue };
        if member.is_self {
            continue;
        }
        if let Some(slot) = node.frames(&id) {
            wanted.insert(id, Source::Remote(Arc::as_ptr(&slot) as usize));
            remote.insert(id, slot);
        }
    }
    let preview = node.preview();
    if let Some(slot) = &preview {
        wanted.insert(node.id(), Source::Preview(Arc::as_ptr(slot) as usize));
    }

    forwarders.retain(|id, (source, task)| {
        let keep = wanted.get(id) == Some(source) && !task.is_finished();
        if !keep {
            task.abort();
        }
        keep
    });

    for (id, source) in wanted {
        if forwarders.contains_key(&id) {
            continue;
        }
        let tx = tx.clone();
        let task = match &source {
            Source::Remote(_) => {
                let slot = remote.remove(&id).expect("collected above");
                tokio::spawn(async move {
                    while let Some(frame) = slot.recv().await {
                        forward(&tx, &id, &frame, None);
                    }
                })
            }
            Source::Preview(_) => {
                let slot = preview.clone().expect("collected above");
                tokio::spawn(async move {
                    while let Some(frame) = slot.recv().await {
                        // Our own screen is shown small; scaling it here keeps
                        // a 1080p self view from costing as much as a remote one.
                        forward(&tx, &id, &frame, Some(Quality::Low.height()));
                    }
                })
            }
        };
        forwarders.insert(id, (source, task));
    }
}

fn forward(tx: &mpsc::Sender<Vec<u8>>, id: &EndpointId, frame: &Frame, max_height: Option<u32>) {
    match encode(id, frame, max_height) {
        // A full queue means the webview is behind; this frame is dropped and
        // the next one goes instead.
        Ok(message) => {
            let _ = tx.try_send(message);
        }
        Err(e) => warn!("could not convert a frame: {e:#}"),
    }
}

fn encode(id: &EndpointId, frame: &Frame, max_height: Option<u32>) -> Result<Vec<u8>> {
    let scaled;
    let frame = match max_height {
        Some(max) if frame.size().height > max => {
            let size = frame.size();
            let width = (u64::from(size.width) * u64::from(max) / u64::from(size.height)) as u32;
            scaled = frame
                .resize(noxid_core::media::Size::new(width & !1, max & !1), &Default::default())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            &scaled
        }
        _ => frame,
    };

    let i420 = frame.surface.to_i420().map_err(|e| anyhow::anyhow!("{e}"))?;
    let size = i420.size();
    let color = i420
        .color()
        .or_else(|| frame.surface.color())
        .unwrap_or_else(|| noxid_core::media::Color::infer(size));
    use noxid_core::media::Color;
    let color_byte = match color {
        Color::Bt601Limited => 0u8,
        Color::Bt601Full => 1,
        Color::Bt709Limited => 2,
        Color::Bt709Full => 3,
        _ => 2,
    };

    let data = i420.data();
    let mut message = Vec::with_capacity(HEADER_LEN + data.len());
    message.extend_from_slice(id.as_bytes());
    message.extend_from_slice(&size.width.to_le_bytes());
    message.extend_from_slice(&size.height.to_le_bytes());
    message.push(color_byte);
    message.extend_from_slice(&[0, 0, 0]);
    message.extend_from_slice(data);
    Ok(message)
}
