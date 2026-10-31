// The app's whole conversation with the Rust side: commands, events, and the
// frame socket.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DisplayInfo,
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
  setMaxQuality: (quality: Quality) => invoke<void>("set_max_quality", { quality }),
  setPeerQuality: (id: string, mode: QualityMode) => invoke<void>("set_peer_quality", { id, mode }),
};

export const onSnapshot = (handler: (s: Snapshot) => void): Promise<UnlistenFn> =>
  listen<Snapshot>("snapshot", (e) => handler(e.payload));

export const onNotice = (handler: (n: Notice) => void): Promise<UnlistenFn> =>
  listen<Notice>("notice", (e) => handler(e.payload));

// Frame socket. Every message is one frame: a 44-byte header (endpoint id,
// width, height, colour space) and the I420 planes.
const HEADER_LEN = 44;

type Sink = (frame: Frame) => void;
const sinks = new Map<string, Sink>();
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
      const id = hex(new Uint8Array(buffer, 0, 32));
      const frame: Frame = {
        width: view.getUint32(32, true),
        height: view.getUint32(36, true),
        colorByte: view.getUint8(40),
        data: new Uint8Array(buffer, HEADER_LEN),
      };
      latest.set(id, frame);
      sinks.get(id)?.(frame);
    };
    // The backend restarts the socket server with the app, never mid-run, so
    // a drop is a hiccup worth one quiet retry.
    ws.onclose = () => setTimeout(open, 1000);
    socket = ws;
  };
  open();
}

/** Registers the drawer for one participant's frames. Returns the unsubscribe. */
export function onFrames(id: string, sink: Sink): () => void {
  sinks.set(id, sink);
  const last = latest.get(id);
  if (last) sink(last);
  return () => {
    if (sinks.get(id) === sink) sinks.delete(id);
  };
}
