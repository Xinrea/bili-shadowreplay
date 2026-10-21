import { invoke } from "../lib/invoker";
import type { StickerElement, TextElement } from "./cover-elements";

// #[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
// pub struct RoomInfo {
//     pub platform: String,
//     pub room_id: String,
//     pub room_title: String,
//     pub room_cover: String,
//     /// Whether the room is live
//     pub status: bool,
// }

// #[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
// pub struct UserInfo {
//     pub user_id: String,
//     pub user_name: String,
//     pub user_avatar: String,
// }

export interface RoomInfo {
  platform: string;
  room_id: string;
  room_title: string;
  room_cover: string;
  status: boolean;
}

export interface UserInfo {
  user_id: string;
  user_name: string;
  user_avatar: string;
}

export interface RecorderInfo {
  room_info: RoomInfo;
  user_info: UserInfo;
  platform_live_id: string;
  live_id: string;
  recording: boolean;
  enabled: boolean;
}

export interface RecorderList {
  count: number;
  recorders: RecorderInfo[];
}

export interface Subtitle {
  open: 0 | 1;
  lan: string;
}

export interface Video {
  title: string;
  filename: string;
  desc: string;
  cid: number;
}

export interface VideoItem {
  id: number;
  room_id: string;
  cover: string;
  file: string;
  length: number;
  size: number;
  status: number;
  bvid: string;
  title: string;
  note: string;
  desc: string;
  tags: string;
  area: number;
  created_at: string;
  platform?: string;
  /** Source recording of a material; empty for imported videos. */
  live_id?: string;
  /** JSON `{ ranges, local_offset }` describing where the material was cut from. */
  source?: string;
  /** Exact length in seconds; 0 for old rows that were never probed. */
  duration?: number;
}

/** A material is one video file; `VideoItem` keeps its historical name. */
export type MaterialItem = VideoItem;

export interface TimelineClip {
  id: string;
  material_id: number;
  /** In point within the material, in seconds. */
  start: number;
  /** Out point within the material, in seconds. */
  end: number;
  /** Of the material's own sound, 0 to 1; full when absent. */
  volume?: number;
}

/** A subtitle anchored to a clip, timed in that clip's material time. */
export interface TimelineSubtitle {
  id: string;
  clip_id: string;
  start: number;
  end: number;
  text: string;
}

/**
 * Something drawn over the main track. Like a subtitle it is anchored to the
 * clip it starts over, in that clip's material time, so it follows the
 * picture when clips move or are trimmed; its length is free and may run on
 * over later clips.
 */
interface OverlayBase {
  id: string;
  clip_id: string;
  /** Where it starts, in the anchor clip's material time. */
  start: number;
  /** Length in project time. */
  duration: number;
}

export interface TextOverlay extends OverlayBase {
  kind: "text";
  /** Drawn as on a cover, on the 1920×1080 picture; its `id` is unused. */
  element: TextElement;
}

export interface StickerOverlay extends OverlayBase {
  kind: "sticker";
  /** As on a cover, except that `src` is a sticker reference such as `builtin:1f600`. */
  element: StickerElement;
}

/** Where a video overlay sits on the 1920×1080 picture. */
export interface VideoPlacement {
  /** Center. */
  x: number;
  y: number;
  /** The height follows the material's aspect ratio. */
  width: number;
  /** Degrees, clockwise. */
  rotation: number;
}

export interface VideoOverlay extends OverlayBase {
  kind: "video";
  material_id: number;
  /** Material time shown at the overlay's start. */
  source_start: number;
  /** 0 to 1. */
  volume: number;
  element: VideoPlacement;
}

export type OverlayItem = TextOverlay | StickerOverlay | VideoOverlay;

/** A track above the main one; items on a track do not overlap in time. */
export interface OverlayTrack {
  id: string;
  items: OverlayItem[];
}

/**
 * Sound heard with the main track, anchored like an overlay to the clip it
 * starts over.
 */
interface AudioBase extends OverlayBase {
  /** Time in the source heard at the item's start. */
  source_start: number;
  /** 0 to 1. */
  volume: number;
  /**
   * Seconds it fades in over from where it is first heard, and out over
   * until where it is last heard; none when absent.
   */
  fade_in?: number;
  fade_out?: number;
}

/** A file of the audio library. */
export interface FileAudio extends AudioBase {
  kind: "file";
  audio_id: number;
}

/** The sound of a material, without its picture. */
export interface MaterialAudio extends AudioBase {
  kind: "material";
  material_id: number;
}

export type AudioItem = FileAudio | MaterialAudio;

