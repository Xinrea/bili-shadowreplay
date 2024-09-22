use std::str::FromStr;

use chrono::DateTime;
use chrono::Utc;
use sqlx::Pool;
use sqlx::Row;
use sqlx::Sqlite;
use sqlx::Value;
use sqlx::ValueRef;
use tokio::sync::RwLock;

pub struct Database {
    db: RwLock<Option<Pool<Sqlite>>>,
}

#[derive(Debug)]
pub struct RecorderRow {
    room_id: u64,
    created_at: DateTime<Utc>,
}

impl Database {
    pub fn new() -> Database {
        Database {
            db: RwLock::new(None),
        }
    }

    pub async fn set(&self, p: Pool<Sqlite>) {
        *self.db.write().await = Some(p);
    }

    pub async fn get_recorders(&self) -> Vec<RecorderRow> {
        let lock = self.db.read().await.clone().unwrap();
        let query = sqlx::query("SELECT * FROM recorders");
        let mut ret = Vec::new();
        if let Ok(rows) = query.fetch_all(&lock).await {
            for r in rows {
                let room_id = r
                    .try_get_raw(0)
                    .unwrap()
                    .to_owned()
                    .try_decode::<u64>()
                    .unwrap();
                let time = r
                    .try_get_raw(1)
                    .unwrap()
                    .to_owned()
                    .try_decode::<String>()
                    .unwrap();
                ret.push(RecorderRow {
                    room_id,
                    created_at: DateTime::from_str(&time).unwrap(),
                })
            }
        }
        ret
    }
}
