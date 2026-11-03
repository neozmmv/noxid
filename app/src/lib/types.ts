// Mirrors of the Rust types the backend sends (noxid_core::node). Keep the
// two in step: serde's field names and tags are the contract.

export type Quality = "low" | "mid" | "high";

export type QualityMode = { mode: "auto" } | { mode: "pinned"; quality: Quality };

/** Which of a participant's sounds a volume applies to. */
export type AudioStream = "voice" | "screen";

export interface MemberView {
  id: string;
  name: string;
  is_self: boolean;
  is_coordinator: boolean;
  connected: boolean;
  has_audio: boolean;
  has_video: boolean;
  /** Whether their screen share carries the PC's sound. */
  has_screen_audio: boolean;
  rendition: string | null;
  fps: number;
  width: number;
  height: number;
  rtt_ms: number | null;
  quality: QualityMode;
  /** How loud we hear them; local to this machine. Gain, 1 = unchanged. */
  voice_volume: number;
  voice_muted: boolean;
  screen_volume: number;
  screen_muted: boolean;
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
  /** Hearing nobody; the mic is off while it lasts. */
  deafened: boolean;
  sharing: boolean;
  max_quality: Quality;
  /** Pictures per second a share sends: 24, 30 or 60. */
  framerate: number;
  /** Top rung bitrate in bit/s if one was chosen, else `null`. */
  bitrate: number | null;
  /** What the top rung is sent at by default, in bit/s. */
  default_bitrate: number;
  knocks: KnockView[];
  /** Chosen devices; null is the system default. */
  mic_device: string | null;
  output_device: string | null;
  screen_audio: boolean;
  screen_audio_supported: boolean;
  /** Whether a window share sends that window's sound alone (Windows only). */
  window_audio_supported: boolean;
  /** Whether the share in progress is of one window. */
  sharing_window: boolean;
}

export interface AudioDevice {
  id: string;
  name: string;
  default: boolean;
}

export interface AudioDevices {
  inputs: AudioDevice[];
  outputs: AudioDevice[];
}

/** Voice level per endpoint id, 0 to 1. */
export type Levels = Record<string, number>;

export type Notice =
  | { kind: "kicked" }
  | { kind: "room_closed" }
  | { kind: "error"; message: string };

export interface DisplayInfo {
  id: string;
  /** The display's name, or the window's title. */
  name: string;
  /** The application a window belongs to; empty for a display. */
  app: string;
  kind: "display" | "window";
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
