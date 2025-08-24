import { invoke } from "../lib/invoker";

export interface RoomInfo {
  live_status: number;
  room_cover: string;
  room_id: number;
  room_keyframe_url: string;
  room_title: string;
  user_id: string;
}

export interface UserInfo {
  user_id: string;
  user_name: string;
  user_sign: string;
  user_avatar: string;
}

export interface RecorderInfo {
  platform: string;
  room_id: number;
  room_info: RoomInfo;
  user_info: UserInfo;
  total_length: number;
  current_live_id: string;
  live_status: boolean;
  is_recording: boolean;
  auto_start: boolean;
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
  room_id: number;
  cover: string;
  file: string;
  length: number;
  size: number;
  status: number;
  bvid: string;
  title: string;
  desc: string;
  tags: string;
  area: number;
  created_at: string;
  platform?: string;
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
  subtitle_generator_type: string;
  whisper_model: string;
  whisper_prompt: string;
  openai_api_endpoint: string;
  openai_api_key: string;
  clip_name_format: string;
  auto_generate: AutoGenerateConfig;
  status_check_interval: number;
  whisper_language: string;
  user_agent: string;
  cleanup_source_flv_after_import: boolean;
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

export interface ProgressUpdate {
  id: string;
  content: string;
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
}

export function parseSubtitleStyle(style: SubtitleStyle): string {
  // Convert hex color to ASS/SSA format (&HBBGGRR)
  function hexToAssColor(hex: string): string {
    if (!hex.startsWith("#")) return hex;
    const r = hex.slice(1, 3);
    const g = hex.slice(3, 5);
    const b = hex.slice(5, 7);
    return `&H${b}${g}${r}`;
  }

  return `FontName=${style.fontName},FontSize=${
    style.fontSize
  },PrimaryColour=${hexToAssColor(
    style.fontColor
  )},OutlineColour=${hexToAssColor(style.outlineColor)},Outline=${
    style.outlineWidth
  },Alignment=${style.alignment},MarginV=${style.marginV},MarginL=${
    style.marginL
  },MarginR=${style.marginR}`;
}

export interface ClipRangeParams {
  title: string;
  cover: string;
  platform: string;
  room_id: number;
  live_id: string;
  range: {
    start: number;
    end: number;
  };
  danmu: boolean;
  local_offset: number;
  fix_encoding: boolean;
}

export function generateEventId() {
  return Math.random().toString(36).substring(2, 15);
}

export async function clipRange(eventId: string, params: ClipRangeParams) {
  return await invoke("clip_range", { eventId, params });
}

export interface DanmuEntry {
  ts: number;
  content: string;
}
