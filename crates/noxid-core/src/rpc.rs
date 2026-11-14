//! One JSON request and one JSON reply per connection: the exchange the admin
//! and friends ALPNs both use. Lifted out of `admin.rs` so the two cannot
//! drift apart on bounds and timeouts.
//!
//! The sender of a request is the connection's remote id, proven by iroh's
//! handshake. Nothing read here is trusted to say who sent it.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::{Connection, SendStream};
use iroh::protocol::AcceptError;
use iroh::{Endpoint, EndpointId};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// How long a handler waits, after replying, for the caller to read the reply
/// and close. Never longer: a peer that does not close is not waited on.
const LINGER: Duration = Duration::from_secs(5);

/// Opens a connection to `to` on `alpn`, sends `request`, and reads one reply
/// of at most `max` bytes, all within `timeout`.
pub(crate) async fn call<Req: Serialize, Resp: DeserializeOwned>(
    endpoint: &Endpoint,
    to: EndpointId,
    alpn: &[u8],
    request: &Req,
    max: usize,
    timeout: Duration,
) -> Result<Resp> {
    tokio::time::timeout(timeout, async {
        let conn: Connection = endpoint.connect(to, alpn).await.context("connect")?;
        let (mut send, mut recv) = conn.open_bi().await.context("open stream")?;
        send.write_all(&serde_json::to_vec(request)?)
            .await
            .context("write")?;
        send.finish().context("finish")?;
        let data = recv.read_to_end(max).await.context("read response")?;
        conn.close(0u32.into(), b"done");
        serde_json::from_slice(&data).context("parse response")
    })
    .await
    .context("timed out")?
}

/// What a handler read from an incoming connection.
pub(crate) enum Incoming<Req> {
    Request(SendStream, Req),
    /// It was not a request we understand; the stream to answer on, and why.
    Malformed(SendStream, String),
    /// Nothing arrived in time; the connection has been closed.
    TimedOut,
}

/// Reads one request of at most `max` bytes, waiting no longer than `timeout`.
pub(crate) async fn read_request<Req: DeserializeOwned>(
    connection: &Connection,
    max: usize,
    timeout: Duration,
) -> Result<Incoming<Req>, AcceptError> {
    let read = tokio::time::timeout(timeout, async {
        let (send, mut recv) = connection.accept_bi().await?;
        let data = recv.read_to_end(max).await.map_err(std::io::Error::other)?;
        Ok::<_, AcceptError>((send, data))
    })
    .await;
    let (send, data) = match read {
        Ok(result) => result?,
        Err(_) => {
            connection.close(1u32.into(), b"request timed out");
            return Ok(Incoming::TimedOut);
        }
    };
    Ok(match serde_json::from_slice(&data) {
        Ok(request) => Incoming::Request(send, request),
        Err(e) => Incoming::Malformed(send, format!("malformed request: {e}")),
    })
}

/// Sends `reply` and gives the caller a moment to read it.
pub(crate) async fn reply<Resp: Serialize>(
    connection: &Connection,
    mut send: SendStream,
    reply: &Resp,
) -> Result<(), AcceptError> {
    let payload = serde_json::to_vec(reply).map_err(std::io::Error::other)?;
    send.write_all(&payload)
        .await
        .map_err(std::io::Error::other)?;
    send.finish()?;
    let _ = tokio::time::timeout(LINGER, connection.closed()).await;
    Ok(())
}
