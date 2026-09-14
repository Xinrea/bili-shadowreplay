use std::io;

use base64::{engine::general_purpose::STANDARD, Engine};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

pub struct CredentialCipher(LessSafeKey);

fn keyring_error(error: keyring::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "The application cannot access your keychain. Unlock it, allow access, or configure a keychain service, then restart. Clearing login information will not fix a keychain access problem.\n\nSystem error: {error}"
        ),
    )
}

impl CredentialCipher {
    pub const ENCRYPTED: bool = true;

    pub fn new(key: &[u8]) -> io::Result<Self> {
        let key = UnboundKey::new(&AES_256_GCM, key).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The encryption key in your keychain is invalid. Quit and configure your keychain, then restart. Clearing login information will not repair an invalid key.",
            )
        })?;
        Ok(Self(LessSafeKey::new(key)))
    }

    pub async fn load(pool: &sqlx::SqlitePool) -> io::Result<Self> {
        let entry = keyring::Entry::new("cn.vjoi.bilishadowreplay", "account-encryption-key")
            .map_err(keyring_error)?;
        let key = match entry.get_secret() {
            Ok(key) => key,
            Err(keyring::Error::NoEntry) => {
                let encrypted: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM accounts WHERE credentials_encrypted = 1)",
                )
                .fetch_one(pool)
                .await
                .map_err(io::Error::other)?;
                if encrypted {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "The encryption key for your saved accounts is missing. Clear all login information and restart, or quit and configure your keychain.",
                    ));
                }
                let mut key = [0; 32];
                SystemRandom::new()
                    .fill(&mut key)
                    .map_err(|_| io::Error::other("Could not generate account encryption key"))?;
                entry.set_secret(&key).map_err(keyring_error)?;
                key.to_vec()
            }
            Err(error) => return Err(keyring_error(error)),
        };
        let cipher = Self::new(&key)?;
        cipher.validate_accounts(pool).await?;
        Ok(cipher)
    }

    async fn validate_accounts(&self, pool: &sqlx::SqlitePool) -> io::Result<()> {
        let accounts: Vec<(String, String)> =
            sqlx::query_as("SELECT csrf, cookies FROM accounts WHERE credentials_encrypted = 1")
                .fetch_all(pool)
                .await
                .map_err(io::Error::other)?;
        for (csrf, cookies) in accounts {
            for value in [&csrf, &cookies] {
                self.decrypt(value).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "The encryption key in your keychain cannot decrypt your saved accounts. Clear all login information and restart, or quit and restore access to the original keychain.",
                    )
                })?;
            }
        }
        Ok(())
    }

    pub fn encrypt(&self, value: &str) -> io::Result<String> {
        let mut nonce = [0; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| io::Error::other("Could not generate account encryption nonce"))?;
        let mut ciphertext = value.as_bytes().to_vec();
        self.0
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut ciphertext,
            )
            .map_err(|_| io::Error::other("Could not encrypt account credentials"))?;
        Ok(STANDARD.encode([nonce.as_slice(), &ciphertext].concat()))
    }

    pub fn decrypt(&self, value: &str) -> io::Result<String> {
        let mut data = STANDARD.decode(value).map_err(io::Error::other)?;
        let (nonce, ciphertext) = data
            .split_at_mut_checked(NONCE_LEN)
            .ok_or_else(|| io::Error::other("Invalid encrypted account credentials"))?;
        let nonce = Nonce::try_assume_unique_for_key(nonce)
            .map_err(|_| io::Error::other("Invalid account encryption nonce"))?;
        let plaintext = self
            .0
            .open_in_place(nonce, Aad::empty(), ciphertext)
            .map_err(|_| io::Error::other("Could not decrypt account credentials"))?;
        String::from_utf8(plaintext.to_vec()).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_credential_startup_errors() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE accounts (csrf TEXT, cookies TEXT, credentials_encrypted INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO accounts VALUES ('plaintext-csrf', 'plaintext-cookies', 0)")
            .execute(&pool)
            .await
            .unwrap();
        let original = CredentialCipher::new(&[0; 32]).unwrap();
        let wrong = CredentialCipher::new(&[1; 32]).unwrap();
        wrong.validate_accounts(&pool).await.unwrap();

        let csrf = original.encrypt("test-csrf").unwrap();
        let cookies = original.encrypt("test-cookies").unwrap();
        sqlx::query("INSERT INTO accounts VALUES ($1, $2, 1)")
            .bind(&csrf)
            .bind(&cookies)
            .execute(&pool)
            .await
            .unwrap();
        let error = wrong.validate_accounts(&pool).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        original.validate_accounts(&pool).await.unwrap();
        let stored: (String, String) =
            sqlx::query_as("SELECT csrf, cookies FROM accounts WHERE credentials_encrypted = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, (csrf, cookies));

        for error in [
            keyring::Error::NoDefaultStore,
            keyring::Error::NoStorageAccess(Box::new(io::Error::other("Keychain locked"))),
            keyring::Error::PlatformFailure(Box::new(io::Error::other("Access denied"))),
        ] {
            assert_eq!(keyring_error(error).kind(), io::ErrorKind::PermissionDenied);
        }
    }

    #[test]
    fn authenticates_ciphertext() {
        let cipher = CredentialCipher::new(&[0; 32]).unwrap();
        let ciphertext = cipher.encrypt("SESSDATA=test-session").unwrap();
        assert_ne!(ciphertext, cipher.encrypt("SESSDATA=test-session").unwrap());
        let reloaded = CredentialCipher::new(&[0; 32]).unwrap();
        assert_eq!(
            reloaded.decrypt(&ciphertext).unwrap(),
            "SESSDATA=test-session"
        );
        assert_eq!(cipher.decrypt(&cipher.encrypt("").unwrap()).unwrap(), "");
        assert!(CredentialCipher::new(&[1; 32])
            .unwrap()
            .decrypt(&ciphertext)
            .is_err());
        let mut tampered = STANDARD.decode(ciphertext).unwrap();
        tampered[NONCE_LEN] ^= 1;
        assert!(cipher.decrypt(&STANDARD.encode(tampered)).is_err());
    }
}
