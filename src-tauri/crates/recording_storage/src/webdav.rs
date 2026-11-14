//! WebDAV objects published with PUT to a private sibling followed by MOVE.
//! The configured collection must exist; nested collections are created on demand.
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    ops::Range,
    path::Path,
    time::Duration,
};

use async_trait::async_trait;
use reqwest::{header, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::{validate_key, validate_prefix, ObjectMetadata, ObjectPage, StorageBackend};

const UPLOAD_PREFIX: &str = ".webdav-upload-";
const MAX_PROPERTIES: usize = 8 * 1024 * 1024;
const PROPERTIES: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getetag/><d:getlastmodified/></d:prop></d:propfind>"#;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn malformed(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn network(error: reqwest::Error) -> io::Error {
    io::Error::other(error.without_url())
}
fn status_error(status: StatusCode) -> io::Error {
    let kind = match status.as_u16() {
        404 => io::ErrorKind::NotFound,
        401 | 403 => io::ErrorKind::PermissionDenied,
        412 => io::ErrorKind::AlreadyExists,
        423 => io::ErrorKind::WouldBlock,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(
        kind,
        format!("WebDAV request failed: HTTP {}", status.as_u16()),
    )
}
fn expect(response: Response, statuses: &[StatusCode]) -> io::Result<Response> {
    if statuses.contains(&response.status()) {
        Ok(response)
    } else {
        Err(status_error(response.status()))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebDavConfig {
    /// URL of an existing collection, including the service's DAV path.
    pub endpoint: String,
    #[serde(default)]
    pub prefix: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebDavCredentials {
    pub username: String,
    pub password: String,
}
impl std::fmt::Debug for WebDavCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WebDavCredentials([redacted])")
    }
}
impl WebDavCredentials {
    pub fn validate(&self) -> io::Result<()> {
        if self.username.trim().is_empty()
            || self.username.contains(':')
            || self.username.chars().any(char::is_control)
            || self.password.is_empty()
        {
            return Err(invalid(
                "WebDAV requires a username (without ':' or control characters) and password",
            ));
        }
        Ok(())
    }
}

fn decoded_segment(value: &str) -> io::Result<String> {
    let value = percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| invalid("WebDAV URL path is not UTF-8"))?
        .into_owned();
    if value == "."
        || value == ".."
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err(invalid("ambiguous WebDAV URL path"));
    }
    Ok(value)
}

impl WebDavConfig {
    pub fn normalized(&self) -> io::Result<Self> {
        validate_prefix(&self.prefix)?;
        if self
            .prefix
            .split('/')
            .any(|part| part.starts_with(UPLOAD_PREFIX))
        {
            return Err(invalid("reserved WebDAV upload path"));
        }
        let mut url = Url::parse(&self.endpoint).map_err(|_| invalid("invalid WebDAV URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid(
                "WebDAV URL must be HTTP(S), without credentials, query or fragment",
            ));
        }
        let parts = url
            .path_segments()
            .ok_or_else(|| invalid("invalid WebDAV path"))?
            .filter(|part| !part.is_empty())
            .map(decoded_segment)
            .collect::<io::Result<Vec<_>>>()?;
        url.path_segments_mut()
            .map_err(|_| invalid("invalid WebDAV path"))?
            .clear()
            .extend(parts.iter().map(String::as_str))
            .push("");
        Ok(Self {
            endpoint: url.to_string(),
            prefix: self.prefix.clone(),
        })
    }

    /// Canonical collection URL, shared by media, catalog and overlap checks.
    pub fn root_url(&self) -> io::Result<Url> {
        let config = self.normalized()?;
        let mut url = Url::parse(&config.endpoint).map_err(|_| invalid("invalid WebDAV URL"))?;
        if !config.prefix.is_empty() {
            url.path_segments_mut()
                .map_err(|_| invalid("invalid WebDAV path"))?
                .pop_if_empty()
                .extend(config.prefix.split('/'))
                .push("");
        }
        Ok(url)
    }

    pub fn overlaps(&self, other: &Self) -> io::Result<bool> {
        let a = self.root_url()?;
        let b = other.root_url()?;
        Ok(a.origin() == b.origin()
            && (a.path().starts_with(b.path()) || b.path().starts_with(a.path())))
    }
}

pub struct WebDavBackend {
    client: reqwest::Client,
    credentials: WebDavCredentials,
    endpoint: Url,
    root: Url,
    directories: tokio::sync::Mutex<BTreeSet<String>>,
}

