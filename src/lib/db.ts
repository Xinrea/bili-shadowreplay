import Database from "@tauri-apps/plugin-sql";

export const db = await Database.load("sqlite:data.db");

// sql: r#"
//     CREATE TABLE records (live_id INTEGER PRIMARY KEY, room_id INTEGER, length INTEGER, size INTEGER, created_at TEXT);
//     CREATE TABLE danmu_statistics (live_id INTEGER PRIMARY KEY, room_id INTEGER, value INTEGER, time_point TEXT);
//     CREATE TABLE messages (id INTEGER PRIMARY KEY, title TEXT, content TEXT, read INTEGER, created_at TEXT);
//     CREATE TABLE videos (id INTEGER PRIMARY KEY, file TEXT, length INTEGER, size INTEGER, status INTEGER, title TEXT, desc TEXT, tags TEXT, area INTEGER);
//     "#,

export interface RecorderItem {
  room_id: number;
  created_at: string;
}

export interface AccountItem {
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
  live_id: number;
  room_id: number;
  length: number;
  size: number;
  created_at: string;
}

export interface AccountInfo {
  primary_uid: number;
  accounts: AccountItem[];
}

//     CREATE TABLE recorders (room_id INTEGER PRIMARY KEY, created_at TEXT);
export class Recorders {
  static async add(room_id: number): Promise<boolean> {
    const result = await db.execute(
      "INSERT into recorders (room_id, created_at) VALUES ($1, $2)",
      [room_id, new Date().toISOString()],
    );
    return result.rowsAffected == 1;
  }

  static async remove(room_id: number): Promise<boolean> {
    const result = await db.execute("DELETE FROM recirders WHERE room_id=$1", [
      room_id,
    ]);
    return result.rowsAffected == 1;
  }

  static async query(): Promise<RecorderItem[]> {
    return await db.select("SELECT * FROM recorders");
  }
}

function parseCookies(cookies_str: string) {
  const cookies = cookies_str.split("; ");
  const cookieObject = {};

  cookies.forEach((cookie) => {
    const [name, value] = cookie.split("=");
    cookieObject[decodeURIComponent(name)] = decodeURIComponent(value);
  });

  return cookieObject;
}

//     CREATE TABLE accounts (uid INTEGER PRIMARY KEY, name TEXT, avatar TEXT, csrf TEXT, cookies TEXT, created_at TEXT);
export class Accounts {
  static async login(): Promise<boolean> {
    const result = (await db.select("SELECT * FROM accounts")) as AccountItem[];
    return result.length > 0;
  }

  static async add(cookies: string): Promise<boolean> {
    const obj = parseCookies(cookies);
    const uid = parseInt(obj["DedeUserID"]);
    const csrf = obj["bili_jct"];
    const result = await db.execute(
      "INSERT OR REPLACE INTO accounts (uid, name, avatar, csrf, cookies, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
      [uid, name, avatar, csrf, cookies, new Date().toISOString],
    );
    return result.rowsAffected == 1;
  }

  static async remove(uid: number): Promise<boolean> {
    const result = await db.execute("DELETE FROM accounts WHERE uid = $1", [
      uid,
    ]);
    return result.rowsAffected == 1;
  }

  static async query(): Promise<AccountItem[]> {
    return await db.select("SELECT * FROM accounts");
  }
}