/** A track below the main one; items on a track do not overlap in time. */
export interface AudioTrack {
  id: string;
  items: AudioItem[];
}

/** A file of the audio library, shared by every project. */
export interface AudioFile {
  id: number;
  name: string;
  /** Relative to the output directory. */
  file: string;
  /** Seconds. */
  duration: number;
  size: number;
  created_at: string;
}

export interface ProjectTimeline {
  version: number;
  /** The main track: clips play back to back. */
  clips: TimelineClip[];
  subtitles: TimelineSubtitle[];
  /** Drawn over the main track, the bottom one first; a higher track covers lower ones. */
  tracks: OverlayTrack[];
  subtitle_style?: SubtitleStyle | null;
  /** Burn the materials' danmaku when exporting. */
  danmu: boolean;
  /** Heard with the main track, the top one, nearest it, first. */
  audio_tracks: AudioTrack[];
}

/** A text or sticker overlay drawn by the editor, which the export burns in. */
export interface OverlayImage {
  id: string;
  /** PNG data URL. */
  image: string;
  /** Top left on the 1920×1080 picture. */
  x: number;
  y: number;
}

/** A project (切片) as listed by the backend. */
export interface ProjectItem {
  id: number;
  room_id: string;
  platform: string;
  title: string;
  note: string;
  /** Relative to the output directory; empty to use the first material's thumbnail. */
  cover: string;
  /** `ProjectTimeline` JSON. */
  timeline: string;
  /** Upload `Profile` JSON carried over from before uploads existed. */
  profile: string;
  status: number;
  bvid: string;
  created_at: string;
  updated_at: string;
  duration: number;
  clip_count: number;
  /** Cover to show, relative to the output directory. */
  display_cover: string;
  /** False when the project is one untouched material, exported and uploaded as is. */
  needs_render: boolean;
}

export interface GeneratedMaterials {
  materials: VideoItem[];
  project: ProjectItem | null;
}

export interface ExportStarted {
  task: { id: string };
  /** The file being written. */
  path: string;
  /** Where the web client downloads it, relative to the cache directory. */
  download: string | null;
}

export type UploadSource = "material" | "project" | "file";
export type UploadStatus = "draft" | "exporting" | "posting" | "posted" | "failed";

/** A file the static server serves under `base`. */
export interface StaticFile {
  base: "output" | "cache";
  path: string;
}

/** An upload (投稿) as listed by the backend. */
export interface UploadItem {
  id: number;
  source: UploadSource;
  /** Material or project id; 0 for files. */
  source_id: number;
  /** Absolute path of a `file` source. */
  file: string;
  room_id: string;
  /** Bilibili account that posts it. */
  uid: string;
  /** Relative to output; empty to use the source's cover. */
  cover: string;
  /** 4:3 homepage cover, relative to output; empty to let Bilibili crop `cover`. */
  cover43: string;
  /** Profile JSON; may be partial for uploads from before this page. */
  profile: string;
  status: UploadStatus;
  /** Why the last attempt failed. */
  message: string;
  bvid: string;
  created_at: string;
  updated_at: string;
  /** Name of the source; empty when it no longer exists. */
  source_title: string;
  /**
   * `material` when the material is uploaded as is (also a project of one
   * untouched material), `render` when the video is rendered when posting (a
   * project, or a material burning its danmaku), `file`, or `missing` when the
   * source is gone.
   */
  source_state: "material" | "render" | "file" | "missing";
  /** Burn danmaku into the posted video; a project's setting is its own. */
  danmu: boolean;
  /** The material, or one of the project's materials, has saved danmaku. */
  danmu_available: boolean;
  /** Relative to output. */
  display_cover: string;
  /** Video to pick cover frames from, when it can be served. */
  video: StaticFile | null;
}

export interface MaterialPeaks {
  rate: number;
  peaks: number[];
}

export interface Profile {
  videos: Video[];
  cover: string;
  cover43: string | null;
  title: string;
  copyright: 1 | 2;
  tid: number;
  tag: string;
  desc_format_id: number;
  desc: string;
  recreate: number;
  dynamic: string;
  interactive: 0 | 1;
  act_reserve_create: 0 | 1;
  no_disturbance: 0 | 1;
  no_reprint: 0 | 1;
  subtitle: Subtitle;
  dolby: 0 | 1;
  lossless_music: 0 | 1;
  up_selection_reply: boolean;
  up_close_reply: boolean;
  up_close_danmu: boolean;
  web_os: 0 | 1;
}

