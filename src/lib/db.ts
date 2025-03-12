import Database from "@tauri-apps/plugin-sql";

export const db = await Database.load("sqlite:data_v2.db");

// sql: r#"
//     CREATE TABLE records (live_id INTEGER PRIMARY KEY, room_id INTEGER, length INTEGER, size INTEGER, created_at TEXT);
//     CREATE TABLE danmu_statistics (live_id INTEGER PRIMARY KEY, room_id INTEGER, value INTEGER, time_point TEXT);
//     CREATE TABLE messages (id INTEGER PRIMARY KEY, title TEXT, content TEXT, read INTEGER, created_at TEXT);
//     CREATE TABLE videos (id INTEGER PRIMARY KEY, file TEXT, length INTEGER, size INTEGER, status INTEGER, title TEXT, desc TEXT, tags TEXT, area INTEGER);
//     "#,

export interface RecorderItem {
  platform: string;
  room_id: number;
  created_at: string;
}

export interface AccountItem {
  platform: string;
  uid: number;
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
  live_id: string;
  room_id: number;
  length: number;
  size: number;
  created_at: string;
  cover: string;
}

export interface AccountInfo {
  primary_uid: number;
  accounts: AccountItem[];
}