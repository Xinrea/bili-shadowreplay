//! Selective migration of BSR-owned files when the cache or output directory
//! changes.
//!
//! Users may point the cache / output setting at a directory that already holds
//! their own files, so migration must not move everything it finds. These
//! helpers build a list of entries BSR itself created and leave the rest alone.

use recorder::platforms::PlatformType;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Extensions of files that sit next to a clip and share its file stem.
/// Mirrors the cleanup list in `handlers::video::delete_video`.
const CLIP_SIDECAR_EXTENSIONS: [&str; 6] = ["jpg", "srt", "wav", "mp3", "opus", "ass"];

/// What a migration walk found in the source directory.
#[derive(Debug, Default)]
pub struct MigrationPlan {
    /// Entries BSR owns. Each is migrated as a unit (a file, or a whole folder).
    pub entries: Vec<PathBuf>,
    /// Entries left behind because they are not BSR's.
    pub skipped: Vec<PathBuf>,
}

/// Outcome of migrating a single entry.
#[derive(Debug, PartialEq, Eq)]
pub enum MigrateOutcome {
    /// Same filesystem: renamed in place, no bytes copied.
    Renamed,
    /// Across filesystems: copied then removed from the source.
    Copied,
    /// Destination already had this name; source left untouched.
    SkippedExists,
}

/// Plan a cache directory migration.
///
/// The cache layout is `<cache>/<platform>/<room_id>/<live_id>/...`, so only
/// top-level directories named after a known platform belong to BSR.
pub fn plan_cache_migration(src_root: &Path, dst_root: &Path) -> MigrationPlan {
    let platforms: HashSet<&str> = PlatformType::ALL.iter().map(|p| p.as_str()).collect();
    let mut plan = MigrationPlan::default();

    let Ok(entries) = std::fs::read_dir(src_root) else {
        return plan;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path == dst_root {
            continue;
        }

        let is_platform_dir = path.is_dir()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| platforms.contains(name));

        if is_platform_dir {
            plan.entries.push(path);
        } else {
            plan.skipped.push(path);
        }
    }

    plan
}

/// Plan an output (clip) directory migration.
///
/// The output directory is flat: clips are `.mp4` files, each optionally
/// accompanied by sidecars sharing its stem (cover, subtitles, audio samples,
/// danmaku ass). `filelist_*.txt` files are FFmpeg scratch files and are never
/// migrated. Sidecars without a matching clip are left behind.
pub fn plan_output_migration(src_root: &Path, dst_root: &Path) -> MigrationPlan {
    let mut plan = MigrationPlan::default();

    let Ok(entries) = std::fs::read_dir(src_root) else {
        return plan;
    };

    let mut files = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if path == dst_root {
            continue;
        }
        if path.is_dir() {
            plan.skipped.push(path);
        } else {
            files.push(path);
        }
    }

    // Clip stems drive the walk: `foo.mp4` claims `foo.jpg`, `foo.srt`, ...
    // `foo.tmp.mp4` yields stem `foo.tmp`, so leftover temp clips migrate on
    // their own rather than being mistaken for a sidecar.
    let clip_stems: HashSet<String> = files
        .iter()
        .filter(|path| has_extension(path, "mp4"))
        .filter_map(|path| file_stem(path))
        .collect();

    for path in files {
        if has_extension(&path, "mp4") {
            plan.entries.push(path);
            continue;
        }

        let is_sidecar = CLIP_SIDECAR_EXTENSIONS
            .iter()
            .any(|ext| has_extension(&path, ext))
            && file_stem(&path).is_some_and(|stem| clip_stems.contains(&stem));

        if is_sidecar {
            plan.entries.push(path);
        } else {
            plan.skipped.push(path);
        }
    }

    plan
}

/// Case-insensitive extension check, so `.MP4` counts too.
fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(expected))
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
}

/// Move one entry into `dst_root`, keeping its file name.
///
/// Tries `rename` first so a same-filesystem directory change costs nothing,
/// and falls back to copy + delete when the two paths live on different
/// filesystems. An existing destination is never overwritten.
pub fn migrate_entry(entry: &Path, dst_root: &Path) -> std::io::Result<MigrateOutcome> {
    let Some(file_name) = entry.file_name() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "entry has no file name",
        ));
    };
    let target = dst_root.join(file_name);

    if target.exists() {
        log::warn!(
            "Skip migrating {}: {} already exists",
            entry.display(),
            target.display()
        );
        return Ok(MigrateOutcome::SkippedExists);
    }

    std::fs::create_dir_all(dst_root)?;

    if std::fs::rename(entry, &target).is_ok() {
        return Ok(MigrateOutcome::Renamed);
    }

    // rename fails across filesystems (EXDEV); fall back to a real copy.
    if entry.is_dir() {
        crate::handlers::utils::copy_dir_all(entry, &target)?;
        std::fs::remove_dir_all(entry)?;
    } else {
        std::fs::copy(entry, &target)?;
        std::fs::remove_file(entry)?;
    }

    Ok(MigrateOutcome::Copied)
}

