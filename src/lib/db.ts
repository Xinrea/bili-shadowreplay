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
  endpoint?: string | null;
  access_token?: string | null;
  token_expires_at?: string | null;
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

export interface SummaryHighlight {
  title: string;
  start_seconds: number;
  end_seconds: number;
  reason: string;
  excerpt: string;
}

export interface RecordSummary {
  id: number;
  platform: string;
  room_id: string;
  live_id: string;
  status: "pending" | "processing" | "success" | "failed";
  stage: string;
  subtitle_srt: string | null;
  subtitle_text: string | null;
  summary_markdown: string | null;
  highlights_json: string | null;
  model_provider: string | null;
  model_name: string | null;
  prompt_version: number;
  source_duration: number | null;
  error_message: string | null;
  task_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface RecordSummaryStatus {
  platform: string;
  room_id: string;
  live_id: string;
  status: RecordSummary["status"];
  stage: string;
}
