//! The Tauri shell: exposes the core's [`Node`] to the webview as commands,
//! pushes snapshots and notices as events, and serves video frames over the
//! loopback socket in [`frames`].

mod frames;

use std::path::PathBuf;

use iroh::EndpointId;
use noxid_core::media::{self, Mic, Quality, Screen};
use noxid_core::{Node, NodeConfig, QualityMode, Snapshot};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, RunEvent};
use tokio::sync::watch;
use tracing::{error, info};

/// What the app remembers between launches, beside the identity key.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Profile {
    #[serde(default)]
    name: String,
}

fn profile_path() -> Option<PathBuf> {
    noxid_core::identity::state_dir()
        .ok()
        .map(|dir| dir.join("profile.json"))
}

fn load_profile() -> Profile {
    profile_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

fn save_profile(profile: &Profile) {
    let Some(path) = profile_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Written beside and renamed over, so a crash mid-write cannot leave a
    // half-written file behind.
    let tmp = path.with_extension("json.tmp");
    if let Ok(data) = serde_json::to_vec_pretty(profile)
        && std::fs::write(&tmp, data).is_ok()
    {
        let _ = std::fs::rename(&tmp, &path);
    }
}

struct AppState {
    /// `None` until the node has started, which takes a few seconds while it
    /// finds a relay.
    node: watch::Receiver<Option<Node>>,
    frames: frames::FrameServer,
}

type CommandResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

async fn node(state: &tauri::State<'_, AppState>) -> CommandResult<Node> {
    let mut rx = state.node.clone();
    loop {
        if let Some(node) = rx.borrow_and_update().clone() {
            return Ok(node);
        }
        rx.changed().await.map_err(|_| "the node failed to start".to_string())?;
    }
}

#[tauri::command]
async fn snapshot(state: tauri::State<'_, AppState>) -> CommandResult<Snapshot> {
    Ok(node(&state).await?.snapshot())
}

#[tauri::command]
fn frame_socket(state: tauri::State<'_, AppState>) -> frames::FrameServer {
    state.frames.clone()
}

#[tauri::command]
async fn set_name(state: tauri::State<'_, AppState>, name: String) -> CommandResult<()> {
    let node = node(&state).await?;
    node.set_name(&name);
    save_profile(&Profile {
        name: node.snapshot().name,
    });
    Ok(())
}

#[tauri::command]
async fn create_room(state: tauri::State<'_, AppState>, name: Option<String>) -> CommandResult<()> {
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    node(&state).await?.create(name).await.map_err(err)
}

#[tauri::command]
async fn invite(state: tauri::State<'_, AppState>) -> CommandResult<String> {
    node(&state).await?.invite(None).map_err(err)
}

#[tauri::command]
async fn join_room(state: tauri::State<'_, AppState>, code: String) -> CommandResult<()> {
    node(&state).await?.join(&code).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn leave_room(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    node(&state).await?.leave().await;
    Ok(())
}

#[tauri::command]
async fn kick(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    node(&state).await?.kick(&id).await.map_err(err)
}

#[tauri::command]
async fn answer_knock(state: tauri::State<'_, AppState>, id: String, approve: bool) -> CommandResult<()> {
    node(&state).await?.answer_knock(&id, approve).map_err(err)
}

#[tauri::command]
async fn set_mic(state: tauri::State<'_, AppState>, on: bool) -> CommandResult<()> {
    node(&state).await?.set_mic(on);
    Ok(())
}

#[derive(Serialize)]
struct DisplayInfo {
    id: String,
    name: String,
    width: u32,
    height: u32,
}

#[tauri::command]
async fn displays() -> CommandResult<Vec<DisplayInfo>> {
    let displays = media::displays().await.map_err(err)?;
    Ok(displays
        .into_iter()
        .map(|d| DisplayInfo {
            id: d.id,
            name: d.name,
            width: d.size.width,
            height: d.size.height,
        })
        .collect())
}

/// Starts sharing the display with this id, or stops sharing with `None`.
#[tauri::command]
async fn share_screen(state: tauri::State<'_, AppState>, display: Option<String>) -> CommandResult<()> {
    let node = node(&state).await?;
    let screen = match display {
        Some(id) => Some(Screen::Display(media::find_display(Some(&id)).await.map_err(err)?)),
        None => None,
    };
    node.share_screen(screen).map_err(err)
}

#[tauri::command]
async fn set_max_quality(state: tauri::State<'_, AppState>, quality: Quality) -> CommandResult<()> {
    node(&state).await?.set_max_quality(quality).map_err(err)
}

#[tauri::command]
async fn set_peer_quality(
    state: tauri::State<'_, AppState>,
    id: String,
    mode: QualityMode,
) -> CommandResult<()> {
    let id: EndpointId = id.parse().map_err(err)?;
    node(&state).await?.set_peer_quality(id, mode);
    Ok(())
}

/// Forwards state changes and notices to the webview for as long as the app
/// runs.
fn forward_events(app: tauri::AppHandle, node: Node) {
    let mut changes = node.changes();
    let mut notices = node.notices();
    tauri::async_runtime::spawn(async move {
        let _ = app.emit("snapshot", node.snapshot());
        loop {
            tokio::select! {
                changed = changes.changed() => {
                    if changed.is_err() { break; }
                    let _ = app.emit("snapshot", node.snapshot());
                }
                notice = notices.recv() => {
                    if let Ok(notice) = notice {
                        let _ = app.emit("notice", notice);
                    }
                }
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    noxid_core::log::init(noxid_core::log::path().or_else(|| {
        noxid_core::identity::state_dir()
            .ok()
            .map(|dir| dir.join("noxid.log"))
    }));

    let app = tauri::Builder::default()
        .setup(|app| {
            let (node_tx, node_rx) = watch::channel(None);
            let frames = tauri::async_runtime::block_on(frames::start(node_rx.clone()))?;
            app.manage(AppState {
                node: node_rx,
                frames,
            });

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result = async {
                    let config = NodeConfig {
                        secret_key: noxid_core::identity::load_or_create()?,
                        name: load_profile().name,
                        mic: Mic::Device,
                        start_muted: false,
                    };
                    Node::spawn(config).await
                }
                .await;
                match result {
                    Ok(node) => {
                        info!(id = %node.id(), "node started");
                        forward_events(handle, node.clone());
                        let _ = node_tx.send(Some(node));
                    }
                    Err(e) => {
                        error!("node failed to start: {e:#}");
                        let _ = handle.emit("notice", noxid_core::Notice::Error(format!("{e:#}")));
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            frame_socket,
            set_name,
            create_room,
            invite,
            join_room,
            leave_room,
            kick,
            answer_knock,
            set_mic,
            displays,
            share_screen,
            set_max_quality,
            set_peer_quality,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the app");

    app.run(|handle, event| {
        // Leave properly on the way out, so the others see us go at once
        // rather than when our connections time out.
        if let RunEvent::ExitRequested { .. } = event {
            let state = handle.state::<AppState>();
            if let Some(node) = state.node.borrow().clone() {
                tauri::async_runtime::block_on(node.shutdown());
            }
        }
    });
}
