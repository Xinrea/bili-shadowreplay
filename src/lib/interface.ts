export interface RoomInfo {
  live_status: number;
  room_cover_url: string;
  room_id: number;
  room_keyframe_url: string;
  room_title: string;
  user_id: string;
}

export interface UserInfo {
  user_id: string;
  user_name: string;
  user_sign: string;
  user_avatar_url: string;
}

export interface RecorderInfo {
  room_id: number;
  room_info: RoomInfo;
  user_info: UserInfo;
  total_length: number;
  current_ts: number;
  live_status: boolean;
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

export interface Config {
  cache: string;
  output: string;
  primary_uid: number;
  live_start_notify: boolean;
  live_end_notify: boolean;
  clip_notify: boolean;
  post_notify: boolean;
}

export interface DiskInfo {
  disk: string;
  total: number;
  free: number;
}
