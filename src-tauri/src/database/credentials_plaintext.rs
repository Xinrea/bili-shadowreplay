use std::io;

pub struct CredentialCipher;

impl CredentialCipher {
    pub const ENCRYPTED: bool = false;

    pub fn new(_key: &[u8]) -> io::Result<Self> {
        Ok(Self)
    }

    pub fn load() -> io::Result<Self> {
        Self::new(&[])
    }

    pub fn encrypt(&self, value: &str) -> io::Result<String> {
        Ok(value.to_owned())
    }

    pub fn decrypt(&self, value: &str) -> io::Result<String> {
        Ok(value.to_owned())
    }
}
