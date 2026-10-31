//! Everything Noxid does that is not UI: identity, room membership, the mesh,
//! and the media pipeline. Kept free of Tauri so a headless node or a future
//! relay can reuse it.

mod admin;
mod gate;
pub mod identity;
pub mod log;
pub mod media;
mod mesh;
mod node;
pub mod room;

pub use node::{KnockView, MemberView, Node, NodeConfig, Notice, PeerView, QualityMode, RoomView, Snapshot};

/// The broadcast path a member publishes under. Named by endpoint id so a
/// subscriber asks for a path rather than a peer, which is what lets a relay
/// serve the same path later without the subscriber caring who answers.
pub fn broadcast_path(id: &iroh::EndpointId) -> String {
    format!("noxid/0/{id}")
}
