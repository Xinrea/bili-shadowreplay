//! Moves what older versions kept under the legacy identifier into the
//! directories of the app's identifier in `tauri.conf.json`.
//!
//! Older versions kept the config, the default cache and the default output
//! under `cn.vjoi.bili-shadowreplay`, while the database, the logs and the web
//! view used the app's identifier, `cn.vjoi.bilishadowreplay`.

use std::path::{Path, PathBuf};

use platform_dirs::AppDirs;

use crate::config::Config;
use crate::database::Database;

/// The identifier older versions kept the config, cache and output under.
pub const LEGACY_IDENTIFIER: &str = "cn.vjoi.bili-shadowreplay";

/// Each legacy directory with the current one its entries move into, once
/// each: config and data are one directory on macOS, data and cache on Windows.
fn moves(legacy: &AppDirs, current: &AppDirs) -> Vec<(PathBuf, PathBuf)> {
    let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in [
        (&legacy.config_dir, &current.config_dir),
        (&legacy.data_dir, &current.data_dir),
        (&legacy.cache_dir, &current.cache_dir),
    ] {
        if from != to && !moves.iter().any(|(moved, _)| moved == from) {
            moves.push((from.clone(), to.clone()));
        }
    }
    moves
}

/// Move every entry of the legacy directories into the current ones, leaving
/// those whose name is already taken there, and remove the legacy directories
/// left empty.
///
/// Entries are only renamed, which moves them whole or not at all: one that
/// cannot be, as a folder with a file open in it on Windows, stays as it was
/// for the next start, and the config keeps pointing to it.
pub fn migrate(legacy: &AppDirs, current: &AppDirs) {
    for (from, to) in moves(legacy, current) {
        let Ok(entries) = std::fs::read_dir(&from) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let target = to.join(entry.file_name());
            if target.exists() {
                log::warn!(
                    "Not moving {}: {} already exists",
                    path.display(),
                    target.display()
                );
                continue;
            }
            match std::fs::create_dir_all(&to).and_then(|()| std::fs::rename(&path, &target)) {
                Ok(()) => log::info!("Moved {} to {}", path.display(), target.display()),
                Err(e) => log::error!(
                    "Failed to move {} to {}: {e}",
                    path.display(),
                    target.display()
                ),
            }
        }
        // Fails while anything is left, which stays for the next start.
        if std::fs::remove_dir(&from).is_ok() {
            log::info!("Removed legacy directory {}", from.display());
        }
    }
}

/// The config file `name`, still in the legacy config directory while it could
/// not be moved: a default one written in the current directory instead would
/// take its name and keep it from ever moving.
pub fn config_path(legacy: &AppDirs, current: &AppDirs, name: &str) -> PathBuf {
    let path = current.config_dir.join(name);
    let legacy_path = legacy.config_dir.join(name);
    if !path.exists() && legacy_path.exists() {
        legacy_path
    } else {
        path
    }
}

/// Where `path` is now if it was inside a legacy directory and has moved: gone
/// from there and found at the same place in the current directory.
fn relocated(path: &str, legacy: &AppDirs, current: &AppDirs) -> Option<String> {
    let path = Path::new(path);
    moves(legacy, current).into_iter().find_map(|(from, to)| {
        let moved = to.join(path.strip_prefix(&from).ok()?);
        (!path.exists() && moved.exists()).then(|| moved.to_string_lossy().into_owned())
    })
}

/// Point the paths of `config` that were inside the legacy directories to
/// where they have moved, saving it if any has.
pub fn relocate_config(config: &mut Config, legacy: &AppDirs, current: &AppDirs) {
    let mut changed = false;
    for path in [
        &mut config.cache,
        &mut config.output,
        &mut config.whisper_model,
    ] {
        if let Some(moved) = relocated(path, legacy, current) {
            log::info!("Config path {path} moved to {moved}");
            *path = moved;
            changed = true;
        }
    }
    if changed {
        config.save();
    }
}

