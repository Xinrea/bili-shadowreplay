//! Moves what older versions kept under the legacy identifier into the
//! directories of the app's identifier in `tauri.conf.json`.
//!
//! Older versions kept the config, the default cache and the default output
//! under `cn.vjoi.bili-shadowreplay`, while the database, the logs and the web
//! view used the app's identifier, `cn.vjoi.bilishadowreplay`.

use std::path::{Path, PathBuf};

use platform_dirs::AppDirs;

use crate::config::Config;
use crate::handlers::migrate::{migrate_entry, MigrateOutcome};

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
pub fn migrate(legacy: &AppDirs, current: &AppDirs) {
    for (from, to) in moves(legacy, current) {
        let Ok(entries) = std::fs::read_dir(&from) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match migrate_entry(&path, &to) {
                Ok(MigrateOutcome::SkippedExists) => {}
                Ok(_) => log::info!("Moved {} to {}", path.display(), to.display()),
                Err(e) => log::error!("Failed to move {} to {}: {e}", path.display(), to.display()),
            }
        }
        // Fails while anything is left, which stays for the next start.
        if std::fs::remove_dir(&from).is_ok() {
            log::info!("Removed legacy directory {}", from.display());
        }
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
}
