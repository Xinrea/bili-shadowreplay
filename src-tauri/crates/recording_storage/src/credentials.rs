use crate::{S3Credentials, WebDavCredentials};
use serde::{Deserialize, Serialize};
use std::io;

/// Untagged to preserve the existing S3 settings and encrypted credential format.
/// Each variant rejects unknown fields, including mixed credential types.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StorageCredentials {
    S3(S3Credentials),
    WebDav(WebDavCredentials),
}

impl StorageCredentials {
    pub fn validate(&self) -> io::Result<()> {
        match self {
            Self::S3(value) => value.validate(),
            Self::WebDav(value) => value.validate(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::S3(_) => "s3",
            Self::WebDav(_) => "webdav",
        }
    }

    pub fn s3(&self) -> io::Result<&S3Credentials> {
        match self {
            Self::S3(value) => Ok(value),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected S3 credentials",
            )),
        }
    }

    pub fn webdav(&self) -> io::Result<&WebDavCredentials> {
        match self {
            Self::WebDav(value) => Ok(value),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected WebDAV credentials",
            )),
        }
    }
}

impl From<S3Credentials> for StorageCredentials {
    fn from(value: S3Credentials) -> Self {
        Self::S3(value)
    }
}

impl From<WebDavCredentials> for StorageCredentials {
    fn from(value: WebDavCredentials) -> Self {
        Self::WebDav(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_formats_remain_compatible_and_cannot_be_mixed() {
        let old = serde_json::json!({"access_key_id":"access","secret_access_key":"secret","session_token":"token"});
        let decoded: StorageCredentials = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(decoded.kind(), "s3");
        assert_eq!(serde_json::to_value(&decoded).unwrap(), old);
        assert!(!format!("{decoded:?}").contains("secret"));
        let dav = serde_json::json!({"username":"user","password":" password "});
        let decoded: StorageCredentials = serde_json::from_value(dav.clone()).unwrap();
        assert_eq!(decoded.kind(), "webdav");
        assert_eq!(serde_json::to_value(decoded).unwrap(), dav);
        assert!(serde_json::from_value::<StorageCredentials>(serde_json::json!({
            "username":"user", "password":"pass", "access_key_id":"id", "secret_access_key":"secret"
        })).is_err());
    }
}
