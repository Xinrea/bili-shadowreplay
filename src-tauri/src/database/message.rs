use super::Database;
use super::DatabaseError;
use chrono::Utc;

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct MessageRow {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub read: u8,
    pub created_at: String,
}

// messages
// CREATE TABLE messages (id INTEGER PRIMARY KEY, title TEXT, content TEXT, read INTEGER, created_at TEXT);
impl Database {
    pub async fn new_message(&self, title: &str, content: &str) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query(
            "INSERT INTO messages (title, content, read, created_at) VALUES ($1, $2, 0, $3)",
        )
        .bind(title)
        .bind(content)
        .bind(Utc::now().to_rfc3339())
        .execute(&lock)
        .await?;
        Ok(())
    }

    pub async fn read_message(&self, id: i64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("UPDATE messages SET read = $1 WHERE id = $2")
            .bind(1)
            .bind(id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn delete_message(&self, id: i64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("DELETE FROM messages WHERE id = $1")
            .bind(id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn get_messages(&self) -> Result<Vec<MessageRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, MessageRow>("SELECT * FROM messages;")
            .fetch_all(&lock)
            .await?)
    }
}
