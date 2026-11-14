// Mirrors of the Rust types the backend sends (noxid_core::node). Keep the
// two in step: serde's field names and tags are the contract.

export type Quality = "low" | "mid" | "high";

export type QualityMode = { mode: "auto" } | { mode: "pinned"; quality: Quality };

/** Which of a participant's sounds a volume applies to. */
export type AudioStream = "voice" | "screen";

export interface MemberView {
  id: string;
  name: string;
  /** Their picture's hash, once there is a copy to show. */
  avatar: string | null;
  is_self: boolean;
  is_coordinator: boolean;
  connected: boolean;
  has_audio: boolean;
  has_video: boolean;
  /** Whether their screen share carries the PC's sound. */
  has_screen_audio: boolean;
  /** Whether their camera is on. */
  has_camera: boolean;
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
  /** Our profile picture's hash. */
  avatar: string | null;
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
  camera_on: boolean;
  /** The camera in use; null is the default one. */
  camera_device: string | null;
  /** Files being offered, sent or received, and recently finished ones. */
  transfers: TransferView[];
}

export type TransferState =
  /** Sending: waiting for them to accept. */
  | { state: "waiting" }
  /** Receiving: waiting for us to accept. */
  | { state: "offered" }
  | { state: "active" }
  | { state: "done" }
  | { state: "declined" }
  /** We stopped it. */
  | { state: "canceled" }
  /** They stopped it. */
  | { state: "canceled_by_peer" }
  | { state: "failed"; message: string };

export type TransferView = {
  id: number;
  peer: string;
  peer_name: string;
  direction: "send" | "receive";
  file_name: string;
  /** Total size and how much has moved, in bytes. */
  size: number;
  done: number;
  /** Receiving, once done: where the file was saved. */
  saved_to: string | null;
} & TransferState;

export interface CameraInfo {
  id: string;
  name: string;
}

/** Which of a participant's pictures a view shows. */
export type Feed = "screen" | "camera";

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

export type FriendEvent =
  | { kind: "request_received"; id: string; name: string }
  | { kind: "friend_added"; id: string; name: string }
  | { kind: "request_declined"; id: string; name: string }
  | { kind: "ringing"; id: string; name: string }
  | { kind: "missed_call"; id: string; name: string };

export type Notice =
  | { kind: "kicked" }
  | { kind: "room_closed" }
  | { kind: "error"; message: string }
  /** The host left and handed us the room. */
  | { kind: "now_host" }
  /** The host left and handed the room to someone else. */
  | { kind: "host_changed"; message: string }
  | { kind: "friend"; message: FriendEvent };

/** What a host leaving does: hand the room on, or end it for everyone. */
export type LeaveMode = "hand_over" | "end";

// Mirrors of noxid_core::friends::view. Times are unix milliseconds.

/** Only what the backend has seen: `unknown` until it has tried. */
export type Presence = "online" | "offline" | "unknown";

export interface FriendView {
  id: string;
  /** What they call themselves: a label, not proof of anything. */
  name: string;
  alias: string | null;
  presence: Presence;
  last_seen: number | null;
  /** We accepted and are waiting for them to confirm. */
  finishing: boolean;
  /** They answered without knowing of the friendship. */
  unaware: boolean;
}

export interface IncomingView {
  id: string;
  /** The name sent with the request. */
  name: string;
  received_at: number;
}

export interface OutgoingView {
  id: string;
  name: string;
  created_at: number;
  /** When they confirmed they have it. */
  delivered_at: number | null;
  /** No longer retried on its own; Retry sends it again. */
  stalled: boolean;
  last_error: string | null;
}

export interface RingView {
  id: string;
  name: string;
  room_name: string | null;
  expires_at: number;
}

export interface CallingView {
  id: string;
  name: string;
  since: number;
}

export interface MissedView {
  id: string;
  name: string;
  at: number;
}

export interface FriendsView {
  friends: FriendView[];
  incoming: IncomingView[];
  outgoing: OutgoingView[];
  rings: RingView[];
  calling: CallingView | null;
  missed: MissedView[];
  /** Why the list is read-only, if it is. */
  problem: string | null;
}

export type RingOutcome =
  | { outcome: "answered" }
  | { outcome: "declined" }
  | { outcome: "no_answer" }
  | { outcome: "hung_up" }
  | { outcome: "busy" }
  | { outcome: "not_friends" }
  | { outcome: "unreachable"; message: string };

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
