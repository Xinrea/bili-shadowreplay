use recorder::platforms::bilibili::api::UserInfoCache;
use recorder::UserInfo;
use sqlx::FromRow;

use super::{Database, DatabaseError};

#[derive(Debug, FromRow)]
struct BilibiliUserInfoRow {
    user_id: String,
    user_name: String,
    user_avatar: String,
}

impl Database {
    async fn get_bilibili_user_info(
        &self,
        user_id: &str,
    ) -> Result<Option<UserInfo>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let row = sqlx::query_as::<_, BilibiliUserInfoRow>(
            "SELECT user_id, user_name, user_avatar
             FROM bilibili_user_profiles
             WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&lock)
        .await?;

        Ok(row.map(|row| UserInfo {
            user_id: row.user_id,
            user_name: row.user_name,
            user_avatar: row.user_avatar,
        }))
    }

    async fn save_bilibili_user_info(&self, user_info: &UserInfo) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query(
            "INSERT INTO bilibili_user_profiles
                (user_id, user_name, user_avatar, updated_at)
             VALUES ($1, $2, $3, datetime('now'))
             ON CONFLICT(user_id) DO UPDATE SET
                user_name = excluded.user_name,
                user_avatar = excluded.user_avatar,
                updated_at = excluded.updated_at",
        )
        .bind(&user_info.user_id)
        .bind(&user_info.user_name)
        .bind(&user_info.user_avatar)
        .execute(&lock)
        .await?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl UserInfoCache for Database {
    async fn get_user_info(&self, user_id: &str) -> Result<Option<UserInfo>, String> {
        self.get_bilibili_user_info(user_id)
            .await
            .map_err(|error| error.to_string())
    }

    async fn save_user_info(&self, user_info: &UserInfo) -> Result<(), String> {
        self.save_bilibili_user_info(user_info)
            .await
            .map_err(|error| error.to_string())
    }
}
