use super::Database;
use super::DatabaseError;
use crate::events::DataChange;

/// A file of the audio library: music, voiceover or another sound imported
/// for the audio tracks of projects, which all share it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct AudioRow {
    pub id: i64,
    pub name: String,
    /// Relative to the output directory.
    pub file: String,
    /// Seconds.
    pub duration: f64,
    pub size: i64,
    pub created_at: String,
}

impl Database {
    /// The audio library, the latest import first.
    pub async fn get_audio_files(&self) -> Result<Vec<AudioRow>, DatabaseError> {
        Ok(
            sqlx::query_as::<_, AudioRow>("SELECT * FROM audio_files ORDER BY id DESC;")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn get_audio_file(&self, id: i64) -> Result<AudioRow, DatabaseError> {
        Ok(
            sqlx::query_as::<_, AudioRow>("SELECT * FROM audio_files WHERE id = $1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn add_audio_file(&self, audio: &AudioRow) -> Result<AudioRow, DatabaseError> {
        let sql = sqlx::query(
            "INSERT INTO audio_files (name, file, duration, size, created_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&audio.name)
        .bind(&audio.file)
        .bind(audio.duration)
        .bind(audio.size)
        .bind(&audio.created_at)
        .execute(&self.pool)
        .await?;
        let audio = AudioRow {
            id: sql.last_insert_rowid(),
            ..audio.clone()
        };
        self.notify_change(DataChange::Audio);
        Ok(audio)
    }

    pub async fn rename_audio_file(&self, id: i64, name: &str) -> Result<(), DatabaseError> {
        let result = sqlx::query("UPDATE audio_files SET name = $1 WHERE id = $2")
            .bind(name)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DatabaseError::NotFound);
        }
        self.notify_change(DataChange::Audio);
        Ok(())
    }

    pub async fn delete_audio_file(&self, id: i64) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM audio_files WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Audio);
        Ok(())
    }
}
