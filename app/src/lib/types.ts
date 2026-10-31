// Mirrors of the Rust types the backend sends (noxid_core::node). Keep the
// two in step: serde's field names and tags are the contract.

export type Quality = "low" | "mid" | "high";

export type QualityMode = { mode: "auto" } | { mode: "pinned"; quality: Quality };

export interface MemberView {
  id: string;
  name: string;
  is_self: boolean;
  is_coordinator: boolean;
  connected: boolean;
  has_audio: boolean;
  has_video: boolean;
  level: number;
  rendition: string | null;
  fps: number;
  width: number;
  height: number;
  rtt_ms: number | null;
  quality: QualityMode;
}

export interface RoomView {
  name: string | null;
  is_coordinator: boolean;
  room_code: string | null;
  members: MemberView[];
}

export interface KnockView {
  id: string;
  name: string;
}

export interface Snapshot {
  id: string;
  name: string;
  room: RoomView | null;
  mic_on: boolean;
  sharing: boolean;
  max_quality: Quality;
  knocks: KnockView[];
}

export type Notice =
  | { kind: "kicked" }
  | { kind: "room_closed" }
  | { kind: "error"; message: string };

export interface DisplayInfo {
  id: string;
  name: string;
  width: number;
  height: number;
}

export interface FrameSocket {
  port: number;
  token: string;
}

/// One decoded frame off the socket.
export interface Frame {
  width: number;
  height: number;
  colorByte: number;
  data: Uint8Array;
}
