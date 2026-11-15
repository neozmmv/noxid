//! Profile pictures, passed straight between members and between friends.
//!
//! The roster (or a friend's ping) carries only a picture's hash (blake3,
//! hex). Each node serves its own picture on an ALPN of its own, to room
//! members and friends only; a node that sees a
//! hash it has no copy of asks its owner for it, checks the bytes against the
//! hash, and keeps them on disk under that hash. Someone met before shows up
//! with their picture at once, and a changed picture is a new hash, so a copy
//! is never stale.
//!
//! The picture itself is made by the frontend (cropped, scaled, encoded); this
//! side only stores it and moves it. Anything arriving from a peer is checked
//! for size, hash and an image signature before it is kept, and the frontend
//! shows it with the webview's own image decoder, never anything of ours.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use iroh::EndpointId;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use tracing::{debug, warn};

use crate::identity::state_dir;
use crate::node::Inner;

pub const AVATAR_ALPN: &[u8] = b"noxid-avatar/0";

/// The largest picture stored or accepted. The frontend sends 256px WebP, a
/// few tens of kilobytes; this leaves room without letting a peer fill a disk.
pub const MAX_AVATAR: usize = 512 * 1024;

/// A hash is 32 bytes of blake3, in lowercase hex.
const HASH_LEN: usize = 64;

/// How long a fetch may take, connecting included.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// How long before a failed fetch is tried again. The owner may not have
/// heard we are a member yet, or may be offline for a moment.
const RETRY_AFTER: Duration = Duration::from_secs(10);

/// Whether `s` is a well-formed picture hash. Hashes name files on disk, so
/// nothing else may ever pass for one.
pub fn is_hash(s: &str) -> bool {
    s.len() == HASH_LEN && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Whether `bytes` start like an image the webview can show: WebP, PNG,
/// JPEG or GIF (an animated picture). Not a full check, which the webview's
/// decoder makes anyway; this keeps anything that is plainly not a picture
/// off the disk.
fn looks_like_image(bytes: &[u8]) -> bool {
    let webp = bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP";
    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    let jpeg = bytes.starts_with(&[0xff, 0xd8, 0xff]);
    let gif = bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a");
    webp || png || jpeg || gif
}

fn check(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_AVATAR {
        bail!("a profile picture must be under {} KB", MAX_AVATAR / 1024);
    }
    if !looks_like_image(bytes) {
        bail!("that is not a WebP, PNG, JPEG or GIF picture");
    }
    Ok(())
}

/// The part of a picture to keep, in the source's pixels: a square with its
/// top left corner at (`x`, `y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub side: u32,
}

/// The largest GIF taken in to be cropped. What leaves is far smaller.
pub const MAX_ANIMATED_INPUT: usize = 32 * 1024 * 1024;

/// The largest canvas and the most frames an animated picture may have. Each
/// frame is scaled down as it is decoded, so these bound the work, not the
/// memory.
const MAX_ANIMATED_SIDE: u32 = 4096;
const MAX_ANIMATED_FRAMES: usize = 300;

/// Sizes tried for an animated picture, largest first, each with how many
/// frames it keeps one of. It takes the first that fits in `MAX_AVATAR`.
const ANIMATED_STEPS: [(u32, usize); 6] = [(256, 1), (192, 1), (128, 1), (128, 2), (96, 2), (96, 3)];

/// Crops and scales an animated GIF to a square profile picture, every frame
/// of it, and encodes it as a GIF again: the webview shows a GIF animated,
/// where the frontend's canvas would keep only its first frame.
pub fn animated(bytes: &[u8], crop: Crop) -> Result<Vec<u8>> {
    use image::codecs::gif::{GifDecoder, GifEncoder, Repeat};
    use image::imageops::{self, FilterType};
    use image::{AnimationDecoder, Delay, Frame, ImageDecoder};

    if bytes.len() > MAX_ANIMATED_INPUT {
        bail!("that GIF is over {} MB", MAX_ANIMATED_INPUT / (1024 * 1024));
    }
    let decoder = GifDecoder::new(std::io::Cursor::new(bytes)).context("that is not a GIF")?;
    let (width, height) = decoder.dimensions();
    if width > MAX_ANIMATED_SIDE || height > MAX_ANIMATED_SIDE {
        bail!("that GIF is too large to crop");
    }
    // Kept inside the canvas, whatever was asked.
    let side = crop.side.clamp(1, width.min(height));
    let x = crop.x.min(width - side);
    let y = crop.y.min(height - side);

    let top = ANIMATED_STEPS[0].0;
    let mut frames: Vec<(image::RgbaImage, Delay)> = Vec::new();
    for frame in decoder.into_frames().take(MAX_ANIMATED_FRAMES) {
        let frame = frame.context("that GIF could not be read")?;
        let delay = frame.delay();
        let part = imageops::crop_imm(frame.buffer(), x, y, side, side).to_image();
        frames.push((imageops::resize(&part, top, top, FilterType::Triangle), delay));
    }
    if frames.is_empty() {
        bail!("that GIF has no pictures in it");
    }

    for (size, every) in ANIMATED_STEPS {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new_with_speed(&mut out, 10);
            encoder.set_repeat(Repeat::Infinite)?;
            let picked = frames.chunks(every).map(|chunk| {
                // A skipped frame's time goes to the one kept, so the
                // animation runs at the same speed.
                let ms: u32 = chunk
                    .iter()
                    .map(|(_, delay)| {
                        let (numer, denom) = delay.numer_denom_ms();
                        numer / denom.max(1)
                    })
                    .sum();
                let picture = if size == top {
                    chunk[0].0.clone()
                } else {
                    imageops::resize(&chunk[0].0, size, size, FilterType::Triangle)
                };
                Frame::from_parts(picture, 0, 0, Delay::from_numer_denom_ms(ms, 1))
            });
            encoder.encode_frames(picked)?;
        }
        if out.len() <= MAX_AVATAR {
            return Ok(out);
        }
    }
    bail!(
        "that GIF stays over {} KB even made small; try a shorter one",
        MAX_AVATAR / 1024
    )
}

