//! Commands for the cover editor's own stickers: images the user imports,
//! kept in the output directory so every client of this server shares them.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;

use crate::handlers::project::decode_data_url;
use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

/// Directory of the stickers, relative to the output directory.
const STICKER_DIR: &str = "cover_stickers";
const MAX_STICKER_BYTES: usize = 8 * 1024 * 1024;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoverSticker {
    /// File name.
    pub id: String,
    /// Path relative to the output directory.
    pub file: String,
}

impl CoverSticker {
    fn new(id: String) -> Self {
        Self {
            file: format!("{STICKER_DIR}/{id}"),
            id,
        }
    }
}

async fn sticker_dir(state: &State) -> PathBuf {
    PathBuf::from(&state.config.read().await.output).join(STICKER_DIR)
}

/// Only names this module gives out, so an id cannot leave the directory.
fn is_sticker_name(name: &str) -> bool {
    name.strip_suffix(".png")
        .and_then(|stem| stem.split_once('-'))
        .is_some_and(|(time, nonce)| {
            !time.is_empty()
                && time.bytes().all(|b| b.is_ascii_digit())
                && !nonce.is_empty()
                && nonce.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

/// Stickers in `dir`, in the order they were imported.
async fn list_stickers(dir: &Path) -> Result<Vec<CoverSticker>, String> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        if let Some(name) = entry.file_name().to_str().filter(|n| is_sticker_name(n)) {
            names.push(name.to_string());
        }
    }
    // The names start with the import time.
    names.sort_by_key(|name| {
        let time = name.split('-').next().and_then(|t| t.parse::<u64>().ok());
        (time.unwrap_or(u64::MAX), name.clone())
    });
    Ok(names.into_iter().map(CoverSticker::new).collect())
}

/// Save the PNG `bytes` in `dir` as a new sticker.
async fn save_sticker(dir: &Path, bytes: &[u8]) -> Result<CoverSticker, String> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err("贴纸必须是 PNG 图片".to_string());
    }
    if bytes.len() > MAX_STICKER_BYTES {
        return Err("贴纸图片不能超过 8 MB".to_string());
    }
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| e.to_string())?;
    let id = format!(
        "{}-{}.png",
        Utc::now().timestamp_millis(),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    tokio::fs::write(dir.join(&id), bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(CoverSticker::new(id))
}

async fn remove_sticker(dir: &Path, id: &str) -> Result<(), String> {
    if !is_sticker_name(id) {
        return Err(format!("无效的贴纸: {id}"));
    }
    match tokio::fs::remove_file(dir.join(id)).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// The imported stickers, oldest first.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_cover_stickers(state: state_type!()) -> Result<Vec<CoverSticker>, String> {
    list_stickers(&sticker_dir(&state).await).await
}

/// Import the base64 PNG data URL `image` as a sticker.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn add_cover_sticker(
    state: state_type!(),
    image: String,
) -> Result<CoverSticker, String> {
    let bytes = decode_data_url(&image)?;
    save_sticker(&sticker_dir(&state).await, &bytes).await
}

/// Remove an imported sticker; covers already made with it keep it.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_cover_sticker(state: state_type!(), id: String) -> Result<(), String> {
    remove_sticker(&sticker_dir(&state).await, &id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    #[test]
    fn only_generated_names_are_stickers() {
        assert!(is_sticker_name("1760000000000-0a1b2c3d.png"));
        assert!(!is_sticker_name("1760000000000-0a1b2c3d.jpg"));
        assert!(!is_sticker_name("../1760000000000-0a1b2c3d.png"));
        assert!(!is_sticker_name("1760000000000-0a1b/../x.png"));
        assert!(!is_sticker_name("-0a1b2c3d.png"));
        assert!(!is_sticker_name("1760000000000-.png"));
        assert!(!is_sticker_name("cover.png"));
    }

    #[tokio::test]
    async fn stickers_are_listed_in_import_order() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_stickers(&dir.path().join("missing"))
            .await
            .unwrap()
            .is_empty());

        for name in ["99-aa.png", "100-bb.png", "100-aa.png", "notes.txt"] {
            tokio::fs::write(dir.path().join(name), PNG).await.unwrap();
        }
        let saved = save_sticker(dir.path(), PNG).await.unwrap();
        let listed = list_stickers(dir.path()).await.unwrap();
        let ids: Vec<_> = listed.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            ["99-aa.png", "100-aa.png", "100-bb.png", saved.id.as_str()]
        );
        assert_eq!(saved.file, format!("cover_stickers/{}", saved.id));
        assert_eq!(
            tokio::fs::read(dir.path().join(&saved.id)).await.unwrap(),
            PNG
        );
    }

    #[tokio::test]
    async fn only_png_images_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_sticker(dir.path(), b"<svg/>").await.is_err());
        let mut large = PNG.to_vec();
        large.resize(MAX_STICKER_BYTES + 1, 0);
        assert!(save_sticker(dir.path(), &large).await.is_err());
        assert!(list_stickers(dir.path()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn removing_stays_inside_the_directory() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(STICKER_DIR);
        let saved = save_sticker(&dir, PNG).await.unwrap();
        let outside = root.path().join("1-aa.png");
        tokio::fs::write(&outside, PNG).await.unwrap();

        assert!(remove_sticker(&dir, "../1-aa.png").await.is_err());
        assert!(outside.exists());
        remove_sticker(&dir, &saved.id).await.unwrap();
        assert!(list_stickers(&dir).await.unwrap().is_empty());
        // Already gone is fine.
        remove_sticker(&dir, &saved.id).await.unwrap();
    }
}
