use chrono::Utc;
use custom_error::custom_error;
use sqlx::Pool;
use sqlx::Sqlite;
use tokio::sync::RwLock;

pub struct Database {
    db: RwLock<Option<Pool<Sqlite>>>,
}

/// Recorder in database is pretty simple
/// because many room infos are collected in realtime
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct RecorderRow {
    pub room_id: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct AccountRow {
    pub uid: u64,
    pub name: String,
    pub avatar: String,
    pub csrf: String,
    pub cookies: String,
    pub created_at: String,
}

custom_error! { pub DatabaseError
    InsertError = "Entry insert failed",
    NotFoundError = "Entry not found",
    InvalidCookiesError = "Cookies are invalid",
    DBError {err: sqlx::Error } = "DB error: {err}",
    SQLError { sql: String } = "SQL is incorret: {sql}"
}

impl From<DatabaseError> for String {
    fn from(value: DatabaseError) -> Self {
        value.to_string()
    }
}

impl From<sqlx::Error> for DatabaseError {
    fn from(value: sqlx::Error) -> Self {
        DatabaseError::DBError { err: value }
    }
}

impl Database {
    pub fn new() -> Database {
        Database {
            db: RwLock::new(None),
        }
    }

    /// db *must* be set in tauri setup
    pub async fn set(&self, p: Pool<Sqlite>) {
        *self.db.write().await = Some(p);
    }
}

// recorders
impl Database {
    pub async fn add_recorder(&self, room_id: u64) -> Result<RecorderRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let recorder = RecorderRow {
            room_id,
            created_at: Utc::now().to_rfc3339(),
        };
        let _ = sqlx::query("INSERT INTO recorders (room_id, created_at) VALUES ($1, $2)")
            .bind(room_id as i64)
            .bind(&recorder.created_at)
            .execute(&lock)
            .await?;
        Ok(recorder)
    }

    pub async fn remove_recorder(&self, room_id: u64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let sql = sqlx::query("DELETE FROM recorders WHERE room_id = $1")
            .bind(room_id as i64)
            .execute(&lock)
            .await?;
        if sql.rows_affected() != 1 {
            return Err(DatabaseError::NotFoundError);
        }
        Ok(())
    }

    pub async fn get_recorders(&self) -> Result<Vec<RecorderRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecorderRow>("SELECT * FROM recorders")
            .fetch_all(&lock)
            .await?)
    }
}

// accounts
impl Database {
    // CREATE TABLE accounts (uid INTEGER PRIMARY KEY, name TEXT, avatar TEXT, csrf TEXT, cookies TEXT, created_at TEXT);
    pub async fn add_account(&self, cookies: &str) -> Result<AccountRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        // parse cookies
        let csrf =
            cookies
                .split(';')
                .map(|cookie| cookie.trim())
                .find_map(|cookie| -> Option<String> {
                    match cookie.starts_with("bili_jct=") {
                        true => {
                            let var_name = &"bili_jct=";
                            Some(cookie[var_name.len()..].to_string())
                        }
                        false => None,
                    }
                });
        if csrf.is_none() {
            return Err(DatabaseError::InvalidCookiesError);
        }
        // parse uid
        let uid = cookies
            .split("DedeUserID=")
            .collect::<Vec<&str>>()
            .get(1)
            .unwrap()
            .split(";")
            .collect::<Vec<&str>>()
            .first()
            .unwrap()
            .to_string()
            .parse::<u64>()
            .map_err(|_| DatabaseError::InvalidCookiesError)?;
        let account = AccountRow {
            uid,
            name: "".into(),
            avatar: "".into(),
            csrf: csrf.unwrap(),
            cookies: cookies.into(),
            created_at: Utc::now().to_rfc3339(),
        };

        sqlx::query("INSERT INTO accounts (uid, name, avatar, csrf, cookies, created_at) VALUES ($1, $2, $3, $4, $5, $6)").bind(account.uid as i64).bind(&account.name).bind(&account.avatar).bind(&account.csrf).bind(&account.cookies).bind(&account.created_at).execute(&lock).await?;

        Ok(account)
    }

    pub async fn remove_account(&self, uid: u64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let sql = sqlx::query("DELETE FROM accounts WHERE uid = $1")
            .bind(uid as i64)
            .execute(&lock)
            .await?;
        if sql.rows_affected() != 1 {
            return Err(DatabaseError::NotFoundError);
        }
        Ok(())
    }

    pub async fn update_account(
        &self,
        uid: u64,
        name: &str,
        avatar: &str,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let sql = sqlx::query("UPDATE accounts SET name = $1, avatar = $2 WHERE uid = $3")
            .bind(name)
            .bind(avatar)
            .bind(uid as i64)
            .execute(&lock)
            .await?;
        if sql.rows_affected() != 1 {
            return Err(DatabaseError::NotFoundError);
        }
        Ok(())
    }

    pub async fn get_accounts(&self) -> Result<Vec<AccountRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, AccountRow>("SELECT * FROM accounts")
            .fetch_all(&lock)
            .await?)
    }

    pub async fn get_account(&self, uid: u64) -> Result<AccountRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(
            sqlx::query_as::<_, AccountRow>("SELECT * FROM accounts WHERE uid = $1")
                .bind(uid as i64)
                .fetch_one(&lock)
                .await?,
        )
    }
}

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
