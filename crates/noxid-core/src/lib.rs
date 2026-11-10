//! Everything Noxid does that is not UI: identity, room membership, the mesh,
//! and the media pipeline. Kept free of Tauri so a headless node or a future
//! relay can reuse it.

mod admin;
mod avatar;
pub mod audio;
#[cfg(windows)]
mod dshow;
pub mod files;
mod gate;
pub mod identity;
pub mod log;
pub mod media;
mod mesh;
mod node;
pub mod room;
#[cfg(windows)]
mod still;
#[cfg(windows)]
mod wgc;

pub use node::{
    KnockView, MAX_SCREEN_VOLUME, MAX_VOLUME, MemberView, Node, NodeConfig, Notice, PeerAudio, PeerView, QualityMode,
    RoomView, Snapshot, Stream,
};

/// Where a member publishes their voice. Named by endpoint id so a subscriber
/// asks for a path rather than a peer, which is what lets a relay serve the
/// same path later without the subscriber caring who answers.
pub fn voice_path(id: &iroh::EndpointId) -> String {
    format!("noxid/1/voice/{id}")
}

/// Where a member publishes a screen share: the picture and, when enabled, the
/// PC's sound. A broadcast of its own, so it comes and goes with the share and
/// its sound has a volume of its own on the receiving end.
pub fn screen_path(id: &iroh::EndpointId) -> String {
    format!("noxid/1/screen/{id}")
}

/// Where a member publishes their camera. Separate from the screen share, so
/// either can run without the other and both can run at once.
pub fn camera_path(id: &iroh::EndpointId) -> String {
    format!("noxid/1/camera/{id}")
}