fn own_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("avatar"))
}

fn cache_path(hash: &str) -> Result<PathBuf> {
    debug_assert!(is_hash(hash));
    Ok(state_dir()?.join("avatars").join(hash))
}

/// Our own picture's hash, if we have one saved.
pub(crate) fn load_own() -> Option<String> {
    let bytes = std::fs::read(own_path().ok()?).ok()?;
    check(&bytes).ok()?;
    Some(hash(&bytes))
}

/// Saves `bytes` as our picture, returning its hash.
pub(crate) fn save_own(bytes: &[u8]) -> Result<String> {
    check(bytes)?;
    let path = own_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, bytes).context("saving the profile picture")?;
    Ok(hash(bytes))
}

pub(crate) fn clear_own() -> Result<()> {
    match std::fs::remove_file(own_path()?) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// The picture with `hash`: ours, or a member's we have a copy of.
pub(crate) fn read(own: Option<&str>, hash: &str) -> Option<Vec<u8>> {
    if !is_hash(hash) {
        return None;
    }
    let path = if own == Some(hash) {
        own_path().ok()?
    } else {
        cache_path(hash).ok()?
    };
    std::fs::read(path).ok()
}

/// What we know about other members' pictures.
#[derive(Debug, Default)]
pub(crate) struct Avatars {
    /// Hashes with a copy on disk, which the frontend may ask for.
    ready: HashSet<String>,
    in_flight: HashSet<String>,
    failed: HashMap<String, Instant>,
}

impl Avatars {
    pub fn is_ready(&self, hash: &str) -> bool {
        self.ready.contains(hash)
    }
}

/// Keeps a copy of every member's and friend's picture: on each change,
/// fetches the ones missing, and every so often retries the ones that failed.
pub(crate) async fn run(inner: Arc<Inner>) {
    let mut changes = inner.subscribe_changes();
    let mut retry = tokio::time::interval(RETRY_AFTER);
    loop {
        tokio::select! {
            changed = changes.changed() => if changed.is_err() { return },
            _ = retry.tick() => {}
        }
        let own_id = inner.own_id().to_string();
        let wanted: Vec<(EndpointId, String)> = inner
            .room
            .lock()
            .unwrap()
            .roster()
            .into_iter()
            .filter(|m| m.id != own_id)
            .filter_map(|m| Some((m.id.parse::<EndpointId>().ok()?, m.avatar?)))
            .chain(inner.friends.avatars())
            .collect();
        for (peer, hash) in wanted {
            {
                let mut avatars = inner.avatars.lock().unwrap();
                if avatars.ready.contains(&hash) || avatars.in_flight.contains(&hash) {
                    continue;
                }
                if avatars.failed.get(&hash).is_some_and(|at| at.elapsed() < RETRY_AFTER) {
                    continue;
                }
                // Met before: the copy on disk is good, its name is its hash.
                if cache_path(&hash).is_ok_and(|p| p.is_file()) {
                    avatars.ready.insert(hash);
                    drop(avatars);
                    inner.bump();
                    inner.friends.refresh();
                    continue;
                }
                avatars.in_flight.insert(hash.clone());
            }
            let inner = inner.clone();
            tokio::spawn(async move {
                let result = tokio::time::timeout(FETCH_TIMEOUT, fetch(&inner, peer, &hash)).await;
                let ok = match result {
                    Ok(Ok(())) => true,
                    Ok(Err(e)) => {
                        debug!(%peer, "profile picture fetch failed: {e:#}");
                        false
                    }
                    Err(_) => {
                        debug!(%peer, "profile picture fetch timed out");
                        false
                    }
                };
                let mut avatars = inner.avatars.lock().unwrap();
                avatars.in_flight.remove(&hash);
                if ok {
                    avatars.failed.remove(&hash);
                    avatars.ready.insert(hash);
                } else {
                    avatars.failed.insert(hash, Instant::now());
                }
                drop(avatars);
                if ok {
                    inner.bump();
                    inner.friends.refresh();
                }
            });
        }
    }
}

/// Asks `peer` for the picture with `hash`, and keeps it if it is that one.
async fn fetch(inner: &Inner, peer: EndpointId, hash: &str) -> Result<()> {
    let connection = inner.endpoint.connect(peer, AVATAR_ALPN).await.context("connect")?;
    let (mut send, mut recv) = connection.open_bi().await?;
    send.write_all(hash.as_bytes()).await?;
    send.finish()?;
    let bytes = recv.read_to_end(MAX_AVATAR).await?;
    connection.close(0u32.into(), b"thanks");
    if bytes.is_empty() {
        bail!("they no longer have that picture");
    }
    check(&bytes)?;
    if self::hash(&bytes) != hash {
        bail!("the picture does not match its hash");
    }
    let path = cache_path(hash)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Written aside and renamed, so a half-written file never passes for a copy.
    let part = path.with_extension("part");
    tokio::fs::write(&part, &bytes).await?;
    tokio::fs::rename(&part, &path).await?;
    Ok(())
}

/// Serves our own picture to members and friends who ask for it by hash.
#[derive(Clone)]
pub(crate) struct AvatarHandler {
    pub inner: Arc<Inner>,
}

impl std::fmt::Debug for AvatarHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvatarHandler").finish_non_exhaustive()
    }
}