/// Migrate every entry in `plan` into `dst_root`.
///
/// Returns the number of entries actually moved. A failure on one entry aborts
/// the run: entries already moved stay at the destination, and the rest stay at
/// the source, so no data is lost either way.
pub fn run_plan(plan: &MigrationPlan, dst_root: &Path) -> std::io::Result<usize> {
    if !plan.skipped.is_empty() {
        log::info!(
            "Leaving {} non-BSR entries in place: {:?}",
            plan.skipped.len(),
            plan.skipped
                .iter()
                .take(10)
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
        );
    }

    let mut moved = 0;
    for entry in &plan.entries {
        match migrate_entry(entry, dst_root) {
            Ok(MigrateOutcome::SkippedExists) => {}
            Ok(_) => moved += 1,
            Err(e) => {
                log::error!("Failed to migrate {}: {e}", entry.display());
                return Err(e);
            }
        }
    }

    log::info!(
        "Migrated {moved}/{} entries to {}",
        plan.entries.len(),
        dst_root.display()
    );
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Isolated scratch directory, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "bsr_migrate_{tag}_{}",
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn file(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, b"x").unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        let mut names = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn cache_plan_picks_platform_dirs_only() {
        let src = TempDir::new("cache_src");
        let dst = TempDir::new("cache_dst");

        src.file("bilibili/1713546334/live_001/playlist.m3u8");
        src.file("douyin/123/live_002/danmu.txt");
        src.dir("我的电影收藏");
        src.file("家庭录像.mp4");
        src.file("notes.txt");

        let plan = plan_cache_migration(&src.0, &dst.0);

        assert_eq!(names(&plan.entries), vec!["bilibili", "douyin"]);
        assert_eq!(
            names(&plan.skipped),
            vec!["notes.txt", "家庭录像.mp4", "我的电影收藏"]
        );
    }

    #[test]
    fn cache_plan_skips_destination_nested_in_source() {
        let src = TempDir::new("cache_nested");
        src.file("bilibili/1/live/playlist.m3u8");
        let dst = src.dir("bilibili_new");

        let plan = plan_cache_migration(&src.0, &dst);

        assert_eq!(names(&plan.entries), vec!["bilibili"]);
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn output_plan_takes_clips_and_their_sidecars() {
        let src = TempDir::new("out_src");
        let dst = TempDir::new("out_dst");

        src.file("[1713546334][note][live_001][title][2026-08-14].mp4");
        src.file("[1713546334][note][live_001][title][2026-08-14].jpg");
        src.file("[1713546334][note][live_001][title][2026-08-14].srt");
        src.file("[1713546334][note][live_001][title][2026-08-14].opus");
        src.file("旅行vlog.mp4");
        src.file("旅行vlog.jpg");

        let plan = plan_output_migration(&src.0, &dst.0);

        assert_eq!(
            names(&plan.entries),
            vec![
                "[1713546334][note][live_001][title][2026-08-14].jpg",
                "[1713546334][note][live_001][title][2026-08-14].mp4",
                "[1713546334][note][live_001][title][2026-08-14].opus",
                "[1713546334][note][live_001][title][2026-08-14].srt",
                "旅行vlog.jpg",
                "旅行vlog.mp4",
            ]
        );
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn output_plan_leaves_scratch_orphans_and_dirs() {
        let src = TempDir::new("out_misc");
        let dst = TempDir::new("out_misc_dst");

        src.file("clip.mp4");
        src.file("filelist_aa08ca7df6f1319d.txt");
        src.file("孤立字幕.srt");
        src.file("readme.txt");
        src.dir("素材");

        let plan = plan_output_migration(&src.0, &dst.0);

        assert_eq!(names(&plan.entries), vec!["clip.mp4"]);
        assert_eq!(
            names(&plan.skipped),
            vec![
                "filelist_aa08ca7df6f1319d.txt",
                "readme.txt",
                "孤立字幕.srt",
                "素材",
            ]
        );
    }

    #[test]
    fn output_plan_keeps_temp_clip_as_its_own_entry() {
        let src = TempDir::new("out_tmp");
        let dst = TempDir::new("out_tmp_dst");

        src.file("clip.mp4");
        src.file("clip.tmp.mp4");
        src.file("clip.0.mp4");
        src.file("clip.ass");

        let plan = plan_output_migration(&src.0, &dst.0);

        assert_eq!(
            names(&plan.entries),
            vec!["clip.0.mp4", "clip.ass", "clip.mp4", "clip.tmp.mp4"]
        );
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn migrate_entry_moves_file_and_skips_existing() {
        let src = TempDir::new("move_src");
        let dst = TempDir::new("move_dst");

        let clip = src.file("clip.mp4");
        let outcome = migrate_entry(&clip, &dst.0).unwrap();

        assert_eq!(outcome, MigrateOutcome::Renamed);
        assert!(!clip.exists());
        assert!(dst.0.join("clip.mp4").exists());

        // A second clip with the same name must not clobber the destination.
        let again = src.file("clip.mp4");
        std::fs::write(&again, b"different").unwrap();
        let outcome = migrate_entry(&again, &dst.0).unwrap();

        assert_eq!(outcome, MigrateOutcome::SkippedExists);
        assert!(again.exists(), "source must survive a skipped migration");
        assert_eq!(std::fs::read(dst.0.join("clip.mp4")).unwrap(), b"x");
    }

    #[test]
    fn run_plan_moves_platform_dir_contents() {
        let src = TempDir::new("run_src");
        let dst = TempDir::new("run_dst");

        src.file("bilibili/1713546334/live_001/playlist.m3u8");
        src.file("我的视频.mp4");

        let plan = plan_cache_migration(&src.0, &dst.0);
        let moved = run_plan(&plan, &dst.0).unwrap();

        assert_eq!(moved, 1);
        assert!(dst
            .0
            .join("bilibili/1713546334/live_001/playlist.m3u8")
            .exists());
        assert!(!src.0.join("bilibili").exists());
        assert!(
            src.0.join("我的视频.mp4").exists(),
            "user files must stay in the old directory"
        );
    }
}
