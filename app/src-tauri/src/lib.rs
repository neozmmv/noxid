//! The Tauri shell: exposes the core's [`Node`] to the webview as commands,
//! pushes snapshots and notices as events, and serves video frames over the
//! loopback socket in [`frames`].

mod frames;

use std::path::PathBuf;

use iroh::EndpointId;
use noxid_core::media::{self, Mic, Quality, Screen};
use noxid_core::audio::AudioDevice;
use noxid_core::{Node, NodeConfig, QualityMode, Snapshot, Stream};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, RunEvent};
use tokio::sync::watch;
use tracing::{error, info};

/// What the app remembers between launches, beside the identity key.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Profile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    mic_device: Option<String>,
    #[serde(default)]
    output_device: Option<String>,
    /// `None` until chosen, so the platform default applies.
    #[serde(default)]
    screen_audio: Option<bool>,
}

/// How often voice levels are pushed to the webview for the speaking
/// indicator. Separate from snapshots so it never redraws anything else.
const LEVELS_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

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

/// Saves what the user chose, read back from the node so the file always
/// matches what is in effect.
fn remember(node: &Node) {
    let snapshot = node.snapshot();
    save_profile(&Profile {
        name: snapshot.name,
        mic_device: snapshot.mic_device,
        output_device: snapshot.output_device,
        screen_audio: Some(snapshot.screen_audio),
    });
}

#[tauri::command]
async fn set_name(state: tauri::State<'_, AppState>, name: String) -> CommandResult<()> {
    let node = node(&state).await?;
    node.set_name(&name);
    remember(&node);
    Ok(())
}

#[derive(Serialize)]
struct AudioDevices {
    inputs: Vec<AudioDevice>,
    outputs: Vec<AudioDevice>,
}

#[tauri::command]
async fn audio_devices() -> CommandResult<AudioDevices> {
    let (inputs, outputs) = noxid_core::audio::devices().await;
    Ok(AudioDevices { inputs, outputs })
}

#[tauri::command]
async fn set_mic_device(state: tauri::State<'_, AppState>, device: Option<String>) -> CommandResult<()> {
    let node = node(&state).await?;
    node.set_mic_device(device);
    remember(&node);
    Ok(())
}

#[tauri::command]
async fn set_output_device(state: tauri::State<'_, AppState>, device: Option<String>) -> CommandResult<()> {
    let node = node(&state).await?;
    node.set_output_device(device).await.map_err(|e| format!("{e:#}"))?;
    remember(&node);
    Ok(())
}

#[tauri::command]
async fn set_deafened(state: tauri::State<'_, AppState>, on: bool) -> CommandResult<()> {
    node(&state).await?.set_deafened(on);
    Ok(())
}

#[tauri::command]
async fn set_mic_test(state: tauri::State<'_, AppState>, on: bool) -> CommandResult<()> {
    node(&state).await?.set_mic_test(on);
    Ok(())
}

#[tauri::command]
async fn set_screen_audio(state: tauri::State<'_, AppState>, on: bool) -> CommandResult<()> {
    let node = node(&state).await?;
    node.set_screen_audio(on);
    remember(&node);
    Ok(())
}

#[tauri::command]
async fn set_peer_audio(
    state: tauri::State<'_, AppState>,
    id: String,
    stream: Stream,
    volume: f32,
    muted: bool,
) -> CommandResult<()> {
    let id: EndpointId = id.parse().map_err(err)?;
    node(&state).await?.set_peer_audio(id, stream, volume, muted);
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
    app: String,
    kind: media::Kind,
    width: u32,
    height: u32,
}

/// Everything that can be shared: the displays first, then the windows.
#[tauri::command]
async fn displays() -> CommandResult<Vec<DisplayInfo>> {
    let mut all = media::displays().await.map_err(err)?;
    all.extend(media::windows().await);
    Ok(all
        .into_iter()
        .map(|d| DisplayInfo {
            id: d.id,
            name: d.name,
            app: d.app,
            kind: d.kind,
            width: d.size.width,
            height: d.size.height,
        })
        .collect())
}

/// Starts sharing the display or window with this id, or stops sharing with
/// `None`.
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
/// runs, and voice levels on a fast timer of their own.
fn forward_events(app: tauri::AppHandle, node: Node) {
    {
        let app = app.clone();
        let node = node.clone();
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(LEVELS_EVERY);
            loop {
                tick.tick().await;
                let _ = app.emit("levels", node.levels());
            }
        });
    }
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
                    let profile = load_profile();
                    let config = NodeConfig {
                        secret_key: noxid_core::identity::load_or_create()?,
                        name: profile.name,
                        mic: Mic::Device,
                        start_muted: false,
                        mic_device: profile.mic_device,
                        output_device: profile.output_device,
                    };
                    let node = Node::spawn(config).await?;
                    if let Some(on) = profile.screen_audio {
                        node.set_screen_audio(on);
                    }
                    anyhow::Ok(node)
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
            audio_devices,
            set_mic_device,
            set_output_device,
            set_screen_audio,
            set_peer_audio,
            set_mic_test,
            set_deafened,
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
