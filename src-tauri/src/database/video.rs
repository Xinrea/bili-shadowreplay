use super::Database;
use super::DatabaseError;
use crate::events::DataChange;

/// A material (素材): one video file in the output directory, usually cut from
/// a recording. Projects (切片) arrange materials; the upload columns remain for
/// materials posted directly by the assistant and MCP tools.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct VideoRow {
    pub id: i64,
    pub room_id: String,
    pub cover: String,
    pub file: String,
    pub note: String,
    pub length: i64,
    pub size: i64,
    pub status: i64,
    pub bvid: String,
    pub title: String,
    pub desc: String,
    pub tags: String,
    pub area: i64,
    pub created_at: String,
    pub platform: String,
    /// Source recording; empty for imported and derived videos.
    #[sqlx(default)]
    #[serde(default)]
    pub live_id: String,
    /// [`MaterialSource`] JSON, empty when unknown.
    #[sqlx(default)]
    #[serde(default)]
    pub source: String,
    /// Exact length in seconds, `0.0` for rows created before materials existed.
    #[sqlx(default)]
    #[serde(default)]
    pub duration: f64,
}

/// Where a material was cut from, in seconds from the recording's playlist start.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MaterialSource {
    pub ranges: Vec<crate::ffmpeg::Range>,
    #[serde(default)]
    pub local_offset: i64,
}

impl Database {
    pub async fn get_videos(&self, room_id: &str) -> Result<Vec<VideoRow>, DatabaseError> {
        let videos = sqlx::query_as::<_, VideoRow>("SELECT * FROM videos WHERE room_id = $1;")
            .bind(room_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(videos)
    }

    pub async fn get_video(&self, id: i64) -> Result<VideoRow, DatabaseError> {
        Ok(
            sqlx::query_as::<_, VideoRow>("SELECT * FROM videos WHERE id = $1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn update_video(&self, video_row: &VideoRow) -> Result<(), DatabaseError> {
        sqlx::query("UPDATE videos SET status = $1, bvid = $2, title = $3, desc = $4, tags = $5, area = $6, note = $7 WHERE id = $8")
            .bind(video_row.status)
            .bind(&video_row.bvid)
            .bind(&video_row.title)
            .bind(&video_row.desc)
            .bind(&video_row.tags)
            .bind(video_row.area)
            .bind(&video_row.note)
            .bind(video_row.id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Videos);
        Ok(())
    }

    pub async fn delete_video(&self, id: i64) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM videos WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Videos);
        Ok(())
    }

    pub async fn add_video(&self, video: &VideoRow) -> Result<VideoRow, DatabaseError> {
        let sql = sqlx::query("INSERT INTO videos (room_id, cover, file, note, length, size, status, bvid, title, desc, tags, area, created_at, platform, live_id, source, duration) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)")
            .bind(&video.room_id)
            .bind(&video.cover)
            .bind(&video.file)
            .bind(&video.note)
            .bind(video.length)
            .bind(video.size)
            .bind(video.status)
            .bind(&video.bvid)
            .bind(&video.title)
            .bind(&video.desc)
            .bind(&video.tags)
            .bind(video.area)
            .bind(&video.created_at)
            .bind(&video.platform)
            .bind(&video.live_id)
            .bind(&video.source)
            .bind(video.duration)
            .execute(&self.pool)
            .await?;
        let video = VideoRow {
            id: sql.last_insert_rowid(),
            ..video.clone()
        };
        self.notify_change(DataChange::Videos);
        Ok(video)
    }

    pub async fn update_video_cover(&self, id: i64, cover: &str) -> Result<(), DatabaseError> {
        sqlx::query("UPDATE videos SET cover = $1 WHERE id = $2")
            .bind(cover)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Videos);
        Ok(())
    }

    pub async fn get_all_videos(&self) -> Result<Vec<VideoRow>, DatabaseError> {
        let videos =
            sqlx::query_as::<_, VideoRow>("SELECT * FROM videos ORDER BY created_at DESC;")
                .fetch_all(&self.pool)
                .await?;
        Ok(videos)
    }

    pub async fn update_video_duration(&self, id: i64, duration: f64) -> Result<(), DatabaseError> {
        sqlx::query("UPDATE videos SET duration = $1 WHERE id = $2")
            .bind(duration)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Videos that existed before materials and still need their own project.
    pub async fn get_videos_needing_project(&self) -> Result<Vec<VideoRow>, DatabaseError> {
        Ok(sqlx::query_as::<_, VideoRow>(
            "SELECT * FROM videos WHERE needs_project = 1 ORDER BY id;",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn clear_video_needs_project(&self, id: i64) -> Result<(), DatabaseError> {
        sqlx::query("UPDATE videos SET needs_project = 0 WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_video_cover(&self, id: i64) -> Result<String, DatabaseError> {
        let video = sqlx::query_as::<_, VideoRow>("SELECT * FROM videos WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(video.cover)
    }
}