impl WebDavBackend {
    pub fn new(config: &WebDavConfig, credentials: &WebDavCredentials) -> io::Result<Self> {
        credentials.validate()?;
        let config = config.normalized()?;
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_gzip()
                .no_brotli()
                .no_deflate()
                .no_zstd()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30 * 60))
                .build()
                .map_err(network)?,
            credentials: credentials.clone(),
            endpoint: Url::parse(&config.endpoint).map_err(|_| invalid("invalid WebDAV URL"))?,
            root: config.root_url()?,
            directories: Default::default(),
        })
    }

    fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.credentials.username, Some(&self.credentials.password))
            .header(header::ACCEPT_ENCODING, "identity")
    }

    fn url(&self, key: &str) -> io::Result<Url> {
        validate_prefix(key)?;
        if key.split('/').any(|part| part.starts_with(UPLOAD_PREFIX)) {
            return Err(invalid("reserved WebDAV upload path"));
        }
        let mut url = self.root.clone();
        if !key.is_empty() {
            url.path_segments_mut()
                .map_err(|_| invalid("invalid WebDAV URL"))?
                .pop_if_empty()
                .extend(key.split('/'));
        }
        Ok(url)
    }

    async fn properties(&self, url: &Url, depth: &str) -> io::Result<Vec<DavEntry>> {
        let mut current = url.clone();
        for attempt in 0..2 {
            let response = self
                .request(Method::from_bytes(b"PROPFIND").unwrap(), current.clone())
                .header("Depth", depth)
                .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
                .timeout(Duration::from_secs(120))
                .body(PROPERTIES)
                .send()
                .await
                .map_err(network)?;
            if attempt == 0 && matches!(response.status().as_u16(), 301 | 302 | 307 | 308) {
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| current.join(v).ok())
                    .ok_or_else(|| malformed("invalid WebDAV collection redirect"))?;
                let mut canonical = current.clone();
                canonical
                    .path_segments_mut()
                    .map_err(|_| invalid("invalid WebDAV path"))?
                    .pop_if_empty()
                    .push("");
                // Only allow adding the collection slash. Never forward credentials
                // to a different origin, path or user supplied redirect target.
                if location != canonical || current == canonical {
                    return Err(malformed("unexpected WebDAV redirect"));
                }
                current = canonical;
                continue;
            }
            let response = expect(response, &[StatusCode::MULTI_STATUS])?;
            let bytes = collect(response, MAX_PROPERTIES).await?;
            let entries = parse_properties(&bytes, &current)?;
            for entry in &entries {
                if same_path(&entry.url, &current) {
                    continue;
                }
                let base = format!("{}/", current.path().trim_end_matches('/'));
                let child = entry.url.path().strip_prefix(&base);
                if depth == "0"
                    || child.is_none_or(|part| {
                        part.trim_end_matches('/').is_empty()
                            || part.trim_end_matches('/').contains('/')
                    })
                {
                    return Err(malformed(
                        "WebDAV response escaped requested collection depth",
                    ));
                }
            }
            return Ok(entries);
        }
        Err(malformed("too many WebDAV redirects"))
    }

    async fn ensure_parent(&self, target: &Url) -> io::Result<()> {
        let parent = target
            .join(".")
            .map_err(|_| invalid("invalid WebDAV parent"))?;
        let mut known = self.directories.lock().await;
        if known.contains(parent.as_str()) {
            return Ok(());
        }
        if !known.contains(self.endpoint.as_str()) {
            let entries = self.properties(&self.endpoint, "0").await?;
            if !entries
                .iter()
                .any(|e| same_path(&e.url, &self.endpoint) && e.collection)
            {
                return Err(invalid("WebDAV endpoint must be an existing collection"));
            }
            known.insert(self.endpoint.to_string());
        }
        let relative = parent
            .path()
            .strip_prefix(self.endpoint.path())
            .ok_or_else(|| invalid("WebDAV parent escaped endpoint"))?;
        let mut current = self.endpoint.clone();
        for part in relative
            .trim_end_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
        {
            let part = decoded_segment(part)?;
            current
                .path_segments_mut()
                .map_err(|_| invalid("invalid WebDAV path"))?
                .pop_if_empty()
                .push(&part)
                .push("");
            if known.contains(current.as_str()) {
                continue;
            }
            let response = self
                .request(Method::from_bytes(b"MKCOL").unwrap(), current.clone())
                .timeout(Duration::from_secs(120))
                .send()
                .await
                .map_err(network)?;
            if response.status() == StatusCode::METHOD_NOT_ALLOWED {
                // 405 can also mean that MKCOL is unsupported or a file exists.
                let entries = self.properties(&current, "0").await?;
                if !entries
                    .iter()
                    .any(|e| same_path(&e.url, &current) && e.collection)
                {
                    return Err(invalid("WebDAV parent is not a collection"));
                }
            } else {
                expect(response, &[StatusCode::CREATED])?;
            }
            known.insert(current.to_string());
        }
        Ok(())
    }

    async fn publish(
        &self,
        key: &str,
        body: reqwest::Body,
        size: u64,
        create: bool,
    ) -> io::Result<()> {
        validate_key(key)?;
        let destination = self.url(key)?;
        self.ensure_parent(&destination).await?;
        let temporary = destination
            .join(&format!("{UPLOAD_PREFIX}{}", uuid::Uuid::new_v4()))
            .map_err(|_| invalid("invalid upload URL"))?;
        let result = async {
            let response = self
                .request(Method::PUT, temporary.clone())
                .header(header::CONTENT_LENGTH, size)
                .header(header::IF_NONE_MATCH, "*")
                .body(body)
                .send()
                .await
                .map_err(network)?;
            expect(
                response,
                &[StatusCode::CREATED, StatusCode::NO_CONTENT, StatusCode::OK],
            )?;
            let response = self
                .request(Method::from_bytes(b"MOVE").unwrap(), temporary.clone())
                .header("Destination", destination.as_str())
                .header("Overwrite", if create { "F" } else { "T" })
                .send()
                .await
                .map_err(network)?;
            expect(response, &[StatusCode::CREATED, StatusCode::NO_CONTENT])?;
            Ok(())
        }
        .await;
        if result.is_err() {
            // Best effort: never remove the destination after an ambiguous MOVE.
            let _ = self
                .request(Method::DELETE, temporary)
                .timeout(Duration::from_secs(10))
                .send()
                .await;
        }
        result
    }

    async fn head(&self, key: &str) -> io::Result<Response> {
        validate_key(key)?;
        let response = self
            .request(Method::HEAD, self.url(key)?)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(network)?;
        expect(response, &[StatusCode::OK])
    }
}

