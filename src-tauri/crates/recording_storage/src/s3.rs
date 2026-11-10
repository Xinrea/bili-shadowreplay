//! S3-compatible object storage. Namespace configuration and explicitly supplied
//! credentials are separate. Buckets must already exist.
use std::{io, ops::Range, time::Duration};

use async_trait::async_trait;
use futures::TryStreamExt;
use object_store::signer::Signer;
use object_store::{
    aws::{AmazonS3, AmazonS3Builder},
    path::Path,
    ClientOptions, GetOptions, GetResult, ObjectStore,
};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::{validate_key, validate_prefix, ObjectMetadata, ObjectPage, StorageBackend};

const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct S3Config {
    /// Service origin; empty uses the standard AWS regional endpoint.
    /// Virtual-hosted access also accepts an already bucket-qualified host.
    #[serde(default)]
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default = "default_path_style")]
    pub path_style: bool,
}

fn default_path_style() -> bool {
    true
}

/// Runtime credentials, supplied by the caller rather than ambient environment.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S3Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    #[serde(default)]
    pub session_token: String,
}

impl std::fmt::Debug for S3Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("S3Credentials([redacted])")
    }
}

impl S3Credentials {
    pub fn validate(&self) -> io::Result<()> {
        if self.access_key_id.trim().is_empty() || self.secret_access_key.trim().is_empty() {
            return Err(invalid(
                "S3 Access Key ID and Secret Access Key are required",
            ));
        }
        Ok(())
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

impl S3Config {
    pub fn validate(&self) -> io::Result<()> {
        // General purpose S3 buckets only, not access point ARNs or S3 Express.
        if !(3..=63).contains(&self.bucket.len())
            || !self
                .bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            || !self.bucket.starts_with(|c: char| c.is_ascii_alphanumeric())
            || !self.bucket.ends_with(|c: char| c.is_ascii_alphanumeric())
            || self.bucket.contains("..")
            || self.bucket.ends_with("--x-s3")
        {
            return Err(invalid("invalid S3 bucket name"));
        }
        if self.region.is_empty()
            || !self
                .region
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(invalid(
                "S3 region must be nonempty and contain only letters, digits or '-'",
            ));
        }
        validate_prefix(&self.prefix)?;
        if !self.endpoint.is_empty() {
            let url =
                url::Url::parse(&self.endpoint).map_err(|_| invalid("invalid S3 endpoint"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || !matches!(url.path(), "" | "/")
            {
                return Err(invalid("S3 endpoint must be an HTTP(S) origin without credentials, path, query or fragment"));
            }
            if !self.path_style && !matches!(url.host(), Some(url::Host::Domain(_))) {
                return Err(invalid(
                    "virtual-hosted S3 requires a DNS endpoint; use path-style for IP addresses",
                ));
            }
        }
        Ok(())
    }

    /// object_store treats a custom virtual-hosted endpoint as the *complete*
    /// bucket origin. Derive it at client construction, not normalization:
    /// persisted namespace identities and CAS configuration snapshots stay intact.
    pub fn client_endpoint(&self) -> io::Result<Option<String>> {
        self.validate()?;
        if self.endpoint.is_empty() {
            return Ok(None); // Let the SDK select its standard AWS endpoint.
        }
        let mut endpoint =
            url::Url::parse(&self.endpoint).map_err(|_| invalid("invalid S3 endpoint"))?;
        if !self.path_style {
            let host = endpoint
                .host_str()
                .ok_or_else(|| invalid("missing S3 endpoint host"))?;
            let prefix = format!("{}.", self.bucket);
            if !host.starts_with(&prefix) {
                let host = format!("{prefix}{host}");
                endpoint
                    .set_host(Some(&host))
                    .map_err(|_| invalid("invalid S3 bucket hostname"))?;
            }
        }
        Ok(Some(endpoint.as_str().trim_end_matches('/').to_owned()))
    }

    /// Canonical service origin for namespace-overlap checks. Accepting a
    /// bucket-qualified spelling must not bypass mount/transfer isolation.
    pub fn service_endpoint(&self) -> io::Result<String> {
        self.validate()?;
        if self.endpoint.is_empty() {
            return Ok(format!("https://s3.{}.amazonaws.com", self.region));
        }
        let mut endpoint =
            url::Url::parse(&self.endpoint).map_err(|_| invalid("invalid S3 endpoint"))?;
        if !self.path_style {
            if let Some(host) = endpoint
                .host_str()
                .and_then(|host| host.strip_prefix(&format!("{}.", self.bucket)))
            {
                let host = host.to_owned();
                endpoint
                    .set_host(Some(&host))
                    .map_err(|_| invalid("invalid S3 service hostname"))?;
            }
        }
        Ok(endpoint.as_str().trim_end_matches('/').to_owned())
    }

    pub fn normalized(mut self) -> io::Result<Self> {
        self.validate()?;
        if !self.endpoint.is_empty() {
            self.endpoint = url::Url::parse(&self.endpoint)
                .map_err(|_| invalid("invalid S3 endpoint"))?
                .as_str()
                .trim_end_matches('/')
                .to_owned();
        }
        Ok(self)
    }
}

pub struct S3Backend {
    store: AmazonS3,
    prefix: String,
    transfer_client: reqwest::Client,
    create_gate: tokio::sync::Mutex<()>,
}

impl S3Backend {
    pub fn new(config: &S3Config, credentials: &S3Credentials) -> io::Result<Self> {
        let endpoint = config.client_endpoint()?;
        credentials.validate()?;
        // Do not use from_env(): ambient AWS_ENDPOINT/BUCKET settings must never
        // redirect an archive's immutable namespace.
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&config.bucket)
            .with_region(&config.region)
            .with_virtual_hosted_style_request(!config.path_style)
            .with_access_key_id(&credentials.access_key_id)
            .with_secret_access_key(&credentials.secret_access_key)
            .with_client_options(
                ClientOptions::new()
                    .with_allow_http(config.endpoint.starts_with("http://"))
                    .with_connect_timeout(Duration::from_secs(10))
                    .with_timeout(Duration::from_secs(120)),
            );
        if let Some(endpoint) = endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        if !credentials.session_token.is_empty() {
            builder = builder.with_token(&credentials.session_token);
        }
        Ok(Self {
            store: builder.build().map_err(storage_error)?,
            prefix: config.prefix.clone(),
            create_gate: tokio::sync::Mutex::new(()),
            transfer_client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_gzip()
                .no_brotli()
                .no_deflate()
                .no_zstd()
                .connect_timeout(Duration::from_secs(10))
                .timeout(TRANSFER_TIMEOUT)
                .build()
                .map_err(|error| io::Error::other(error.without_url()))?,
        })
    }

    fn path(&self, key: &str) -> io::Result<Path> {
        validate_key(key)?;
        self.prefix_path(key)
    }

    async fn require_absent(&self, key: &str) -> io::Result<()> {
        match self.store.head(&self.path(key)?).await {
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "transfer target already exists",
            )),
            Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(storage_error(error)),
        }
    }

