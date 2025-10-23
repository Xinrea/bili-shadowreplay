use std::str::FromStr;

use recorder::account::Account;
use recorder::platforms::PlatformType;

use super::Database;
use super::DatabaseError;
use chrono::Utc;
use rand::seq::SliceRandom;
use rand::Rng;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct AccountRow {
    pub platform: String,
    pub uid: i64,               // Keep for Bilibili compatibility
    pub id_str: Option<String>, // New field for string IDs like Douyin sec_uid
    pub name: String,
    pub avatar: String,
    pub csrf: String,
    pub cookies: String,
    pub created_at: String,
}

impl AccountRow {
    pub fn to_account(&self) -> Account {
        Account {
            platform: self.platform.clone(),
            id: if let Some(id) = self.id_str.as_ref() {
                id.clone()
            } else {
                self.uid.to_string()
            },
            name: self.name.clone(),
            avatar: self.avatar.clone(),
            csrf: self.csrf.clone(),
            cookies: self.cookies.clone(),
        }
    }
}

// accounts
impl Database {
    // CREATE TABLE accounts (uid INTEGER PRIMARY KEY, name TEXT, avatar TEXT, csrf TEXT, cookies TEXT, created_at TEXT);
    pub async fn add_account(
        &self,
        platform: &str,
        cookies: &str,
    ) -> Result<AccountRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let platform = PlatformType::from_str(platform).unwrap();

        let csrf = match platform {
            PlatformType::BiliBili => {
                cookies
                    .split(';')
                    .map(str::trim)
                    .find_map(|cookie| -> Option<String> {
                        if cookie.starts_with("bili_jct=") {
                            let var_name = &"bili_jct=";
                            Some(cookie[var_name.len()..].to_string())
                        } else {
                            None
                        }
                    })
            }
            _ => Some(String::new()),
        };

        if csrf.is_none() {
            return Err(DatabaseError::InvalidCookies);
        }

        // parse uid and id_str based on platform
        let (uid, id_str) = match platform {
            PlatformType::BiliBili => {
                // For Bilibili, extract numeric uid from cookies
                let uid = (*cookies
                    .split("DedeUserID=")
                    .collect::<Vec<&str>>()
                    .get(1)
                    .unwrap()
                    .split(';')
                    .collect::<Vec<&str>>()
                    .first()
                    .unwrap())
                .to_string()
                .parse::<u64>()
                .map_err(|_| DatabaseError::InvalidCookies)?;
                (uid, None)
            }
            PlatformType::Douyin => {
                // For Douyin, use temporary uid and will set id_str later with real sec_uid
                // Fix: Generate a u32 within the desired range and then cast to u64 to avoid `clippy::cast-sign-loss`.
                let temp_uid = rand::thread_rng().gen_range(10000u64..=i32::MAX as u64);
                (temp_uid, Some(format!("temp_{temp_uid}")))
            }
            PlatformType::Huya => {
                let uid = (*cookies
                    .split("yyuid=")
                    .collect::<Vec<&str>>()
                    .get(1)
                    .unwrap()
                    .split(';')
                    .collect::<Vec<&str>>()
                    .first()
                    .unwrap())
                .to_string()
                .parse::<u64>()
                .map_err(|_| DatabaseError::InvalidCookies)?;
                (uid, None)
            }
            PlatformType::Youtube => {
                // unsupported
                return Err(DatabaseError::InvalidCookies);
            }
        };

        let uid = i64::try_from(uid).map_err(|_| DatabaseError::InvalidCookies)?;

        let account = AccountRow {
            platform: platform.as_str().to_string(),
            uid,
            id_str,
            name: String::new(),
            avatar: String::new(),
            csrf: csrf.unwrap(),
            cookies: cookies.into(),
            created_at: Utc::now().to_rfc3339(),
        };

        sqlx::query("INSERT INTO accounts (uid, platform, id_str, name, avatar, csrf, cookies, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)").bind(uid).bind(&account.platform).bind(&account.id_str).bind(&account.name).bind(&account.avatar).bind(&account.csrf).bind(&account.cookies).bind(&account.created_at).execute(&lock).await?;

        Ok(account)
    }

    pub async fn remove_account(&self, platform: &str, uid: i64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let sql = sqlx::query("DELETE FROM accounts WHERE uid = $1 and platform = $2")
            .bind(uid)
            .bind(platform)
            .execute(&lock)
            .await?;
        if sql.rows_affected() != 1 {
            return Err(DatabaseError::NotFound);
        }
        Ok(())
    }

    pub async fn update_account(
        &self,
        platform: &str,
        uid: i64,
        name: &str,
        avatar: &str,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let sql = sqlx::query(
            "UPDATE accounts SET name = $1, avatar = $2 WHERE uid = $3 and platform = $4",
        )
        .bind(name)
        .bind(avatar)
        .bind(uid)
        .bind(platform)
        .execute(&lock)
        .await?;
        if sql.rows_affected() != 1 {
            return Err(DatabaseError::NotFound);
        }
        Ok(())
    }

    pub async fn update_account_with_id_str(
        &self,
        old_account: &AccountRow,
        new_id_str: &str,
        name: &str,
        avatar: &str,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();

        // If the id_str changed, we need to delete the old record and create a new one
        if old_account.id_str.as_deref() == Some(new_id_str) {
            // id_str is the same, just update name and avatar
            sqlx::query(
                "UPDATE accounts SET name = $1, avatar = $2 WHERE uid = $3 and platform = $4",
            )
            .bind(name)
            .bind(avatar)
            .bind(old_account.uid)
            .bind(&old_account.platform)
            .execute(&lock)
            .await?;
        } else {
            // Delete the old record (for Douyin accounts, we use uid to identify)
            sqlx::query("DELETE FROM accounts WHERE uid = $1 and platform = $2")
                .bind(old_account.uid)
                .bind(&old_account.platform)
                .execute(&lock)
                .await?;

            // Insert the new record with updated id_str
            sqlx::query("INSERT INTO accounts (uid, platform, id_str, name, avatar, csrf, cookies, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
                .bind(old_account.uid)
                .bind(&old_account.platform)
                .bind(new_id_str)
                .bind(name)
                .bind(avatar)
                .bind(&old_account.csrf)
                .bind(&old_account.cookies)
                .bind(&old_account.created_at)
                .execute(&lock)
                .await?;
        }

        Ok(())
    }

    pub async fn get_accounts(&self) -> Result<Vec<AccountRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, AccountRow>("SELECT * FROM accounts")
            .fetch_all(&lock)
            .await?)
    }

    pub async fn get_account(&self, platform: &str, uid: i64) -> Result<AccountRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, AccountRow>(
            "SELECT * FROM accounts WHERE uid = $1 and platform = $2",
        )
        .bind(uid)
        .bind(platform)
        .fetch_one(&lock)
        .await?)
    }

    pub async fn get_account_by_platform(
        &self,
        platform: &str,
    ) -> Result<AccountRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let accounts =
            sqlx::query_as::<_, AccountRow>("SELECT * FROM accounts WHERE platform = $1")
                .bind(platform)
                .fetch_all(&lock)
                .await?;
        if accounts.is_empty() {
            return Err(DatabaseError::NotFound);
        }
        // randomly select one account
        let account = accounts.choose(&mut rand::thread_rng()).unwrap();
        Ok(account.clone())
    }
}
