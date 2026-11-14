//! The Tauri shell: exposes the core's [`Node`] to the webview as commands,
//! pushes snapshots and notices as events, and serves video frames over the
//! loopback socket in [`frames`].

mod frames;

use std::path::PathBuf;

use iroh::EndpointId;
use noxid_core::media::{self, Mic, Quality, Screen};
use noxid_core::audio::AudioDevice;
use noxid_core::friends::{FriendsView, RingOutcome, parse_friend_id};
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

/// Sets our profile picture: the request body is the encoded image itself,
/// sent raw rather than as a JSON array of numbers.
#[tauri::command]
async fn set_avatar(state: tauri::State<'_, AppState>, request: tauri::ipc::Request<'_>) -> CommandResult<()> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the picture's bytes".to_string());
    };
    node(&state).await?.set_avatar(Some(bytes)).map_err(err)
}

#[tauri::command]
async fn clear_avatar(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    node(&state).await?.set_avatar(None).map_err(err)
}

/// A profile picture by its hash, as raw bytes.
#[tauri::command]
async fn avatar(state: tauri::State<'_, AppState>, hash: String) -> CommandResult<tauri::ipc::Response> {
    let bytes = node(&state)
        .await?
        .avatar(&hash)
        .ok_or_else(|| "no such picture".to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
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

/// The mic's noise gate threshold, in dBFS. A preference of this machine,
/// which the webview keeps and sends at startup and on every change.
#[tauri::command]
async fn set_sensitivity(state: tauri::State<'_, AppState>, db: f32) -> CommandResult<()> {
    node(&state).await?.set_sensitivity(db);
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

/// Width of the stills in the share picker.
const PREVIEW_WIDTH: u32 = 320;

/// A still of a display or window for the share picker: width and height
/// (u32 LE each) followed by RGBA pixels, as raw bytes rather than JSON.
#[tauri::command]
async fn share_preview(id: String) -> CommandResult<tauri::ipc::Response> {
    let (size, rgba) = media::preview(&id, PREVIEW_WIDTH).await.map_err(err)?;
    let mut bytes = Vec::with_capacity(8 + rgba.len());
    bytes.extend_from_slice(&size.width.to_le_bytes());
    bytes.extend_from_slice(&size.height.to_le_bytes());
    bytes.extend_from_slice(&rgba);
    Ok(tauri::ipc::Response::new(bytes))
}

/// A still from a camera for the camera picker, in the same layout as
/// [`share_preview`].
#[tauri::command]
async fn camera_preview(id: String) -> CommandResult<tauri::ipc::Response> {
    let (size, rgba) = media::camera_preview(&id, PREVIEW_WIDTH).await.map_err(err)?;
    let mut bytes = Vec::with_capacity(8 + rgba.len());
    bytes.extend_from_slice(&size.width.to_le_bytes());
    bytes.extend_from_slice(&size.height.to_le_bytes());
    bytes.extend_from_slice(&rgba);
    Ok(tauri::ipc::Response::new(bytes))
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

/// Offers a file to a member: the given paths (dropped on their picture), or
/// with none, one picked in the system's file picker. Returns how many were
/// offered; picking nothing offers nothing.
#[tauri::command]
async fn send_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    peer: String,
    paths: Vec<String>,
) -> CommandResult<usize> {
    use tauri_plugin_dialog::DialogExt;

    let node = node(&state).await?;
    let paths: Vec<std::path::PathBuf> = if paths.is_empty() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .set_title("Send a file")
            .pick_files(move |picked| {
                let _ = tx.send(picked);
            });
        rx.await
            .map_err(err)?
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| p.into_path().ok())
            .collect()
    } else {
        paths.into_iter().map(Into::into).collect()
    };
    let count = paths.len();
    for path in paths {
        node.send_file(&peer, path).await.map_err(err)?;
    }
    Ok(count)
}

#[tauri::command]
async fn answer_transfer(state: tauri::State<'_, AppState>, id: u64, accept: bool) -> CommandResult<()> {
    node(&state).await?.answer_transfer(id, accept).map_err(err)
}

#[tauri::command]
async fn cancel_transfer(state: tauri::State<'_, AppState>, id: u64) -> CommandResult<()> {
    node(&state).await?.cancel_transfer(id);
    Ok(())
}

#[tauri::command]
async fn dismiss_transfer(state: tauri::State<'_, AppState>, id: u64) -> CommandResult<()> {
    node(&state).await?.dismiss_transfer(id);
    Ok(())
}

/// Opens the folder a received file was saved in, with the file selected.
#[tauri::command]
async fn show_transfer(state: tauri::State<'_, AppState>, id: u64) -> CommandResult<()> {
    let path = node(&state)
        .await?
        .transfer_path(id)
        .ok_or_else(|| "that file was not saved".to_string())?;
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(err)
}

/// Where releases are published; the update button links into it.
const REPO: &str = "https://github.com/neozmmv/noxid";

/// Opens the release page for `tag` (`vMAJOR.MINOR.PATCH`) in the browser.
/// Only a release tag is accepted, so the page cannot open anything else.
#[tauri::command]
fn open_release(tag: String) -> CommandResult<()> {
    let parts: Vec<&str> = tag.strip_prefix('v').unwrap_or("").split('.').collect();
    let valid = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !valid {
        return Err(format!("not a release tag: {tag:?}"));
    }
    tauri_plugin_opener::open_url(format!("{REPO}/releases/tag/{tag}"), None::<&str>).map_err(err)
}

/// WebKitGTK draws through DMA-BUF buffers, which NVIDIA's proprietary driver
/// mishandles: the window opens and stays blank. With that driver loaded it
/// draws through shared memory instead. Anyone can still pick for themselves
/// by setting the variable before launch.
#[cfg(target_os = "linux")]
fn webkit_workarounds() {
    const DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    if std::env::var_os(DMABUF).is_none() && std::path::Path::new("/sys/module/nvidia").exists() {
        // SAFETY: called first thing in `run`, before the log, the runtime or
        // the webview start any thread that could read the environment.
        unsafe { std::env::set_var(DMABUF, "1") };
    }
}

/// The tag of the latest release on GitHub, or None when it can't be had
/// (offline, rate-limited). Asked from here rather than from the webview: on
/// Linux the webview's TLS looks for CA certificates where the distro it was
/// built on keeps them, and in the AppImage that is Ubuntu's path, which
/// Fedora and others don't have. rustls finds the system's own.
#[tauri::command]
async fn latest_release() -> Option<String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }
    let api = REPO.replace("https://github.com/", "https://api.github.com/repos/");
    let response = reqwest::Client::new()
        .get(format!("{api}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        // GitHub refuses requests without one.
        .header("User-Agent", "noxid")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .ok()?;
    Some(response.json::<Release>().await.ok()?.tag_name)
}

/// The cameras this machine has.
#[tauri::command]
async fn cameras() -> Vec<media::CameraInfo> {
    media::cameras().await
}

/// Turns the camera with this id on, or the camera off with `None`.
#[tauri::command]
async fn share_camera(state: tauri::State<'_, AppState>, camera: Option<String>) -> CommandResult<()> {
    let node = node(&state).await?;
    let camera = match camera {
        Some(id) => Some(
            media::resolve_camera(&id)
                .await
                .map_err(|e| format!("The camera could not be opened: {e:#}"))?,
        ),
        None => None,
    };
    node.share_camera(camera).map_err(err)
}

#[tauri::command]
async fn set_framerate(state: tauri::State<'_, AppState>, fps: u32) -> CommandResult<()> {
    let rate = media::Framerate::try_from(fps).map_err(err)?;
    node(&state).await?.set_framerate(rate).map_err(err)
}

/// The top rung's bitrate in kbit/s, or `None` for the resolution's default.
#[tauri::command]
async fn set_bitrate(state: tauri::State<'_, AppState>, kbps: Option<u64>) -> CommandResult<()> {
    node(&state).await?.set_bitrate(kbps.map(|k| k * 1000)).map_err(err)
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

/// A Noxid ID from the webview, checked here whatever the webview checked.
fn friend_id(node: &Node, text: &str) -> CommandResult<EndpointId> {
    parse_friend_id(text, node.id()).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn friends(state: tauri::State<'_, AppState>) -> CommandResult<FriendsView> {
    Ok(node(&state).await?.friends().view())
}

/// Checks a pasted Noxid ID and returns it in its canonical form.
#[tauri::command]
async fn check_friend_id(state: tauri::State<'_, AppState>, text: String) -> CommandResult<String> {
    let node = node(&state).await?;
    Ok(friend_id(&node, &text)?.to_string())
}

#[tauri::command]
async fn send_friend_request(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    let node = node(&state).await?;
    let peer = friend_id(&node, &id)?;
    node.friends().send_request(peer).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn answer_friend_request(state: tauri::State<'_, AppState>, id: String, accept: bool) -> CommandResult<()> {
    let node = node(&state).await?;
    let peer = friend_id(&node, &id)?;
    node.friends().answer(peer, accept).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn cancel_friend_request(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    let node = node(&state).await?;
    let peer = friend_id(&node, &id)?;
    node.friends().cancel(peer).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn remove_friend(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    let node = node(&state).await?;
    let peer = friend_id(&node, &id)?;
    node.friends().remove(peer).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn retry_friend_request(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    let node = node(&state).await?;
    let peer = friend_id(&node, &id)?;
    node.friends().retry(peer);
    Ok(())
}

/// Rings a friend into our room and resolves once they answer, decline, miss
/// it, or we hang up.
#[tauri::command]
async fn call_friend(state: tauri::State<'_, AppState>, id: String) -> CommandResult<RingOutcome> {
    let node = node(&state).await?;
    let peer = friend_id(&node, &id)?;
    node.call_friend(peer).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn hang_up(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    node(&state).await?.friends().hang_up();
    Ok(())
}

#[tauri::command]
async fn answer_ring(state: tauri::State<'_, AppState>, id: String, accept: bool) -> CommandResult<()> {
    let node = node(&state).await?;
    let peer = friend_id(&node, &id)?;
    node.answer_ring(peer, accept).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn dismiss_missed_calls(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    node(&state).await?.friends().dismiss_missed();
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
    let mut friend_changes = node.friends().changes();
    tauri::async_runtime::spawn(async move {
        let _ = app.emit("snapshot", node.snapshot());
        let _ = app.emit("friends", node.friends().view());
        loop {
            tokio::select! {
                changed = changes.changed() => {
                    if changed.is_err() { break; }
                    let _ = app.emit("snapshot", node.snapshot());
                }
                // Apart from the snapshot, so presence and requests never
                // redraw the call.
                changed = friend_changes.changed() => {
                    if changed.is_err() { break; }
                    let _ = app.emit("friends", node.friends().view());
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
    #[cfg(target_os = "linux")]
    webkit_workarounds();

    noxid_core::log::init(noxid_core::log::path().or_else(|| {
        noxid_core::identity::state_dir()
            .ok()
            .map(|dir| dir.join("noxid.log"))
    }));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Without the system's titlebar the window is not given the app's
            // icon on its own, and the taskbar shows none; set it outright.
            if let (Some(window), Some(icon)) = (app.get_webview_window("main"), app.default_window_icon()) {
                let _ = window.set_icon(icon.clone());
            }
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
            cameras,
            camera_preview,
            send_files,
            answer_transfer,
            cancel_transfer,
            dismiss_transfer,
            show_transfer,
            open_release,
            latest_release,
            set_avatar,
            clear_avatar,
            avatar,
            share_camera,
            set_max_quality,
            set_framerate,
            set_bitrate,
            share_preview,
            set_peer_quality,
            audio_devices,
            set_mic_device,
            set_output_device,
            set_screen_audio,
            set_peer_audio,
            set_mic_test,
            set_sensitivity,
            set_deafened,
            friends,
            check_friend_id,
            send_friend_request,
            answer_friend_request,
            cancel_friend_request,
            remove_friend,
            retry_friend_request,
            call_friend,
            hang_up,
            answer_ring,
            dismiss_missed_calls,
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
