# 数据库模块

## 概述

BiliBili ShadowReplay 使用 SQLite 作为主要数据存储，通过 sqlx 提供异步数据库操作。数据库模块位于 `src-tauri/src/database/`。

## 技术栈

- **数据库**: SQLite 3
- **ORM**: sqlx (compile-time checked queries)
- **模式**: WAL (Write-Ahead Logging) 模式
- **迁移**: 自定义迁移系统

## 数据库配置

### 连接池初始化

```rust
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

pub async fn init_database() -> Result<SqlitePool, sqlx::Error> {
    let database_url = "sqlite:data/data_v2.db";

    // 创建连接池
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    // 启用 WAL 模式
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&pool)
        .await?;

    // 启用外键约束
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;

    Ok(pool)
}
```

## 数据表结构

### rooms - 直播间表

```sql
CREATE TABLE rooms (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    platform TEXT NOT NULL,  -- 'bilibili', 'douyin', 'huya', 'kuaishou', 'tiktok'
    url TEXT NOT NULL,
    status TEXT DEFAULT 'offline',  -- 'online', 'offline'
    auto_record INTEGER DEFAULT 0,  -- 0: 关闭, 1: 开启
    quality TEXT DEFAULT 'high',    -- 'high', 'medium', 'low'
    save_path TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### recordings - 录播表

```sql
CREATE TABLE recordings (
    id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL,
    title TEXT,
    start_time INTEGER NOT NULL,
    end_time INTEGER,
    status TEXT DEFAULT 'recording',  -- 'recording', 'stopped', 'error'
    file_path TEXT NOT NULL,
    file_size INTEGER DEFAULT 0,
    duration INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (room_id) REFERENCES rooms(id) ON DELETE CASCADE
);
```

### clips - 切片表

```sql
CREATE TABLE clips (
    id TEXT PRIMARY KEY,
    recording_id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    start_time REAL NOT NULL,  -- 相对于录播的开始时间（秒）
    end_time REAL NOT NULL,
    file_path TEXT NOT NULL,
    file_size INTEGER DEFAULT 0,
    cover_path TEXT,
    status TEXT DEFAULT 'draft',  -- 'draft', 'ready', 'uploading', 'uploaded'
    created_at INTEGER NOT NULL,
    FOREIGN KEY (recording_id) REFERENCES recordings(id) ON DELETE CASCADE
);
```

### accounts - 账号表

```sql
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    platform TEXT NOT NULL,
    username TEXT NOT NULL,
    cookies TEXT,  -- JSON 格式存储
    token TEXT,
    expires_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### tasks - 任务表

