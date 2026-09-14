use recorder::account::Account;

use super::credentials::CredentialCipher;
use super::Database;
use super::DatabaseError;
use rand::seq::IndexedRandom;

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
    pub async fn migrate_account_credentials(&self) -> Result<(), DatabaseError> {
        let pool = self.pool().await?;
        if !CredentialCipher::ENCRYPTED {
            let encrypted: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE credentials_encrypted = 1)",
            )
            .fetch_one(&pool)
            .await?;
            if encrypted {
                return Err(DatabaseError::EncryptionRequired);
            }
            return Ok(());
        }

        let mut transaction = pool.begin().await?;
        let accounts = sqlx::query_as::<_, AccountRow>(
            "SELECT * FROM accounts WHERE credentials_encrypted = 0",
        )
        .fetch_all(&mut *transaction)
        .await?;

        if accounts.is_empty() {
            let cleanup_pending: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM account_credential_cleanup)")
                    .fetch_one(&mut *transaction)
                    .await?;
            if !cleanup_pending {
                // Skip VACUUM when there are no accounts to migrate or unfinished cleanups to retry.
                return Ok(());
            }
        } else {
            // Commit the cleanup marker together with the encrypted accounts.
            sqlx::query("INSERT OR IGNORE INTO account_credential_cleanup (id) VALUES (1)")
                .execute(&mut *transaction)
                .await?;
        }

        sqlx::query("PRAGMA secure_delete = ON")
            .execute(&mut *transaction)
            .await?;
        for account in accounts {
            sqlx::query(
                "UPDATE accounts SET csrf = $1, cookies = $2, credentials_encrypted = 1
                 WHERE uid = $3 AND platform = $4",
            )
            .bind(self.credentials.encrypt(&account.csrf)?)
            .bind(self.credentials.encrypt(&account.cookies)?)
            .bind(&account.uid)
            .bind(&account.platform)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;

        // Keep the marker until both database and WAL cleanup succeed so an
        // interrupted cleanup is retried on the next startup.
        sqlx::query("VACUUM").execute(&pool).await?;
        let busy: i64 = sqlx::query_scalar("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&pool)
            .await?;
        if busy == 0 {
            sqlx::query("DELETE FROM account_credential_cleanup")
                .execute(&pool)
                .await?;
        }
        Ok(())
    }

    fn decrypt_account(&self, mut account: AccountRow) -> Result<AccountRow, DatabaseError> {
        account.csrf = self.credentials.decrypt(&account.csrf)?;
        account.cookies = self.credentials.decrypt(&account.cookies)?;
        Ok(account)
    }

    pub async fn add_account(&self, account: &AccountRow) -> Result<(), DatabaseError> {
        let lock = self.pool().await?;
        sqlx::query("INSERT INTO accounts (uid, platform, name, avatar, csrf, cookies, created_at, credentials_encrypted) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)").bind(&account.uid).bind(&account.platform).bind(&account.name).bind(&account.avatar).bind(self.credentials.encrypt(&account.csrf)?).bind(self.credentials.encrypt(&account.cookies)?).bind(&account.created_at).bind(CredentialCipher::ENCRYPTED).execute(&lock).await?;

        Ok(())
    }

    pub async fn remove_account(&self, platform: &str, uid: &str) -> Result<(), DatabaseError> {
        let lock = self.pool().await?;
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
        let lock = self.pool().await?;
        sqlx::query_as::<_, AccountRow>("SELECT * FROM accounts")
            .fetch_all(&lock)
            .await?
            .into_iter()
            .map(|account| self.decrypt_account(account))
            .collect()
    }

    pub async fn get_account(
        &self,
        platform: &str,
        uid: &str,
    ) -> Result<AccountRow, DatabaseError> {
        let lock = self.pool().await?;
        let account = sqlx::query_as::<_, AccountRow>(
            "SELECT * FROM accounts WHERE uid = $1 and platform = $2",
        )
        .bind(uid)
        .bind(platform)
        .fetch_one(&lock)
        .await?;
        self.decrypt_account(account)
    }

    pub async fn get_account_by_platform(
        &self,
        platform: &str,
    ) -> Result<AccountRow, DatabaseError> {
        let lock = self.pool().await?;
        let accounts =
            sqlx::query_as::<_, AccountRow>("SELECT * FROM accounts WHERE platform = $1")
                .bind(platform)
                .fetch_all(&lock)
                .await?;
        if accounts.is_empty() {
            return Err(DatabaseError::NotFound);
        }
        // randomly select one account
        let account = accounts
            .choose(&mut rand::rng())
            .ok_or(DatabaseError::NotFound)?;
        self.decrypt_account(account.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "credential-encryption")]
    #[tokio::test]
    async fn cleans_up_plaintext_when_accounts_are_already_encrypted() {
        let path =
            std::env::temp_dir().join(format!("account-cleanup-{}.db", uuid::Uuid::new_v4()));
        let wal_path = path.with_extension("db-wal");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .pragma("wal_autocheckpoint", "0")
                    .pragma("secure_delete", "ON"),
            )
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let cookies = "unfinished-migration-cookies";
        sqlx::query(
            "INSERT INTO accounts VALUES ('123', 'bilibili', 'test', '', 'test-csrf', $1, '', 0)",
        )
        .bind(cookies)
        .execute(&pool)
        .await
        .unwrap();
        let db = Database::new(CredentialCipher::new(&[0; 32]).unwrap());
        db.set(pool.clone()).await;

        // Simulate the committed migration before VACUUM and WAL cleanup run.
        let mut transaction = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO account_credential_cleanup (id) VALUES (1)")
            .execute(&mut *transaction)
            .await
            .unwrap();
        sqlx::query("UPDATE accounts SET csrf = $1, cookies = $2, credentials_encrypted = 1")
            .bind(db.credentials.encrypt("test-csrf").unwrap())
            .bind(db.credentials.encrypt(cookies).unwrap())
            .execute(&mut *transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        assert!(std::fs::read(&wal_path)
            .unwrap()
            .windows(cookies.len())
            .any(|bytes| bytes == cookies.as_bytes()));

        db.migrate_account_credentials().await.unwrap();

        let wal = std::fs::read(&wal_path).unwrap();
        assert!(!wal
            .windows(cookies.len())
            .any(|bytes| bytes == cookies.as_bytes()));
        assert!(!std::fs::read(&path)
            .unwrap()
            .windows(cookies.len())
            .any(|bytes| bytes == cookies.as_bytes()));
        assert_eq!(
            db.get_account("bilibili", "123").await.unwrap().cookies,
            cookies
        );
        // A normal startup after cleanup must not rewrite the database or WAL.
        db.migrate_account_credentials().await.unwrap();
        assert_eq!(std::fs::read(&wal_path).unwrap(), wal);
        pool.close().await;
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn stores_accounts_in_the_selected_mode() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO accounts VALUES ('123', 'bilibili', 'test', '', 'test-csrf', 'test-cookies', '', 0)")
            .execute(&pool).await.unwrap();
        let db = Database::new(CredentialCipher::new(&[0; 32]).unwrap());
        db.set(pool.clone()).await;
        db.migrate_account_credentials().await.unwrap();
        let stored: (String, String, i64) =
            sqlx::query_as("SELECT csrf, cookies, credentials_encrypted FROM accounts")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored.0 != "test-csrf", CredentialCipher::ENCRYPTED);
        assert_eq!(stored.1 != "test-cookies", CredentialCipher::ENCRYPTED);
        assert_eq!(stored.2, i64::from(CredentialCipher::ENCRYPTED));
        db.migrate_account_credentials().await.unwrap();
        let migrated = db.get_account("bilibili", "123").await.unwrap();
        assert_eq!(migrated.csrf, "test-csrf");
        assert_eq!(migrated.cookies, "test-cookies");
        db.add_account(&AccountRow {
            platform: "douyin".into(),
            csrf: String::new(),
            ..migrated
        })
        .await
        .unwrap();
        let account = db.get_account_by_platform("douyin").await.unwrap();
        assert_eq!(account.cookies, "test-cookies");
        assert!(account.csrf.is_empty());
        let accounts = db.get_accounts().await.unwrap();
        assert_eq!(accounts.len(), 2);
        assert!(accounts
            .iter()
            .all(|account| account.cookies == "test-cookies"));
        let stored: Vec<(String, String, i64)> =
            sqlx::query_as("SELECT csrf, cookies, credentials_encrypted FROM accounts")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(stored.iter().all(|(csrf, cookies, encrypted)| {
            *encrypted == i64::from(CredentialCipher::ENCRYPTED)
                && if CredentialCipher::ENCRYPTED {
                    !csrf.is_empty() && cookies != "test-cookies"
                } else {
                    (csrf.is_empty() || csrf == "test-csrf") && cookies == "test-cookies"
                }
        }));
        db.remove_account("douyin", "123").await.unwrap();
        assert_eq!(db.get_accounts().await.unwrap().len(), 1);
        if !CredentialCipher::ENCRYPTED {
            sqlx::query("UPDATE accounts SET credentials_encrypted = 1")
                .execute(&pool)
                .await
                .unwrap();
            assert!(matches!(
                db.migrate_account_credentials().await,
                Err(DatabaseError::EncryptionRequired)
            ));
        }
    }
}
