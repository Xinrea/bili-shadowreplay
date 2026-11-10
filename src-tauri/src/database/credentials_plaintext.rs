use std::io;

pub struct CredentialCipher;

impl CredentialCipher {
    pub const ENCRYPTED: bool = false;

    pub fn new(_key: &[u8]) -> io::Result<Self> {
        Ok(Self)
    }

    pub async fn load(pool: &sqlx::SqlitePool) -> io::Result<Self> {
        if !super::storage::encrypted_storage_credentials(pool)
            .await
            .map_err(io::Error::other)?
            .is_empty()
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                "Database contains encrypted storage credentials; enable the credential-encryption feature"));
        }
        Self::new(&[])
    }

    pub fn encrypt(&self, value: &str) -> io::Result<String> {
        Ok(value.to_owned())
    }

    pub fn decrypt(&self, value: &str) -> io::Result<String> {
        Ok(value.to_owned())
    }
}
