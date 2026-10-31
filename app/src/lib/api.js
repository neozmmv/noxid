// The app's whole conversation with the Rust side: commands, events, and the
// frame socket.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const api = {
  snapshot: () => invoke("snapshot"),
  setName: (name) => invoke("set_name", { name }),
  createRoom: (name) => invoke("create_room", { name }),
  invite: () => invoke("invite"),
  joinRoom: (code) => invoke("join_room", { code }),
  leaveRoom: () => invoke("leave_room"),
  kick: (id) => invoke("kick", { id }),
  answerKnock: (id, approve) => invoke("answer_knock", { id, approve }),
  setMic: (on) => invoke("set_mic", { on }),
  displays: () => invoke("displays"),
  shareScreen: (display) => invoke("share_screen", { display }),
  setMaxQuality: (quality) => invoke("set_max_quality", { quality }),
  setPeerQuality: (id, mode) => invoke("set_peer_quality", { id, mode }),
};

export const onSnapshot = (handler) => listen("snapshot", (e) => handler(e.payload));
export const onNotice = (handler) => listen("notice", (e) => handler(e.payload));

// Frame socket. Every message is one frame: a 44-byte header (endpoint id,
// width, height, colour space) and the I420 planes.
const HEADER_LEN = 44;
const sinks = new Map();
const latest = new Map();

function hex(bytes) {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

let socket = null;

export async function connectFrames() {
  if (socket) return;
  const { port, token } = await invoke("frame_socket");
  const open = () => {
    socket = new WebSocket(`ws://127.0.0.1:${port}/?token=${token}`);
    socket.binaryType = "arraybuffer";
    socket.onmessage = (event) => {
      const buffer = event.data;
      const view = new DataView(buffer);
      const id = hex(new Uint8Array(buffer, 0, 32));
      const frame = {
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
    socket.onclose = () => setTimeout(open, 1000);
  };
  open();
}

// Registers the drawer for one participant's frames. Returns the unsubscribe.
export function onFrames(id, sink) {
  sinks.set(id, sink);
  const last = latest.get(id);
  if (last) sink(last);
  return () => {
    if (sinks.get(id) === sink) sinks.delete(id);
  };
}
