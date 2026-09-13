use std::io;

use base64::{engine::general_purpose::STANDARD, Engine};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

pub struct CredentialCipher(LessSafeKey);

impl CredentialCipher {
    pub const ENCRYPTED: bool = true;

    pub fn new(key: &[u8]) -> io::Result<Self> {
        let key = UnboundKey::new(&AES_256_GCM, key)
            .map_err(|_| io::Error::other("Invalid account encryption key"))?;
        Ok(Self(LessSafeKey::new(key)))
    }

    pub fn load() -> io::Result<Self> {
        let entry = keyring::Entry::new("cn.vjoi.bilishadowreplay", "account-encryption-key")
            .map_err(|error| io::Error::other(error.to_string()))?;
        let key = match entry.get_secret() {
            Ok(key) => key,
            Err(keyring::Error::NoEntry) => {
                let mut key = [0; 32];
                SystemRandom::new()
                    .fill(&mut key)
                    .map_err(|_| io::Error::other("Could not generate account encryption key"))?;
                entry
                    .set_secret(&key)
                    .map_err(|error| io::Error::other(error.to_string()))?;
                key.to_vec()
            }
            Err(error) => return Err(io::Error::other(error.to_string())),
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