    fn prefix_path(&self, key: &str) -> io::Result<Path> {
        let path = match (self.prefix.is_empty(), key.is_empty()) {
            (true, _) => key.to_owned(),
            (false, true) => self.prefix.clone(),
            (false, false) => format!("{}/{key}", self.prefix),
        };
        Path::parse(path).map_err(|_| invalid("invalid S3 object path"))
    }
}

fn storage_error(error: object_store::Error) -> io::Error {
    let kind = match &error {
        object_store::Error::NotFound { .. } => io::ErrorKind::NotFound,
        object_store::Error::PermissionDenied { .. }
        | object_store::Error::Unauthenticated { .. } => io::ErrorKind::PermissionDenied,
        object_store::Error::Precondition { .. } => io::ErrorKind::WouldBlock,
        object_store::Error::AlreadyExists { .. } => io::ErrorKind::AlreadyExists,
        object_store::Error::InvalidPath { .. } => io::ErrorKind::InvalidInput,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}

async fn collect_bounded(result: GetResult, max_bytes: usize) -> io::Result<Vec<u8>> {
    if result.range.end - result.range.start > max_bytes as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recording object exceeds read limit",
        ));
    }
    let mut stream = result.into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.try_next().await.map_err(storage_error)? {
        if chunk.len() > max_bytes - bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "recording object exceeds read limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[async_trait]
impl StorageBackend for S3Backend {
    async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        self.read_bounded(key, usize::MAX).await
    }

    async fn read_bounded(&self, key: &str, max_bytes: usize) -> io::Result<Vec<u8>> {
        let result = self
            .store
            .get(&self.path(key)?)
            .await
            .map_err(storage_error)?;
        collect_bounded(result, max_bytes).await
    }

    async fn read_range(&self, key: &str, range: Range<u64>) -> io::Result<Vec<u8>> {
        if range.start > range.end {
            return Err(invalid("reversed S3 read range"));
        }
        let path = self.path(key)?;
        // HEAD provides EOF semantics (including empty files) without a 416
        // error, and If-Match prevents mixing metadata with a replacement.
        let meta = self.store.head(&path).await.map_err(storage_error)?;
        let end = range.end.min(meta.size);
        if range.start >= end {
            return Ok(Vec::new());
        }
        let max =
            usize::try_from(end - range.start).map_err(|_| invalid("S3 range is too large"))?;
        let result = self
            .store
            .get_opts(
                &path,
                GetOptions {
                    range: Some((range.start..end).into()),
                    if_match: meta.e_tag,
                    ..Default::default()
                },
            )
            .await
            .map_err(storage_error)?;
        if result.range != (range.start..end) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "S3 server returned an unexpected range",
            ));
        }
        collect_bounded(result, max).await
    }

    async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.store
            .put(&self.path(key)?, bytes.to_vec().into())
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    async fn create(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        let _gate = self.create_gate.lock().await;
        self.require_absent(key).await?;
        self.write(key, bytes).await
    }

    async fn create_from_file(&self, key: &str, path: &std::path::Path) -> io::Result<()> {
        let _gate = self.create_gate.lock().await;
        self.require_absent(key).await?;
        let file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
        if size > 5 * 1024 * 1024 * 1024 {
            return Err(invalid(
                "S3 transfer object exceeds the 5 GiB single-PUT limit",
            ));
        }
        // A single BSR writer owns this namespace. Preflight plus the application
        // transfer gate prevents clobbering; this is not cross-machine CAS.
        // Streaming avoids allocating a whole media object in memory.
        let url = self
            .store
            .signed_url(reqwest::Method::PUT, &self.path(key)?, TRANSFER_TIMEOUT)
            .await
            .map_err(storage_error)?;
        let response = self
            .transfer_client
            .put(url)
            .header(reqwest::header::CONTENT_LENGTH, size)
            .body(reqwest::Body::from(file))
            .send()
            .await
            .map_err(|error| io::Error::other(error.without_url()))?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let kind = match status.as_u16() {
                409 | 412 => io::ErrorKind::AlreadyExists,
                401 | 403 => io::ErrorKind::PermissionDenied,
                _ => io::ErrorKind::Other,
            };
            // Never put a signed URL (containing credentials) into an error.
            Err(io::Error::new(
                kind,
                format!("S3 transfer PUT failed: HTTP {status}"),
            ))
        }
    }

    async fn download_to_file(&self, key: &str, path: &std::path::Path) -> io::Result<()> {
        let url = self
            .store
            .signed_url(reqwest::Method::GET, &self.path(key)?, TRANSFER_TIMEOUT)
            .await
            .map_err(storage_error)?;
        let mut response = self
            .transfer_client
            .get(url)
            .send()
            .await
            .map_err(|error| io::Error::other(error.without_url()))?;
        if !response.status().is_success() {
            let kind = match response.status().as_u16() {
                404 => io::ErrorKind::NotFound,
                401 | 403 => io::ErrorKind::PermissionDenied,
                _ => io::ErrorKind::Other,
            };
            return Err(io::Error::new(
                kind,
                format!("S3 transfer GET failed: HTTP {}", response.status()),
            ));
        }
        let mut file = tokio::fs::File::create(path).await?;
        while let Some(bytes) = response
            .chunk()
            .await
            .map_err(|error| io::Error::other(error.without_url()))?
        {
            file.write_all(&bytes).await?;
        }
        file.flush().await
    }

    async fn transfer_revision(&self, key: &str) -> io::Result<String> {
        let meta = self
            .store
            .head(&self.path(key)?)
            .await
            .map_err(storage_error)?;
        let etag = meta
            .e_tag
            .ok_or_else(|| io::Error::other("S3 transfer requires object ETags"))?;
        serde_json::to_string(&(etag, meta.version, meta.size, meta.last_modified))
            .map_err(io::Error::other)
    }

    async fn remove(&self, key: &str) -> io::Result<()> {
        match self.store.delete(&self.path(key)?).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(storage_error(error)),
        }
    }

    async fn stat(&self, key: &str) -> io::Result<ObjectMetadata> {
        Ok(ObjectMetadata {
            size: self
                .store
                .head(&self.path(key)?)
                .await
                .map_err(storage_error)?
                .size,
        })
    }

    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> io::Result<ObjectPage> {
        validate_prefix(prefix)?;
        if limit == 0 {
            return Err(invalid("S3 list limit must be positive"));
        }
        if let Some(cursor) = cursor {
            validate_key(cursor)?;
            if !crate::in_subtree(cursor, prefix) {
                return Err(invalid("S3 list cursor is outside the subtree"));
            }
        }
        let mut keys = Vec::new();
        // object_store's list API already appends a delimiter to prefixes; the
        // exact object (also part of our subtree contract) is checked separately.
        if !prefix.is_empty() && cursor.is_none() {
            match self.stat(prefix).await {
                Ok(_) => keys.push(prefix.to_owned()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        let path = self.prefix_path(prefix)?;
        let mut stream = match cursor {
            Some(cursor) => self
                .store
                .list_with_offset(Some(&path), &self.path(cursor)?),
            None => self.store.list(Some(&path)),
        };
        let root = if self.prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", self.prefix)
        };
        while let Some(object) = stream.try_next().await.map_err(storage_error)? {
            if keys.len() == limit {
                return Ok(ObjectPage {
                    next_cursor: keys.last().cloned(),
                    keys,
                });
            }
            let key = object
                .location
                .as_ref()
                .strip_prefix(&root)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "S3 returned an object outside its namespace",
                    )
                })?;
            validate_key(key)?;
            if !crate::in_subtree(key, prefix)
                || keys.last().is_some_and(|last| last.as_str() >= key)
                || cursor.is_some_and(|cursor| key <= cursor)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "S3 returned unordered or out-of-subtree keys",
                ));
            }
            keys.push(key.to_owned());
        }
        Ok(ObjectPage {
            keys,
            next_cursor: None,
        })
    }
}

