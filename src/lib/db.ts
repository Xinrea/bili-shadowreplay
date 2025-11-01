export interface RecorderItem {
  platform: string;
  room_id: string;
  created_at: string;
}

export interface AccountItem {
  platform: string;
  uid: string;
  name: string;
  avatar: string;
  csrf: string;
  cookies: string;
  created_at: string;
}

export interface MessageItem {
  id: number;
  title: string;
  content: string;
  read: number;
  created_at: string;
}

// from RecordRow
export interface RecordItem {
  platform: string;
  title: string;
  parent_id: string;
  live_id: string;
  room_id: string;
  length: number;
  size: number;
  created_at: string;
  cover: string;
}

export interface AccountInfo {
  accounts: AccountItem[];
}

export interface TaskRow {
  id: string;
  task_type: string;
  status: string;
  message: string;
  metadata: string;
  created_at: string;
}
