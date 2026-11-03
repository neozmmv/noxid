// The app's whole conversation with the Rust side: commands, events, and the
// frame socket.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AudioDevices,
  AudioStream,
  CameraInfo,
  DisplayInfo,
  Feed,
  Levels,
  Frame,
  FrameSocket,
  Notice,
  Quality,
  QualityMode,
  Snapshot,
} from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  setName: (name: string) => invoke<void>("set_name", { name }),
  createRoom: (name: string | null) => invoke<void>("create_room", { name }),
  invite: () => invoke<string>("invite"),
  joinRoom: (code: string) => invoke<void>("join_room", { code }),
  leaveRoom: () => invoke<void>("leave_room"),
  kick: (id: string) => invoke<void>("kick", { id }),
  answerKnock: (id: string, approve: boolean) => invoke<void>("answer_knock", { id, approve }),
  setMic: (on: boolean) => invoke<void>("set_mic", { on }),
  displays: () => invoke<DisplayInfo[]>("displays"),
  shareScreen: (display: string | null) => invoke<void>("share_screen", { display }),
  cameras: () => invoke<CameraInfo[]>("cameras"),
  /** Offers files to a member: these paths, or with none, a file picker.
   *  Resolves to how many were offered. */
  sendFiles: (peer: string, paths: string[] = []) => invoke<number>("send_files", { peer, paths }),
  answerTransfer: (id: number, accept: boolean) => invoke<void>("answer_transfer", { id, accept }),
  cancelTransfer: (id: number) => invoke<void>("cancel_transfer", { id }),
  dismissTransfer: (id: number) => invoke<void>("dismiss_transfer", { id }),
  /** Opens the folder a received file was saved in. */
  showTransfer: (id: number) => invoke<void>("show_transfer", { id }),
  /** A still from a camera, in the layout sharePreview uses. */
  cameraPreview: (id: string) => invoke<ArrayBuffer>("camera_preview", { id }),
  /** Turns the camera with this id on, or the camera off with null. */
  shareCamera: (camera: string | null) => invoke<void>("share_camera", { camera }),
  /** A still of a display or window: width, height (u32 LE), then RGBA. */
  sharePreview: (id: string) => invoke<ArrayBuffer>("share_preview", { id }),
  setMaxQuality: (quality: Quality) => invoke<void>("set_max_quality", { quality }),
  setFramerate: (fps: number) => invoke<void>("set_framerate", { fps }),
  /** In kbit/s; `null` for the resolution's default. */
  setBitrate: (kbps: number | null) => invoke<void>("set_bitrate", { kbps }),
  setPeerQuality: (id: string, mode: QualityMode) => invoke<void>("set_peer_quality", { id, mode }),
  audioDevices: () => invoke<AudioDevices>("audio_devices"),
  setMicDevice: (device: string | null) => invoke<void>("set_mic_device", { device }),
  setOutputDevice: (device: string | null) => invoke<void>("set_output_device", { device }),
  setScreenAudio: (on: boolean) => invoke<void>("set_screen_audio", { on }),
  setMicTest: (on: boolean) => invoke<void>("set_mic_test", { on }),
  setDeafened: (on: boolean) => invoke<void>("set_deafened", { on }),
  setPeerAudio: (id: string, stream: AudioStream, volume: number, muted: boolean) =>
    invoke<void>("set_peer_audio", { id, stream, volume, muted }),
};

export const onSnapshot = (handler: (s: Snapshot) => void): Promise<UnlistenFn> =>
  listen<Snapshot>("snapshot", (e) => handler(e.payload));

export const onNotice = (handler: (n: Notice) => void): Promise<UnlistenFn> =>
  listen<Notice>("notice", (e) => handler(e.payload));

export const onLevels = (handler: (l: Levels) => void): Promise<UnlistenFn> =>
  listen<Levels>("levels", (e) => handler(e.payload));

// Frame socket. Every message is one frame: a 44-byte header (endpoint id,
// width, height, colour space, feed) and the I420 planes.
const HEADER_LEN = 44;

type Sink = (frame: Frame) => void;
// Several views can show the same share (a thumbnail while the stage is
// switching, say), so each id has a set of drawers.
const sinks = new Map<string, Set<Sink>>();
const latest = new Map<string, Frame>();

function hex(bytes: Uint8Array): string {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

let socket: WebSocket | null = null;

export async function connectFrames(): Promise<void> {
  if (socket) return;
  const { port, token } = await invoke<FrameSocket>("frame_socket");
  const open = () => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/?token=${token}`);
    ws.binaryType = "arraybuffer";
    ws.onmessage = (event: MessageEvent<ArrayBuffer>) => {
      const buffer = event.data;
      const view = new DataView(buffer);
      const feed: Feed = view.getUint8(41) === 1 ? "camera" : "screen";
      const id = key(hex(new Uint8Array(buffer, 0, 32)), feed);
      const frame: Frame = {
        width: view.getUint32(32, true),
        height: view.getUint32(36, true),
        colorByte: view.getUint8(40),
        data: new Uint8Array(buffer, HEADER_LEN),
      };
      latest.set(id, frame);
      for (const sink of sinks.get(id) ?? []) sink(frame);
    };
    // The backend restarts the socket server with the app, never mid-run, so
    // a drop is a hiccup worth one quiet retry.
    ws.onclose = () => setTimeout(open, 1000);
    socket = ws;
  };
  open();
}

/** Frames are filed by participant and feed: someone can send a screen and a
 *  camera at once. */
function key(id: string, feed: Feed): string {
  return feed === "camera" ? `${id}:camera` : id;
}

/** Registers the drawer for one participant's screen or camera. Returns the
 *  unsubscribe. */
export function onFrames(member: string, feed: Feed, sink: Sink): () => void {
  const id = key(member, feed);
  let set = sinks.get(id);
  if (!set) sinks.set(id, (set = new Set()));
  set.add(sink);
  const last = latest.get(id);
  if (last) sink(last);
  return () => {
    set.delete(sink);
    if (set.size === 0) sinks.delete(id);
  };
}