impl ProtocolHandler for AvatarHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer = connection.remote_id();
        let member = self.inner.room.lock().unwrap().is_member(&peer.to_string());
        let known = peer != self.inner.own_id() && (member || self.inner.friends.is_friend(&peer));
        if !known {
            connection.close(1u32.into(), b"not a member");
            return Ok(());
        }
        if let Err(e) = tokio::time::timeout(FETCH_TIMEOUT, self.serve(&connection)).await {
            warn!(%peer, "serving a profile picture timed out: {e}");
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
}

impl AvatarHandler {
    async fn serve(&self, connection: &Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let asked = recv.read_to_end(HASH_LEN).await.map_err(std::io::Error::other)?;
        let own = self.inner.own_avatar();
        // Only our current picture is served: never a cached copy of someone
        // else's, and an old hash gets nothing.
        let bytes = match std::str::from_utf8(&asked) {
            Ok(asked) if own.as_deref() == Some(asked) => read(own.as_deref(), asked).unwrap_or_default(),
            _ => Vec::new(),
        };
        send.write_all(&bytes).await.map_err(std::io::Error::other)?;
        send.finish()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny two-frame GIF, red then blue, `side` pixels square.
    fn two_frame_gif(side: u32) -> Vec<u8> {
        use image::codecs::gif::GifEncoder;
        use image::{Delay, Frame, Rgba, RgbaImage};
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            for colour in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                let picture = RgbaImage::from_pixel(side, side, Rgba(colour));
                let frame = Frame::from_parts(picture, 0, 0, Delay::from_numer_denom_ms(100, 1));
                encoder.encode_frame(frame).unwrap();
            }
        }
        out
    }

    #[test]
    fn an_animated_picture_keeps_its_frames() {
        use image::AnimationDecoder;
        use image::codecs::gif::GifDecoder;
        let gif = two_frame_gif(300);
        let out = animated(&gif, Crop { x: 10, y: 20, side: 200 }).unwrap();
        assert!(check(&out).is_ok());
        let frames = GifDecoder::new(std::io::Cursor::new(&out))
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        assert_eq!(frames.len(), 2, "still animated");
        assert_eq!(frames[0].buffer().dimensions(), (256, 256));
    }

    #[test]
    fn a_crop_outside_the_picture_is_kept_inside_it() {
        let gif = two_frame_gif(64);
        let out = animated(&gif, Crop { x: 500, y: 500, side: 9000 }).unwrap();
        assert!(check(&out).is_ok());
    }

    #[test]
    fn only_gifs_are_animated() {
        assert!(animated(b"\x89PNG\r\n\x1a\n....", Crop { x: 0, y: 0, side: 1 }).is_err());
    }

    #[test]
    fn only_hex_hashes_of_the_right_length_pass() {
        let good = hash(b"picture");
        assert!(is_hash(&good));
        assert!(!is_hash(&good[1..]));
        assert!(!is_hash(&good.to_uppercase()));
        assert!(!is_hash(&format!("../{}", &good[3..])));
        assert!(!is_hash(""));
    }

    #[test]
    fn images_pass_and_other_bytes_do_not() {
        assert!(check(b"RIFF\x10\x00\x00\x00WEBPVP8 ").is_ok());
        assert!(check(b"\x89PNG\r\n\x1a\n....").is_ok());
        assert!(check(&[0xff, 0xd8, 0xff, 0xe0]).is_ok());
        assert!(check(b"GIF89a\x01\x00\x01\x00").is_ok());
        assert!(check(b"<svg></svg>").is_err());
        assert!(check(b"").is_err());
        let mut huge = b"\x89PNG\r\n\x1a\n".to_vec();
        huge.resize(MAX_AVATAR + 1, 0);
        assert!(check(&huge).is_err());
    }
}
