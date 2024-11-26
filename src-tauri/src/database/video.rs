use super::Database;
use super::DatabaseError;
use chrono::Utc;

// CREATE TABLE videos (id INTEGER PRIMARY KEY, room_id INTEGER, cover TEXT, file TEXT, length INTEGER, size INTEGER, status INTEGER, bvid TEXT, title TEXT, desc TEXT, tags TEXT, area INTEGER, created_at TEXT);
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct VideoRow {
    pub id: i64,
    pub room_id: u64,
    pub cover: String,
    pub file: String,
    pub length: i64,
    pub size: i64,
    pub status: i64,
    pub bvid: String,
    pub title: String,
    pub desc: String,
    pub tags: String,
    pub area: i64,
    pub created_at: String,
}

impl Database {
    pub async fn get_videos(&self, room_id: u64) -> Result<Vec<VideoRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(
            sqlx::query_as::<_, VideoRow>("SELECT * FROM videos WHERE room_id = $1;")
                .bind(room_id as i64)
                .fetch_all(&lock)
                .await?,
        )
    }

    pub async fn get_video(&self, id: i64) -> Result<VideoRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(
            sqlx::query_as::<_, VideoRow>("SELECT * FROM videos WHERE id = $1")
                .bind(id)
                .fetch_one(&lock)
                .await?,
        )
    }

    pub async fn update_video(
        &self,
        video_id: i64,
        status: i64,
        bvid: &str,
        title: &str,
        desc: &str,
        tags: &str,
        area: u64,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("UPDATE videos SET status = $1, bvid = $2, title = $3, desc = $4, tags = $5, area = $6 WHERE id = $7")
            .bind(status)
            .bind(bvid)
            .bind(title)
            .bind(desc)
            .bind(tags)
            .bind(area as i64)
            .bind(video_id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn delete_video(&self, id: i64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("DELETE FROM videos WHERE id = $1")
            .bind(id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn add_video(
        &self,
        room_id: u64,
        cover: &str,
        file: &str,
        length: i64,
        size: i64,
        status: i64,
        bvid: &str,
        title: &str,
        desc: &str,
        tags: &str,
        area: i64,
    ) -> Result<VideoRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let mut video = VideoRow {
            id: 0,
            room_id,
            cover: cover.into(),
            file: file.into(),
            length,
            size,
            status,
            bvid: bvid.into(),
            title: title.into(),
            desc: desc.into(),
            tags: tags.into(),
            area,
            created_at: Utc::now().to_rfc3339(),
        };
        let sql = sqlx::query("INSERT INTO videos (room_id, cover, file, length, size, status, bvid, title, desc, tags, area, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)")
            .bind(video.room_id as i64)
            .bind(&video.cover)
            .bind(&video.file)
            .bind(video.length)
            .bind(video.size)
            .bind(video.status)
            .bind(&video.bvid)
            .bind(&video.title)
            .bind(&video.desc)
            .bind(&video.tags)
            .bind(video.area)
            .bind(&video.created_at)
            .execute(&lock)
            .await?;
        video.id = sql.last_insert_rowid();
        Ok(video)
    }
}