/// Point the uploads of files that were inside the legacy directories to
/// where those have moved: unlike other paths, they are kept absolute.
pub async fn relocate_uploads(db: &Database, legacy: &AppDirs, current: &AppDirs) {
    let uploads = match db.get_uploads().await {
        Ok(uploads) => uploads,
        Err(e) => {
            log::error!("Failed to read uploads to relocate: {e}");
            return;
        }
    };
    for upload in uploads.iter().filter(|upload| upload.source == "file") {
        let Some(moved) = relocated(&upload.file, legacy, current) else {
            continue;
        };
        match db.set_upload_file(upload.id, &moved).await {
            Ok(_) => log::info!("Upload {} file {} moved to {moved}", upload.id, upload.file),
            Err(e) => log::error!("Failed to relocate upload {}: {e}", upload.id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path, identifier: &str) -> AppDirs {
        let data = root.join("Application Support").join(identifier);
        AppDirs {
            cache_dir: root.join("Caches").join(identifier),
            config_dir: data.clone(),
            data_dir: data.clone(),
            state_dir: data,
        }
    }

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn legacy_files_move_beside_the_database_and_the_legacy_dirs_go() {
        let root = tempfile::tempdir().unwrap();
        let legacy = dirs(root.path(), LEGACY_IDENTIFIER);
        let current = dirs(root.path(), "cn.vjoi.bilishadowreplay");
        write(&legacy.config_dir.join("Conf.toml"), "conf");
        write(&legacy.data_dir.join("output/clip.mp4"), "clip");
        write(&legacy.cache_dir.join("cache/bilibili/1/2/0.ts"), "ts");
        write(&current.config_dir.join("data_v2.db"), "db");
        write(&current.cache_dir.join("WebKit/state"), "webkit");

        migrate(&legacy, &current);

        assert_eq!(read(&current.config_dir.join("Conf.toml")), "conf");
        assert_eq!(read(&current.data_dir.join("output/clip.mp4")), "clip");
        assert_eq!(
            read(&current.cache_dir.join("cache/bilibili/1/2/0.ts")),
            "ts"
        );
        assert_eq!(read(&current.config_dir.join("data_v2.db")), "db");
        assert_eq!(read(&current.cache_dir.join("WebKit/state")), "webkit");
        assert!(!legacy.config_dir.exists());
        assert!(!legacy.cache_dir.exists());
    }

    #[test]
    fn taken_names_are_left_in_the_legacy_dir() {
        let root = tempfile::tempdir().unwrap();
        let legacy = dirs(root.path(), LEGACY_IDENTIFIER);
        let current = dirs(root.path(), "cn.vjoi.bilishadowreplay");
        write(&legacy.config_dir.join("Conf.toml"), "old");
        write(&legacy.config_dir.join("models/tiny.bin"), "model");
        write(&current.config_dir.join("Conf.toml"), "new");

        migrate(&legacy, &current);

        assert_eq!(read(&current.config_dir.join("Conf.toml")), "new");
        assert_eq!(read(&legacy.config_dir.join("Conf.toml")), "old");
        assert_eq!(read(&current.config_dir.join("models/tiny.bin")), "model");
    }

    #[test]
    fn only_paths_that_moved_are_relocated() {
        let root = tempfile::tempdir().unwrap();
        let legacy = dirs(root.path(), LEGACY_IDENTIFIER);
        let current = dirs(root.path(), "cn.vjoi.bilishadowreplay");
        write(&legacy.cache_dir.join("cache/bilibili/1/2/0.ts"), "ts");
        write(&legacy.config_dir.join("output/clip.mp4"), "clip");
        write(&current.config_dir.join("output/other.mp4"), "other");
        let elsewhere = root.path().join("Movies/output");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let path = |path: PathBuf| path.to_string_lossy().into_owned();

        migrate(&legacy, &current);

        assert_eq!(
            relocated(&path(legacy.cache_dir.join("cache")), &legacy, &current),
            Some(path(current.cache_dir.join("cache")))
        );
        // Its name was taken, so it is still where it was.
        assert_eq!(
            relocated(&path(legacy.data_dir.join("output")), &legacy, &current),
            None
        );
        assert_eq!(relocated(&path(elsewhere), &legacy, &current), None);
        assert_eq!(relocated("whisper_model.bin", &legacy, &current), None);
    }

    #[test]
    fn each_directory_moves_once_on_every_layout() {
        let root = Path::new("/home");
        let layout = |config: &str, data: &str, cache: &str| AppDirs {
            config_dir: root.join(config),
            data_dir: root.join(data),
            cache_dir: root.join(cache),
            state_dir: root.join("state"),
        };
        let pair = |from: &str, to: &str| (root.join(from), root.join(to));

        // Windows: data and cache are both under AppData\Local.
        let windows = moves(
            &layout("Roaming/old", "Local/old", "Local/old"),
            &layout("Roaming/new", "Local/new", "Local/new"),
        );
        assert_eq!(
            windows,
            vec![
                pair("Roaming/old", "Roaming/new"),
                pair("Local/old", "Local/new")
            ]
        );
        // Linux: three directories.
        let linux = moves(
            &layout(".config/old", ".local/share/old", ".cache/old"),
            &layout(".config/new", ".local/share/new", ".cache/new"),
        );
        assert_eq!(
            linux,
            vec![
                pair(".config/old", ".config/new"),
                pair(".local/share/old", ".local/share/new"),
                pair(".cache/old", ".cache/new"),
            ]
        );
    }

    #[test]
    fn an_entry_that_cannot_be_renamed_stays_whole_and_in_use() {
        let root = tempfile::tempdir().unwrap();
        let legacy = dirs(root.path(), LEGACY_IDENTIFIER);
        let current = dirs(root.path(), "cn.vjoi.bilishadowreplay");
        write(&legacy.cache_dir.join("cache/bilibili/1/2/0.ts"), "ts");
        // A file where the current cache directory goes keeps the rename from
        // happening, as a file open in the folder does on Windows.
        write(&current.cache_dir, "in the way");

        migrate(&legacy, &current);

        assert_eq!(
            read(&legacy.cache_dir.join("cache/bilibili/1/2/0.ts")),
            "ts"
        );
        let cache = legacy
            .cache_dir
            .join("cache")
            .to_string_lossy()
            .into_owned();
        assert_eq!(relocated(&cache, &legacy, &current), None);
    }

    #[test]
    fn the_legacy_config_is_used_until_it_has_moved() {
        let root = tempfile::tempdir().unwrap();
        let legacy = dirs(root.path(), LEGACY_IDENTIFIER);
        let current = dirs(root.path(), "cn.vjoi.bilishadowreplay");
        let path = || config_path(&legacy, &current, "Conf.toml");

        assert_eq!(path(), current.config_dir.join("Conf.toml"));
        write(&legacy.config_dir.join("Conf.toml"), "old");
        assert_eq!(path(), legacy.config_dir.join("Conf.toml"));
        write(&current.config_dir.join("Conf.toml"), "new");
        assert_eq!(path(), current.config_dir.join("Conf.toml"));
    }

    #[test]
    fn a_move_that_failed_is_done_on_a_later_start() {
        let root = tempfile::tempdir().unwrap();
        let legacy = dirs(root.path(), LEGACY_IDENTIFIER);
        let current = dirs(root.path(), "cn.vjoi.bilishadowreplay");
        write(&legacy.cache_dir.join("cache/bilibili/1/2/0.ts"), "ts");
        write(&current.cache_dir, "in the way");
        migrate(&legacy, &current);
        assert!(legacy.cache_dir.exists());

        std::fs::remove_file(&current.cache_dir).unwrap();
        migrate(&legacy, &current);

        assert_eq!(
            read(&current.cache_dir.join("cache/bilibili/1/2/0.ts")),
            "ts"
        );
        assert!(!legacy.cache_dir.exists());
    }

    #[test]
    fn windows_folders_move_from_roaming_and_local() {
        let root = tempfile::tempdir().unwrap();
        let windows = |identifier: &str| {
            let local = root.path().join("Local").join(identifier);
            AppDirs {
                cache_dir: local.clone(),
                config_dir: root.path().join("Roaming").join(identifier),
                data_dir: local.clone(),
                state_dir: local,
            }
        };
        let legacy = windows(LEGACY_IDENTIFIER);
        let current = windows("cn.vjoi.bilishadowreplay");
        write(&legacy.config_dir.join("Conf.toml"), "conf");
        write(&legacy.data_dir.join("output/clip.mp4"), "clip");
        write(&legacy.cache_dir.join("cache/bilibili/1/2/0.ts"), "ts");
        write(&current.config_dir.join("data_v2.db"), "db");
        write(&current.cache_dir.join("EBWebView/state"), "webview");

        migrate(&legacy, &current);

        assert_eq!(read(&current.config_dir.join("Conf.toml")), "conf");
        assert_eq!(read(&current.data_dir.join("output/clip.mp4")), "clip");
        assert_eq!(
            read(&current.cache_dir.join("cache/bilibili/1/2/0.ts")),
            "ts"
        );
        assert_eq!(read(&current.cache_dir.join("EBWebView/state")), "webview");
        assert!(!legacy.config_dir.exists());
        assert!(!legacy.data_dir.exists());
    }

    #[tokio::test]
    async fn uploads_of_moved_files_follow_them() {
        let root = tempfile::tempdir().unwrap();
        let legacy = dirs(root.path(), LEGACY_IDENTIFIER);
        let current = dirs(root.path(), "cn.vjoi.bilishadowreplay");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let db = Database::new(
            pool,
            crate::database::credentials::CredentialCipher::new(&[0; 32]).unwrap(),
            crate::events::EventBus::new(),
        );
        let path = |path: PathBuf| path.to_string_lossy().into_owned();
        let moved_clip = legacy.data_dir.join("output/clip.mp4");
        let elsewhere = root.path().join("Movies/clip.mp4");
        write(&moved_clip, "clip");
        write(&elsewhere, "clip");
        let upload = |source: &str, file: &PathBuf| crate::database::upload::UploadRow {
            id: 0,
            source: source.into(),
            source_id: 0,
            file: path(file.clone()),
            room_id: String::new(),
            uid: "1".into(),
            cover: String::new(),
            cover43: String::new(),
            profile: "{}".into(),
            danmu: false,
            status: "draft".into(),
            message: String::new(),
            bvid: String::new(),
            created_at: "2026-10-22T00:00:00+08:00".into(),
            updated_at: "2026-10-22T00:00:00+08:00".into(),
        };
        let moved = db.add_upload(&upload("file", &moved_clip)).await.unwrap();
        let kept = db.add_upload(&upload("file", &elsewhere)).await.unwrap();

        migrate(&legacy, &current);
        relocate_uploads(&db, &legacy, &current).await;

        assert_eq!(
            db.get_upload(moved.id).await.unwrap().file,
            path(current.data_dir.join("output/clip.mp4"))
        );
        assert_eq!(db.get_upload(kept.id).await.unwrap().file, path(elsewhere));
    }
}