```sql
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,  -- 'upload', 'subtitle', 'transcode'
    status TEXT DEFAULT 'pending',  -- 'pending', 'running', 'completed', 'failed'
    progress INTEGER DEFAULT 0,
    target_id TEXT,  -- 关联的 clip_id 或 recording_id
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### settings - 设置表

```sql
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
```

## 数据模型

### Rust 结构体定义

```rust
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub url: String,
    pub status: String,
    pub auto_record: i32,
    pub quality: String,
    pub save_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Recording {
    pub id: String,
    pub room_id: String,
    pub title: Option<String>,
    pub start_time: i64,
    pub end_time: Option<i64>,
    pub status: String,
    pub file_path: String,
    pub file_size: i64,
    pub duration: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Clip {
    pub id: String,
    pub recording_id: String,
    pub title: String,
    pub description: Option<String>,
    pub start_time: f64,
    pub end_time: f64,
    pub file_path: String,
    pub file_size: i64,
    pub cover_path: Option<String>,
    pub status: String,
    pub created_at: i64,
}
```

## 数据库操作

### 直播间操作

```rust
// 创建直播间
pub async fn create_room(
    pool: &SqlitePool,
    room: &Room,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO rooms (id, name, platform, url, status, auto_record, quality, save_path, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
        room.id,
        room.name,
        room.platform,
        room.url,
        room.status,
        room.auto_record,
        room.quality,
        room.save_path,
        room.created_at,
        room.updated_at,
    )
    .execute(pool)
    .await?;

    Ok(())
}

// 获取所有直播间
pub async fn get_all_rooms(pool: &SqlitePool) -> Result<Vec<Room>, sqlx::Error> {
    let rooms = sqlx::query_as!(
        Room,
        r#"SELECT * FROM rooms ORDER BY created_at DESC"#
    )
    .fetch_all(pool)
    .await?;

    Ok(rooms)
}

// 更新直播间状态
pub async fn update_room_status(
    pool: &SqlitePool,
    room_id: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().timestamp();

    sqlx::query!(
        r#"UPDATE rooms SET status = ?, updated_at = ? WHERE id = ?"#,
        status,
        now,
        room_id,
    )
    .execute(pool)
    .await?;

    Ok(())
}

// 删除直播间
pub async fn delete_room(
    pool: &SqlitePool,
    room_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(r#"DELETE FROM rooms WHERE id = ?"#, room_id)
        .execute(pool)
        .await?;

    Ok(())
}
```

### 录播操作

```rust
// 创建录播记录
pub async fn create_recording(
    pool: &SqlitePool,
    recording: &Recording,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO recordings (id, room_id, title, start_time, status, file_path, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
        recording.id,
        recording.room_id,
        recording.title,
        recording.start_time,
        recording.status,
        recording.file_path,
        recording.created_at,
    )
    .execute(pool)
    .await?;

    Ok(())
}

// 获取录播列表
pub async fn get_recordings_by_room(
    pool: &SqlitePool,
    room_id: &str,
) -> Result<Vec<Recording>, sqlx::Error> {
    let recordings = sqlx::query_as!(
        Recording,
        r#"SELECT * FROM recordings WHERE room_id = ? ORDER BY start_time DESC"#,
        room_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(recordings)
}

// 更新录播状态
pub async fn update_recording_status(
    pool: &SqlitePool,
    recording_id: &str,
    status: &str,
    end_time: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE recordings SET status = ?, end_time = ? WHERE id = ?"#,
        status,
        end_time,
        recording_id,
    )
    .execute(pool)
    .await?;

    Ok(())
}
```

### 切片操作

```rust
// 创建切片
pub async fn create_clip(
    pool: &SqlitePool,
    clip: &Clip,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO clips (id, recording_id, title, description, start_time, end_time, file_path, status, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
        clip.id,
        clip.recording_id,
        clip.title,
        clip.description,
        clip.start_time,
        clip.end_time,
        clip.file_path,
        clip.status,
        clip.created_at,
    )
    .execute(pool)
    .await?;

    Ok(())
}

// 获取切片列表
pub async fn get_clips_by_recording(
    pool: &SqlitePool,
    recording_id: &str,
) -> Result<Vec<Clip>, sqlx::Error> {
    let clips = sqlx::query_as!(
        Clip,
        r#"SELECT * FROM clips WHERE recording_id = ? ORDER BY start_time ASC"#,
        recording_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(clips)
}

// 更新切片信息
pub async fn update_clip(
    pool: &SqlitePool,
    clip_id: &str,
    title: &str,
    description: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE clips SET title = ?, description = ? WHERE id = ?"#,
        title,
        description,
        clip_id,
    )
    .execute(pool)
    .await?;

    Ok(())
}
```

## 事务处理

```rust
use sqlx::Transaction;

pub async fn create_clip_with_task(
    pool: &SqlitePool,
    clip: &Clip,
    task_type: &str,
) -> Result<String, sqlx::Error> {
    // 开始事务
    let mut tx: Transaction<'_, sqlx::Sqlite> = pool.begin().await?;

    // 创建切片
    sqlx::query!(
        r#"
        INSERT INTO clips (id, recording_id, title, start_time, end_time, file_path, status, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
        clip.id,
        clip.recording_id,
        clip.title,
        clip.start_time,
        clip.end_time,
        clip.file_path,
        clip.status,
        clip.created_at,
    )
    .execute(&mut *tx)
    .await?;

    // 创建关联任务
    let task_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    sqlx::query!(
        r#"
        INSERT INTO tasks (id, type, status, target_id, created_at, updated_at)
        VALUES (?, ?, 'pending', ?, ?, ?)
        "#,
        task_id,
        task_type,
        clip.id,
        now,
        now,
    )
    .execute(&mut *tx)
    .await?;

    // 提交事务
    tx.commit().await?;

    Ok(task_id)
}
```

## 数据库迁移

迁移系统位于 `src-tauri/src/migration/`：

```rust
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // 创建迁移表
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS migrations (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    // 获取当前版本
    let current_version = get_current_version(pool).await?;

    // 应用新迁移
    for migration in get_pending_migrations(current_version) {
        apply_migration(pool, migration).await?;
    }

    Ok(())
}

async fn apply_migration(
    pool: &SqlitePool,
    migration: Migration,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // 执行迁移 SQL
    sqlx::query(&migration.sql).execute(&mut *tx).await?;

    // 记录迁移
    let now = chrono::Utc::now().timestamp();
    sqlx::query!(
        r#"INSERT INTO migrations (version, applied_at) VALUES (?, ?)"#,
        migration.version,
        now,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(())
}
```

## 查询优化

### 索引

```sql
-- 为常用查询字段创建索引
CREATE INDEX idx_recordings_room_id ON recordings(room_id);
CREATE INDEX idx_recordings_start_time ON recordings(start_time);
CREATE INDEX idx_clips_recording_id ON clips(recording_id);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_target_id ON tasks(target_id);
```

### 复合查询

```rust
// 获取直播间及其最新录播
pub async fn get_rooms_with_latest_recording(
    pool: &SqlitePool,
) -> Result<Vec<RoomWithRecording>, sqlx::Error> {
    let result = sqlx::query_as!(
        RoomWithRecording,
        r#"
        SELECT
            r.*,
            rec.id as recording_id,
            rec.start_time as recording_start_time,
            rec.status as recording_status
        FROM rooms r
        LEFT JOIN (
            SELECT room_id, id, start_time, status,
                   ROW_NUMBER() OVER (PARTITION BY room_id ORDER BY start_time DESC) as rn
            FROM recordings
        ) rec ON r.id = rec.room_id AND rec.rn = 1
        ORDER BY r.created_at DESC
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(result)
}
```

## 最佳实践

1. **使用 sqlx 宏**: 编译时检查 SQL 语法和类型
2. **连接池管理**: 合理配置连接池大小
3. **事务使用**: 对多步操作使用事务保证一致性
4. **索引优化**: 为常用查询字段创建索引
5. **错误处理**: 正确处理数据库错误
6. **数据验证**: 在插入前验证数据
7. **定期清理**: 清理过期或无用数据