#[cfg(test)]
#[path = "s3/tests.rs"]
mod protocol_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_rejects_ambiguous_namespaces_and_embedded_secrets() {
        let base = S3Config {
            endpoint: "http://localhost:9000/".into(),
            bucket: "recordings".into(),
            region: "us-east-1".into(),
            prefix: "archive".into(),
            path_style: true,
        };
        assert_eq!(
            base.clone().normalized().unwrap().endpoint,
            "http://localhost:9000"
        );
        for endpoint in [
            "ftp://localhost",
            "https://user:password@host",
            "https://host/bucket",
            "https://host?token=secret",
            "https://host/#fragment",
        ] {
            assert!(S3Config {
                endpoint: endpoint.into(),
                ..base.clone()
            }
            .validate()
            .is_err());
        }
        for prefix in ["/archive", "archive/", "../archive", "archive//room"] {
            assert!(S3Config {
                prefix: prefix.into(),
                ..base.clone()
            }
            .validate()
            .is_err());
        }
        assert!(S3Config {
            prefix: "".into(),
            ..base.clone()
        }
        .validate()
        .is_ok());
        let credentials = S3Credentials {
            access_key_id: "test-access".into(),
            secret_access_key: "test-secret".into(),
            session_token: "test-token".into(),
        };
        assert_eq!(format!("{credentials:?}"), "S3Credentials([redacted])");
        assert!(credentials.validate().is_ok());
        assert!(S3Credentials {
            secret_access_key: "".into(),
            ..credentials
        }
        .validate()
        .is_err());
    }
}
