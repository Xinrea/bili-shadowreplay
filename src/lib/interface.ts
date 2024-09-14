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
  room_id: number,
  room_info: RoomInfo,
  user_info: UserInfo,
  total_length: number,
  current_ts: number,
  live_status: boolean
}
export interface RecorderList {
  count: number;
  recorders: RecorderInfo[];
}
