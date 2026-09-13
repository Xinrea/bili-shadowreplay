use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

pub struct CredentialCipher(LessSafeKey);

impl CredentialCipher {
    pub fn new(key: &[u8]) -> io::Result<Self> {
        let key = UnboundKey::new(&AES_256_GCM, key)
            .map_err(|_| io::Error::other("Invalid account encryption key"))?;
        Ok(Self(LessSafeKey::new(key)))
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let key = match fs::read(path) {
            Ok(key) => key,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut key = [0; 32];
                SystemRandom::new()
                    .fill(&mut key)
                    .map_err(|_| io::Error::other("Could not generate account encryption key"))?;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(path)?;
                file.write_all(&key)?;
                file.sync_all()?;
                key.to_vec()
            }
            Err(error) => return Err(error),
        };
        Self::new(&key)
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

    #[test]
    fn persists_key_and_authenticates_ciphertext() {
        let path = std::env::temp_dir().join(format!("bsr-key-{}", uuid::Uuid::new_v4()));
        let cipher = CredentialCipher::load(&path).unwrap();
        let ciphertext = cipher.encrypt("SESSDATA=test-session").unwrap();
        assert_ne!(ciphertext, cipher.encrypt("SESSDATA=test-session").unwrap());
        let reloaded = CredentialCipher::load(&path).unwrap();
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_file(path).unwrap();
    }
}
