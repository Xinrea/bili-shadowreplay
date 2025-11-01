use recorder::account::Account;

use super::Database;
use super::DatabaseError;
use rand::seq::SliceRandom;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct AccountRow {
    pub platform: String,
    pub uid: String,
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
            id: self.uid.clone(),
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
    pub async fn add_account(&self, account: &AccountRow) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("INSERT INTO accounts (uid, platform, name, avatar, csrf, cookies, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)").bind(&account.uid).bind(&account.platform).bind(&account.name).bind(&account.avatar).bind(&account.csrf).bind(&account.cookies).bind(&account.created_at).execute(&lock).await?;

        Ok(())
    }

    pub async fn remove_account(&self, platform: &str, uid: &str) -> Result<(), DatabaseError> {
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

    pub async fn get_accounts(&self) -> Result<Vec<AccountRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, AccountRow>("SELECT * FROM accounts")
            .fetch_all(&lock)
            .await?)
    }

    pub async fn get_account(
        &self,
        platform: &str,
        uid: &str,
    ) -> Result<AccountRow, DatabaseError> {
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