fn same_path(a: &Url, b: &Url) -> bool {
    a.origin() == b.origin() && a.path().trim_end_matches('/') == b.path().trim_end_matches('/')
}
fn content_length(response: &Response) -> io::Result<u64> {
    response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| malformed("WebDAV response lacks Content-Length"))
}
async fn collect(mut response: Response, max: usize) -> io::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|size| size > max as u64)
    {
        return Err(malformed("WebDAV object exceeds read limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network)? {
        if chunk.len() > max - bytes.len() {
            return Err(malformed("WebDAV object exceeds read limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

struct DavEntry {
    url: Url,
    collection: bool,
}
fn parse_properties(bytes: &[u8], requested: &Url) -> io::Result<Vec<DavEntry>> {
    let text = std::str::from_utf8(bytes).map_err(|_| malformed("invalid WebDAV XML encoding"))?;
    let doc = roxmltree::Document::parse(text).map_err(|_| malformed("invalid WebDAV XML"))?;
    if !doc.root_element().has_tag_name(("DAV:", "multistatus")) {
        return Err(malformed("expected WebDAV multistatus"));
    }
    let mut entries = Vec::new();
    for response in doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name(("DAV:", "response")))
    {
        let child_text = |name| {
            response
                .children()
                .find(|n| n.has_tag_name(("DAV:", name)))
                .and_then(|n| n.text())
        };
        let href = child_text("href").ok_or_else(|| malformed("WebDAV response lacks href"))?;
        let mut url = requested
            .join(href)
            .map_err(|_| malformed("invalid WebDAV href"))?;
        if url.origin() != requested.origin()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(malformed("WebDAV href escaped origin"));
        }
        let trailing_slash = url.path().ends_with('/');
        let parts = url
            .path_segments()
            .ok_or_else(|| malformed("invalid WebDAV href path"))?
            .filter(|s| !s.is_empty())
            .map(decoded_segment)
            .collect::<io::Result<Vec<_>>>()?;
        url.path_segments_mut()
            .map_err(|_| malformed("invalid WebDAV href path"))?
            .clear()
            .extend(parts.iter().map(String::as_str));
        if trailing_slash {
            url.path_segments_mut().unwrap().push("");
        }
        if let Some(status) = child_text("status") {
            if status.split_whitespace().nth(1) != Some("200") {
                return Err(malformed("WebDAV resource returned a failed status"));
            }
        }
        let mut resource_type = None;
        for propstat in response
            .children()
            .filter(|n| n.has_tag_name(("DAV:", "propstat")))
        {
            let status = propstat
                .children()
                .find(|n| n.has_tag_name(("DAV:", "status")))
                .and_then(|n| n.text());
            if status.and_then(|s| s.split_whitespace().nth(1)) != Some("200") {
                continue;
            }
            resource_type = resource_type.or_else(|| {
                propstat
                    .children()
                    .find(|n| n.has_tag_name(("DAV:", "prop")))
                    .and_then(|p| {
                        p.children()
                            .find(|n| n.has_tag_name(("DAV:", "resourcetype")))
                    })
            });
        }
        let resource_type = resource_type
            .ok_or_else(|| malformed("WebDAV response lacks successful resourcetype"))?;
        entries.push(DavEntry {
            url,
            collection: resource_type
                .children()
                .any(|n| n.has_tag_name(("DAV:", "collection"))),
        });
    }
    if entries.is_empty() {
        return Err(malformed("empty WebDAV multistatus"));
    }
    Ok(entries)
}

#[async_trait]
impl StorageBackend for WebDavBackend {
    async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        self.read_bounded(key, usize::MAX).await
    }

    async fn read_bounded(&self, key: &str, max_bytes: usize) -> io::Result<Vec<u8>> {
        validate_key(key)?;
        let response = self
            .request(Method::GET, self.url(key)?)
            .send()
            .await
            .map_err(network)?;
        collect(expect(response, &[StatusCode::OK])?, max_bytes).await
    }

    async fn read_range(&self, key: &str, range: Range<u64>) -> io::Result<Vec<u8>> {
        if range.start > range.end {
            return Err(invalid("reversed WebDAV range"));
        }
        let meta = self.head(key).await?;
        let size = content_length(&meta)?;
        let end = range.end.min(size);
        if range.start >= end {
            return Ok(Vec::new());
        }
        let max =
            usize::try_from(end - range.start).map_err(|_| invalid("WebDAV range too large"))?;
        let mut request = self
            .request(Method::GET, self.url(key)?)
            .header(header::RANGE, format!("bytes={}-{}", range.start, end - 1));
        if let Some(etag) = meta.headers().get(header::ETAG) {
            if !etag.as_bytes().starts_with(b"W/") {
                request = request.header(header::IF_MATCH, etag);
            } else if let Some(modified) = meta.headers().get(header::LAST_MODIFIED) {
                request = request.header(header::IF_UNMODIFIED_SINCE, modified);
            }
        } else if let Some(modified) = meta.headers().get(header::LAST_MODIFIED) {
            request = request.header(header::IF_UNMODIFIED_SINCE, modified);
        }
        let response = expect(
            request.send().await.map_err(network)?,
            &[StatusCode::PARTIAL_CONTENT],
        )?;
        let expected = format!("bytes {}-{}/{size}", range.start, end - 1);
        if response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            != Some(expected.as_str())
        {
            return Err(malformed("WebDAV server returned an unexpected range"));
        }
        let bytes = collect(response, max).await?;
        if bytes.len() != max {
            return Err(malformed("truncated WebDAV range"));
        }
        Ok(bytes)
    }

    async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.publish(key, bytes.to_vec().into(), bytes.len() as u64, false)
            .await
    }
    async fn create(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.publish(key, bytes.to_vec().into(), bytes.len() as u64, true)
            .await
    }
    async fn create_from_file(&self, key: &str, path: &Path) -> io::Result<()> {
        let file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
        self.publish(key, file.into(), size, true).await
    }
    async fn download_to_file(&self, key: &str, path: &Path) -> io::Result<()> {
        validate_key(key)?;
        let mut response = expect(
            self.request(Method::GET, self.url(key)?)
                .send()
                .await
                .map_err(network)?,
            &[StatusCode::OK],
        )?;
        let mut file = tokio::fs::File::create(path).await?;
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            file.write_all(&chunk).await?;
        }
        file.sync_all().await
    }
    async fn transfer_revision(&self, key: &str) -> io::Result<String> {
        let response = self.head(key).await?;
        let size = content_length(&response)?;
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty());
        let modified = response
            .headers()
            .get(header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok());
        if etag.is_none() && modified.is_none() {
            return Err(malformed("WebDAV transfers require ETag or Last-Modified"));
        }
        serde_json::to_string(&(size, etag, modified)).map_err(io::Error::other)
    }
    async fn remove(&self, key: &str) -> io::Result<()> {
        validate_key(key)?;
        let url = self.url(key)?;
        // Never allow DELETE's recursive collection semantics through an object API.
        let entries = match self.properties(&url, "0").await {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            result => result?,
        };
        if !entries
            .iter()
            .any(|e| same_path(&e.url, &url) && !e.collection)
        {
            return Err(invalid("WebDAV remove requires a file"));
        }
        let response = self
            .request(Method::DELETE, url)
            .send()
            .await
            .map_err(network)?;
        expect(
            response,
            &[
                StatusCode::OK,
                StatusCode::NO_CONTENT,
                StatusCode::NOT_FOUND,
            ],
        )?;
        Ok(())
    }
    async fn stat(&self, key: &str) -> io::Result<ObjectMetadata> {
        Ok(ObjectMetadata {
            size: content_length(&self.head(key).await?)?,
        })
    }
    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> io::Result<ObjectPage> {
        validate_prefix(prefix)?;
        if limit == 0 || limit > 10_000 {
            return Err(invalid("invalid WebDAV page size"));
        }
        if let Some(cursor) = cursor {
            validate_key(cursor)?;
            if !crate::in_subtree(cursor, prefix) {
                return Err(invalid("WebDAV cursor outside prefix"));
            }
        }
        let mut pending = BTreeMap::from([(prefix.to_owned(), self.url(prefix)?)]);
        let mut visited = BTreeSet::new();
        let mut keys = BTreeSet::new();
        while let Some((current_key, mut current)) = pending.pop_first() {
            let lower_bound = format!("{current_key}/");
            if current_key != prefix
                && (cursor.is_some_and(|cursor| {
                    lower_bound.as_str() < cursor && !cursor.starts_with(&lower_bound)
                }) || (keys.len() > limit
                    && keys.last().is_some_and(|last: &String| lower_bound > *last)))
            {
                continue;
            }
            if !visited.insert(current.path().trim_end_matches('/').to_owned()) {
                continue;
            }
            // Collections have a trailing slash on most DAV servers. Depth 0
            // distinguishes a prefix object without accidentally listing siblings.
            let entries = match self.properties(&current, "0").await {
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                result => result?,
            };
            let entry = entries
                .iter()
                .find(|e| same_path(&e.url, &current))
                .ok_or_else(|| malformed("WebDAV response omitted requested resource"))?;
            let entries = if entry.collection {
                current
                    .path_segments_mut()
                    .map_err(|_| invalid("invalid WebDAV path"))?
                    .pop_if_empty()
                    .push("");
                self.properties(&current, "1").await?
            } else {
                entries
            };
            for entry in entries {
                let relative = entry
                    .url
                    .path()
                    .strip_prefix(self.root.path())
                    .or_else(|| {
                        if same_path(&entry.url, &self.root) {
                            Some("")
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| malformed("WebDAV href escaped storage root"))?;
                let key = relative
                    .trim_end_matches('/')
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .map(decoded_segment)
                    .collect::<io::Result<Vec<_>>>()?
                    .join("/");
                validate_prefix(&key)?;
                if !crate::in_subtree(&key, prefix) {
                    return Err(malformed("WebDAV href escaped requested subtree"));
                }
                if key.split('/').any(|part| part.starts_with(UPLOAD_PREFIX)) {
                    continue;
                }
                if entry.collection {
                    if !same_path(&entry.url, &current) {
                        pending.insert(key, entry.url);
                    }
                } else if cursor.is_none_or(|cursor| key.as_str() > cursor) {
                    keys.insert(key);
                    if keys.len() > limit + 1 {
                        keys.pop_last();
                    }
                }
            }
        }
        let more = keys.len() > limit;
        if more {
            keys.pop_last();
        }
        let keys: Vec<_> = keys.into_iter().collect();
        let next_cursor = more.then(|| keys.last().unwrap().clone());
        Ok(ObjectPage { keys, next_cursor })
    }
    async fn prepare(&self, prefix: &str) -> io::Result<()> {
        validate_prefix(prefix)?;
        let key = if prefix.is_empty() {
            ".prepare".to_owned()
        } else {
            format!("{prefix}/.prepare")
        };
        self.ensure_parent(&self.url(&key)?).await
    }
}

#[cfg(test)]
mod tests;