export function default_profile(): Profile {
  return {
    videos: [],
    cover: "",
    cover43: null,
    title: "",
    copyright: 1,
    tid: 27,
    tag: "",
    desc_format_id: 9999,
    desc: "",
    recreate: -1,
    dynamic: "",
    interactive: 0,
    act_reserve_create: 0,
    no_disturbance: 0,
    no_reprint: 0,
    subtitle: {
      open: 0,
      lan: "",
    },
    dolby: 0,
    lossless_music: 0,
    up_selection_reply: false,
    up_close_danmu: false,
    up_close_reply: false,
    web_os: 0,
  };
}

export interface Config {
  cache: string;
  output: string;
  primary_uid: number;
  live_start_notify: boolean;
  live_end_notify: boolean;
  clip_notify: boolean;
  post_notify: boolean;
  auto_cleanup: boolean;
  auto_subtitle: boolean;
  mcp_enabled: boolean;
  mcp_port: number;
  subtitle_generator_type: string;
  whisper_model: string;
  whisper_prompt: string;
  openai_api_endpoint: string;
  openai_api_key: string;
  clip_name_format: string;
  auto_generate: AutoGenerateConfig;
  status_check_interval: number;
  whisper_language: string;
  webhook_url: string;
  danmu_ass_options: Danmu2AssOptions;
  llm: LlmConfig;
}

export interface McpServerStatus {
  enabled: boolean;
  running: boolean;
  port: number;
  url: string;
  error: string | null;
}

export interface LlmConfig {
  provider: "openai" | "ollama";
  endpoint: string;
  api_key: string;
  model: string;
}

export interface Danmu2AssOptions {
  font_size: number;
  opacity: number;
}

export interface AutoGenerateConfig {
  enabled: boolean;
  encode_danmu: boolean;
}

export interface DiskInfo {
  disk: string;
  total: number;
  free: number;
}

export interface VideoType {
  id: number;
  parent: number;
  parent_name: string;
  name: string;
  description: string;
  desc: string;
  intro_original: string;
  intro_copy: string;
  notice: string;
  copy_right: number;
  show: boolean;
  rank: number;
  children: Children[];
  max_video_count: number;
  request_id: string;
}

export interface Children {
  id: number;
  parent: number;
  parent_name: string;
  name: string;
  description: string;
  desc: string;
  intro_original: string;
  intro_copy: string;
  notice: string;
  copy_right: number;
  show: boolean;
  rank: number;
  max_video_count: number;
  request_id: string;
}

export interface Marker {
  offset: number;
  realtime: number;
  content: string;
}

/** Measurable progress of one processing stage; times are media seconds. */
export interface StageProgress {
  stage: string;
  processed: number;
  total?: number;
  /** Processing speed relative to realtime. */
  speed?: number;
}

export interface ProgressUpdate {
  id: string;
  /** Plain-text form, always present. */
  content: string;
  /** Present when the stage reports measurable progress. */
  progress?: StageProgress;
}

export interface ProgressFinished {
  id: string;
  success: boolean;
  message: string;
}

export interface SubtitleStyle {
  fontName: string;
  fontSize: number;
  fontColor: string;
  outlineColor: string;
  outlineWidth: number;
  alignment: number;
  marginV: number;
  marginL: number;
  marginR: number;
  /** Canonical V4+ Format/Style source, in the 1920 × 1080 script coordinate space. */
  assStyle?: string;
}

export interface Range {
  start: number;
  end: number;
  activated?: boolean;
}

export interface ClipRangeParams {
  title: string;
  note: string;
  cover: string;
  platform: string;
  room_id: string;
  live_id: string;
  ranges: Range[];
  danmu: boolean;
  local_offset: number;
  fix_encoding: boolean;
  transition?: string;
}

export function generateEventId() {
  return Math.random().toString(36).substring(2, 15);
}

/** Cut each range into its own material, optionally in a new project. */
export async function generateMaterials(
  eventId: string,
  params: ClipRangeParams,
  createProject: boolean,
): Promise<GeneratedMaterials> {
  return await invoke<GeneratedMaterials>("generate_materials", { eventId, params, createProject });
}

export interface DanmuEntry {
  ts: number;
  /**
   * Event type, aligned with the recorded LiveEvent "type" field, e.g.
   * "danmu" or "super_chat". Open set: more event types may be added.
   */
  type: string;
  content: string;
  /** Image replacing the text of an emoji danmaku; text remains searchable. */
  emoji_content?: {
    url: string;
    width?: number;
    height?: number;
    emoticon_unique?: string;
  } | null;
  user_name?: string;
  /** Super chat price in CNY. Present for super chats. */
  price?: number;
  /** Super chat pinned duration in seconds. */
  sc_duration?: number;
}

export interface RecorderList {
  count: number;
  recorders: RecorderInfo[];
}
