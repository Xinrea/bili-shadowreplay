use std::{
    fs, io,
    path::{Path, PathBuf},
};

use recording_storage::{LocalTransfer, TransferManifest};
use serde_json::Value;
use sha2::{Digest, Sha256};

const NAMESPACE: &str = "bilibili/123/live";
const TOKEN: &str = "24af58ca-c3d9-4b2a-9d44-81035b29ad00";
const OTHER_TOKEN: &str = "24af58ca-c3d9-4b2a-9d44-81035b29ad01";

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    target: PathBuf,
    transfer: LocalTransfer,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        // macOS's system temp location can itself contain a symlink (/var).
        // Transfer roots intentionally require symlink-free input paths.
        let base = temp.path().canonicalize().unwrap();
        let source = base.join("source");
        let target = base.join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        let transfer = LocalTransfer::new(&source, &target).unwrap();
        Self {
            _temp: temp,
            source,
            target,
            transfer,
        }
    }

    fn put(&self, path: &str, bytes: &[u8]) {
        let path = self.source.join(NAMESPACE).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn manifest(&self) -> TransferManifest {
        self.transfer.inspect(NAMESPACE).unwrap()
    }

    fn copy(&self, manifest: &TransferManifest) {
        self.transfer.copy(NAMESPACE, TOKEN, manifest).unwrap();
    }

    fn cleanup(&self, manifest: &TransferManifest) -> io::Result<()> {
        self.transfer.cleanup_source(NAMESPACE, TOKEN, manifest)
    }

    fn proof_path(&self) -> PathBuf {
        fs::read_dir(&self.target)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "json"))
            .unwrap()
    }

    fn stage(&self) -> PathBuf {
        let proof: Value = serde_json::from_slice(&fs::read(self.proof_path()).unwrap()).unwrap();
        self.target.join(proof["stage"].as_str().unwrap())
    }

    fn undo_publish(&self) {
        fs::rename(self.target.join(NAMESPACE), self.stage().join("archive")).unwrap();
    }
}

#[test]
fn transfers_every_private_byte_and_empty_directory_with_serializable_manifest() {
    let f = Fixture::new();
    let large: Vec<_> = (0..(3 * 64 * 1024 + 17)).map(|i| (i % 251) as u8).collect();
    let entries: Vec<(&str, &[u8])> = vec![
        ("playlist.m3u8", b"#EXTM3U"),
        ("media/0.ts", &large),
        (".sequence", b"123"),
        (".checkpoint/state.json", b"{\"complete\":false}"),
        (".recording-logs/events/0", b"private log chunk\0\xff"),
        (".recording-logs/events/count", b"\x01\0\0\0\0\0\0\0"),
        ("cover.jpg", b"archive-specific cover"),
        ("empty-file", b""),
    ];
    for (path, bytes) in &entries {
        f.put(path, bytes);
    }
    fs::create_dir_all(f.source.join(NAMESPACE).join("empty/nested")).unwrap();
    fs::write(
        f.source.join("bilibili/123/cover.jpg"),
        b"shared legacy cover",
    )
    .unwrap();
    let inspected = f.manifest();
    let json = serde_json::to_vec(&inspected).unwrap();
    let manifest: TransferManifest = serde_json::from_slice(&json).unwrap();
    assert_eq!(manifest, inspected);
    assert_eq!(
        manifest.total_bytes(),
        entries
            .iter()
            .map(|(_, bytes)| bytes.len() as u64)
            .sum::<u64>()
    );
    assert_eq!(manifest.files.len(), entries.len());
    for (path, bytes) in &entries {
        let file = manifest.files.iter().find(|f| f.path == *path).unwrap();
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        assert_eq!(file.sha256, hash);
        assert_eq!(file.size, bytes.len() as u64);
    }
    assert!(f.transfer.available_bytes().unwrap() > 0);
    assert_eq!(
        f.transfer
            .required_bytes(NAMESPACE, TOKEN, &manifest)
            .unwrap(),
        manifest.total_bytes()
    );
    f.copy(&manifest);
    assert_eq!(
        f.transfer
            .required_bytes(NAMESPACE, TOKEN, &manifest)
            .unwrap(),
        0
    );
    for (path, bytes) in &entries {
        assert_eq!(
            fs::read(f.target.join(NAMESPACE).join(path)).unwrap(),
            *bytes
        );
        assert_eq!(
            fs::read(f.source.join(NAMESPACE).join(path)).unwrap(),
            *bytes
        );
    }
    assert!(f.target.join(NAMESPACE).join("empty/nested").is_dir());
    assert!(!f.target.join("bilibili/123/cover.jpg").exists());
    // Ownership proof is strictly outside the published namespace.
    assert_eq!(fs::read_dir(f.target.join(NAMESPACE)).unwrap().count(), 8);
    f.cleanup(&manifest).unwrap();
    f.cleanup(&manifest).unwrap();
    assert!(!f.source.join(NAMESPACE).exists());
    assert_eq!(
        fs::read(f.source.join("bilibili/123/cover.jpg")).unwrap(),
        b"shared legacy cover"
    );
    f.transfer.finish(NAMESPACE, TOKEN).unwrap();
    f.transfer.finish(NAMESPACE, TOKEN).unwrap();
    assert!(fs::read_dir(&f.target).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".recording-transfer-")));
    assert!(f.target.join(NAMESPACE).is_dir());
}

#[test]
fn preflight_is_read_only_and_missing_roots_are_not_created() {
    let f = Fixture::new();
    assert!(f.transfer.inspect(NAMESPACE).is_err());
    f.put("file", b"content");
    let manifest = f.manifest();
    f.transfer
        .check_destination(NAMESPACE, TOKEN, &manifest)
        .unwrap();
    assert_eq!(fs::read_dir(&f.target).unwrap().count(), 0);
    let missing = f.target.join("missing");
    assert!(LocalTransfer::new(&f.source, &missing).is_err());
    assert!(!missing.exists());
    assert!(LocalTransfer::new(&missing, &f.target).is_err());
}

#[test]
fn malformed_unknown_proof_is_not_replaced() {
    let f = Fixture::new();
    f.put("file", b"content");
    let manifest = f.manifest();
    let proof = f.target.join(format!(
        ".recording-transfer-{:x}.json",
        Sha256::digest(NAMESPACE.as_bytes())
    ));
    fs::write(&proof, b"unrelated existing file").unwrap();
    assert!(f
        .transfer
        .check_destination(NAMESPACE, TOKEN, &manifest)
        .is_err());
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
    assert!(f.cleanup(&manifest).is_err());
    assert_eq!(fs::read(&proof).unwrap(), b"unrelated existing file");
    assert_eq!(fs::read_dir(&f.target).unwrap().count(), 1);
    assert!(f.source.join(NAMESPACE).join("file").is_file());
}

#[test]
fn rejects_equal_and_overlapping_roots_in_both_directions() {
    let f = Fixture::new();
    let child = f.source.join("child");
    fs::create_dir(&child).unwrap();
    assert!(LocalTransfer::new(&f.source, &f.source).is_err());
    assert!(LocalTransfer::new(&f.source, &child).is_err());
    assert!(LocalTransfer::new(&child, &f.source).is_err());
    assert!(LocalTransfer::new(&f.source, &child.join("..")).is_err());
    assert!(LocalTransfer::new(&f.source.join("."), &f.source).is_err());
}

#[test]
fn refuses_unowned_targets_even_identical_or_empty() {
    for bytes in [
        None,
        Some(b"content".as_slice()),
        Some(b"unrelated".as_slice()),
    ] {
        let f = Fixture::new();
        f.put("file", b"content");
        let manifest = f.manifest();
        fs::create_dir_all(f.target.join(NAMESPACE)).unwrap();
        if let Some(bytes) = bytes {
            fs::write(f.target.join(NAMESPACE).join("file"), bytes).unwrap();
        }
        assert_eq!(
            f.transfer
                .check_destination(NAMESPACE, TOKEN, &manifest)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
        assert!(f.cleanup(&manifest).is_err());
        assert_eq!(
            fs::read(f.source.join(NAMESPACE).join("file")).unwrap(),
            b"content"
        );
        if let Some(bytes) = bytes {
            assert_eq!(
                fs::read(f.target.join(NAMESPACE).join("file")).unwrap(),
                bytes
            );
        }
    }
}

#[test]
fn rejects_foreign_token_or_different_manifest_without_altering_any_bytes() {
    let f = Fixture::new();
    f.put("file", b"content");
    let manifest = f.manifest();
    f.copy(&manifest);
    assert!(f
        .transfer
        .check_destination(NAMESPACE, OTHER_TOKEN, &manifest)
        .is_err());
    assert!(f.transfer.copy(NAMESPACE, OTHER_TOKEN, &manifest).is_err());
    assert!(f
        .transfer
        .cleanup_source(NAMESPACE, OTHER_TOKEN, &manifest)
        .is_err());
    assert!(f.transfer.finish(NAMESPACE, OTHER_TOKEN).is_err());
    let mut different = manifest.clone();
    different.files[0].sha256[0] ^= 1;
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &different).is_err());
    assert_eq!(
        fs::read(f.source.join(NAMESPACE).join("file")).unwrap(),
        b"content"
    );
    assert_eq!(
        fs::read(f.target.join(NAMESPACE).join("file")).unwrap(),
        b"content"
    );
}

#[test]
fn malformed_paths_tokens_and_manifests_cannot_escape_roots() {
    let f = Fixture::new();
    f.put("file", b"content");
    let manifest = f.manifest();
    for namespace in [
        "",
        "../elsewhere",
        "/absolute",
        "bilibili//room",
        "x/../../y",
        ".recording-transfer-reserved",
    ] {
        assert!(f.transfer.inspect(namespace).is_err());
        assert!(f.transfer.copy(namespace, TOKEN, &manifest).is_err());
        assert!(f
            .transfer
            .cleanup_source(namespace, TOKEN, &manifest)
            .is_err());
        assert!(f.transfer.finish(namespace, TOKEN).is_err());
    }
    for token in [
        "",
        "../escape",
        "not-a-uuid",
        "24af58ca-c3d9-4b2a-9d44-81035b29ad0/",
    ] {
        assert!(f.transfer.copy(NAMESPACE, token, &manifest).is_err());
    }
    let mut invalid = manifest.clone();
    invalid.files[0].path = "../outside".into();
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &invalid).is_err());
    let mut invalid = manifest.clone();
    invalid.files.push(invalid.files[0].clone());
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &invalid).is_err());
    let mut invalid = manifest.clone();
    invalid.files[0].path = "absent-parent/file".into();
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &invalid).is_err());
    let mut invalid = manifest.clone();
    invalid.files[0].size += 1;
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &invalid).is_err());
    assert_eq!(fs::read_dir(&f.target).unwrap().count(), 0);
}

#[test]
fn published_retry_survives_restart_and_partial_source_cleanup() {
    let f = Fixture::new();
    f.put("a", b"a");
    f.put("nested/b", b"b");
    let manifest = f.manifest();
    f.copy(&manifest);
    fs::remove_file(f.source.join(NAMESPACE).join("a")).unwrap();
    let restart = LocalTransfer::new(&f.source, &f.target).unwrap();
    restart
        .check_destination(NAMESPACE, TOKEN, &manifest)
        .unwrap();
    restart.copy(NAMESPACE, TOKEN, &manifest).unwrap();
    assert_eq!(
        restart.required_bytes(NAMESPACE, TOKEN, &manifest).unwrap(),
        0
    );
    restart.cleanup_source(NAMESPACE, TOKEN, &manifest).unwrap();
    restart.cleanup_source(NAMESPACE, TOKEN, &manifest).unwrap();
    assert_eq!(fs::read(f.target.join(NAMESPACE).join("a")).unwrap(), b"a");
    assert_eq!(
        fs::read(f.target.join(NAMESPACE).join("nested/b")).unwrap(),
        b"b"
    );
}

#[test]
fn partial_stage_resumes_without_replacing_completed_files() {
    let f = Fixture::new();
    f.put("a", b"a");
    f.put("nested/b", b"b");
    let manifest = f.manifest();
    f.copy(&manifest);
    f.undo_publish();
    let archive = f.stage().join("archive");
    fs::remove_file(archive.join("nested/b")).unwrap();
    fs::remove_dir(archive.join("nested")).unwrap();
    let before = fs::metadata(archive.join("a")).unwrap().modified().unwrap();
    f.transfer
        .check_destination(NAMESPACE, TOKEN, &manifest)
        .unwrap();
    assert_eq!(
        f.transfer
            .required_bytes(NAMESPACE, TOKEN, &manifest)
            .unwrap(),
        manifest.total_bytes()
    );
    assert!(f.cleanup(&manifest).is_err());
    f.copy(&manifest);
    assert_eq!(
        fs::metadata(f.target.join(NAMESPACE).join("a"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
    assert_eq!(
        fs::read(f.target.join(NAMESPACE).join("nested/b")).unwrap(),
        b"b"
    );
}

#[test]
fn interrupted_scratch_write_is_not_published_and_does_not_block_retry() {
    let f = Fixture::new();
    f.put("large", &vec![42; 64 * 1024 + 1]);
    let manifest = f.manifest();
    f.copy(&manifest);
    f.undo_publish();
    let stage = f.stage();
    fs::remove_file(stage.join("archive/large")).unwrap();
    fs::write(stage.join("scratch/interrupted-write"), b"partial").unwrap();
    f.copy(&manifest);
    assert_eq!(
        fs::read(f.target.join(NAMESPACE).join("large")).unwrap(),
        vec![42; 64 * 1024 + 1]
    );
    assert!(!f.target.join(NAMESPACE).join("interrupted-write").exists());
    f.cleanup(&manifest).unwrap();
    f.transfer.finish(NAMESPACE, TOKEN).unwrap();
    assert!(!stage.exists());
}

#[test]
fn unknown_or_corrupt_staged_files_are_never_overwritten() {
    for unknown in [false, true] {
        let f = Fixture::new();
        f.put("file", b"content");
        let manifest = f.manifest();
        f.copy(&manifest);
        f.undo_publish();
        let path = f.stage().join(if unknown {
            "archive/unknown"
        } else {
            "archive/file"
        });
        fs::write(&path, b"foreign").unwrap();
        assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
        assert!(!f.target.join(NAMESPACE).exists());
        assert_eq!(fs::read(&path).unwrap(), b"foreign");
        assert_eq!(
            fs::read(f.source.join(NAMESPACE).join("file")).unwrap(),
            b"content"
        );
    }
}

#[test]
fn changed_source_aborts_copy_without_publishing_or_removing_source() {
    let f = Fixture::new();
    f.put("file", b"content");
    let manifest = f.manifest();
    f.put("file", b"changed");
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
    assert!(!f.target.join(NAMESPACE).exists());
    assert_eq!(
        fs::read(f.source.join(NAMESPACE).join("file")).unwrap(),
        b"changed"
    );
    assert_eq!(fs::read_dir(&f.target).unwrap().count(), 0);
}

#[test]
fn cleanup_preflights_every_file_before_deleting_any_source() {
    for change in ["modify", "add", "recreate"] {
        let f = Fixture::new();
        f.put("a-first", b"keep until all verified");
        f.put("z-last", b"original");
        let manifest = f.manifest();
        f.copy(&manifest);
        match change {
            "modify" => f.put("z-last", b"modified"),
            "add" => f.put("new-file", b"unlisted"),
            "recreate" => {
                let old = f.source.join(NAMESPACE).join("z-last");
                // Keep the original alive to guarantee a distinct inode.
                fs::rename(&old, f.source.join("original-kept")).unwrap();
                f.put("z-last", b"original");
            }
            _ => unreachable!(),
        }
        assert!(f.cleanup(&manifest).is_err(), "{change}");
        assert_eq!(
            fs::read(f.source.join(NAMESPACE).join("a-first")).unwrap(),
            b"keep until all verified"
        );
        assert!(f.source.join(NAMESPACE).join("z-last").exists());
    }
}

#[test]
fn cleanup_rejects_recreated_empty_namespace_and_directories() {
    for root in [false, true] {
        let f = Fixture::new();
        f.put("a", b"retain");
        fs::create_dir(f.source.join(NAMESPACE).join("empty")).unwrap();
        let manifest = f.manifest();
        f.copy(&manifest);
        let path = if root {
            f.source.join(NAMESPACE)
        } else {
            f.source.join(NAMESPACE).join("empty")
        };
        fs::rename(&path, f.source.join("kept-original")).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(f.cleanup(&manifest).is_err());
        assert!(path.is_dir());
        if !root {
            assert!(f.source.join(NAMESPACE).join("a").is_file());
        }
    }
}

#[test]
fn cleanup_retry_reverifies_target_before_deleting_remaining_source() {
    for mutation in ["modify", "delete", "add", "replace-root"] {
        let f = Fixture::new();
        f.put("a", b"a");
        f.put("b", b"b");
        let manifest = f.manifest();
        f.copy(&manifest);
        fs::remove_file(f.source.join(NAMESPACE).join("a")).unwrap();
        let target = f.target.join(NAMESPACE);
        match mutation {
            "modify" => fs::write(target.join("a"), b"x").unwrap(),
            "delete" => fs::remove_file(target.join("a")).unwrap(),
            "add" => fs::write(target.join("new"), b"unlisted").unwrap(),
            "replace-root" => {
                fs::rename(&target, f.target.join("original-kept")).unwrap();
                fs::create_dir(&target).unwrap();
                fs::write(target.join("a"), b"a").unwrap();
                fs::write(target.join("b"), b"b").unwrap();
            }
            _ => unreachable!(),
        }
        assert!(f.cleanup(&manifest).is_err(), "{mutation}");
        assert_eq!(fs::read(f.source.join(NAMESPACE).join("b")).unwrap(), b"b");
    }
}

#[test]
fn cleanup_retry_allows_missing_directories_but_not_new_empty_directories() {
    let f = Fixture::new();
    f.put("a", b"a");
    f.put("gone/b", b"b");
    let manifest = f.manifest();
    f.copy(&manifest);
    fs::remove_file(f.source.join(NAMESPACE).join("gone/b")).unwrap();
    fs::remove_dir(f.source.join(NAMESPACE).join("gone")).unwrap();
    fs::create_dir(f.source.join(NAMESPACE).join("new-empty")).unwrap();
    assert!(f.cleanup(&manifest).is_err());
    assert!(f.source.join(NAMESPACE).join("a").exists());
    fs::remove_dir(f.source.join(NAMESPACE).join("new-empty")).unwrap();
    f.cleanup(&manifest).unwrap();
}

#[test]
fn finish_requires_cleaned_source_and_preserves_published_archive() {
    let f = Fixture::new();
    f.put("file", b"content");
    let manifest = f.manifest();
    f.copy(&manifest);
    assert!(f.transfer.finish(NAMESPACE, TOKEN).is_err());
    f.cleanup(&manifest).unwrap();
    let proof = f.proof_path();
    f.transfer.finish(NAMESPACE, TOKEN).unwrap();
    assert!(!proof.exists());
    f.transfer.finish(NAMESPACE, TOKEN).unwrap();
    assert!(f.cleanup(&manifest).is_err()); // No live journal may outlive finish.
    assert_eq!(
        fs::read(f.target.join(NAMESPACE).join("file")).unwrap(),
        b"content"
    );
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{symlink, MetadataExt};

    #[test]
    fn rejects_symlink_roots_and_symlink_ancestor_aliases() {
        let f = Fixture::new();
        let alias = f.target.join("alias");
        symlink(&f.source, &alias).unwrap();
        assert!(LocalTransfer::new(&alias, &f.target).is_err());
        assert!(LocalTransfer::new(&f.source, &alias).is_err());
        fs::create_dir(f.source.join("nested")).unwrap();
        assert!(LocalTransfer::new(&alias.join("nested"), &f.target).is_err());
    }

    #[test]
    fn rejects_symlinks_in_source_files_directories_and_ancestors() {
        for relative in ["file-link", "dir-link", ".private-link"] {
            let f = Fixture::new();
            f.put("file", b"content");
            symlink(
                f.source.join(NAMESPACE).join("file"),
                f.source.join(NAMESPACE).join(relative),
            )
            .unwrap();
            assert!(f.transfer.inspect(NAMESPACE).is_err());
        }
        let f = Fixture::new();
        fs::create_dir_all(f.source.join("elsewhere/123/live")).unwrap();
        symlink(f.source.join("elsewhere"), f.source.join("bilibili")).unwrap();
        assert!(f.transfer.inspect(NAMESPACE).is_err());
    }

    #[test]
    fn rejects_target_ancestor_and_namespace_symlinks_without_touching_referents() {
        for ancestor in [false, true] {
            let f = Fixture::new();
            f.put("file", b"content");
            let manifest = f.manifest();
            let elsewhere = f.target.join("elsewhere");
            fs::create_dir(&elsewhere).unwrap();
            if ancestor {
                symlink(&elsewhere, f.target.join("bilibili")).unwrap();
            } else {
                fs::create_dir_all(f.target.join("bilibili/123")).unwrap();
                symlink(&elsewhere, f.target.join(NAMESPACE)).unwrap();
            }
            assert!(f
                .transfer
                .check_destination(NAMESPACE, TOKEN, &manifest)
                .is_err());
            assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
            assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        }
    }

    #[test]
    fn cleanup_rejects_source_and_target_symlinks_on_retry() {
        for source in [false, true] {
            let f = Fixture::new();
            f.put("a", b"a");
            f.put("b", b"b");
            let manifest = f.manifest();
            f.copy(&manifest);
            let root = if source { &f.source } else { &f.target };
            let file = root.join(NAMESPACE).join("b");
            let referent = root.join("referent");
            fs::rename(&file, &referent).unwrap();
            symlink(&referent, &file).unwrap();
            assert!(f.cleanup(&manifest).is_err());
            assert!(f.source.join(NAMESPACE).join("a").exists());
            assert_eq!(fs::read(&referent).unwrap(), b"b");
        }
    }

    #[test]
    fn recreated_identical_file_with_original_mtime_is_not_owned() {
        let f = Fixture::new();
        f.put("file", b"same bytes");
        let manifest = f.manifest();
        f.copy(&manifest);
        let path = f.source.join(NAMESPACE).join("file");
        let metadata = fs::metadata(&path).unwrap();
        fs::rename(&path, f.source.join("keep-inode")).unwrap();
        f.put("file", b"same bytes");
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_times(fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
            .unwrap();
        assert_ne!(fs::metadata(&path).unwrap().ino(), metadata.ino());
        assert!(f.cleanup(&manifest).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"same bytes");
    }

    #[test]
    fn hardlinked_archive_entries_are_rejected_before_any_transfer() {
        let f = Fixture::new();
        f.put("file", b"content");
        fs::hard_link(
            f.source.join(NAMESPACE).join("file"),
            f.source.join(NAMESPACE).join("alias"),
        )
        .unwrap();
        assert!(f.transfer.inspect(NAMESPACE).is_err());
        assert_eq!(fs::read_dir(&f.target).unwrap().count(), 0);
        assert!(f.source.join(NAMESPACE).join("file").is_file());
        assert!(f.source.join(NAMESPACE).join("alias").is_file());
    }

    #[test]
    fn fifo_is_rejected_without_blocking() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let f = Fixture::new();
        f.put("file", b"content");
        let fifo = f.source.join(NAMESPACE).join("fifo");
        let cpath = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        assert!(f.transfer.inspect(NAMESPACE).is_err());
    }

    #[test]
    fn proof_and_owned_staging_symlinks_are_rejected() {
        for proof in [false, true] {
            let f = Fixture::new();
            f.put("file", b"content");
            let manifest = f.manifest();
            f.copy(&manifest);
            let path = if proof { f.proof_path() } else { f.stage() };
            let original = f.target.join("original-kept");
            fs::rename(&path, &original).unwrap();
            symlink(&original, &path).unwrap();
            assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
            assert!(f.cleanup(&manifest).is_err());
            assert!(f.source.join(NAMESPACE).join("file").exists());
        }
    }
}

#[test]
fn file_roots_and_file_ancestors_are_rejected() {
    let f = Fixture::new();
    let file = f.target.join("file");
    fs::write(&file, b"not directory").unwrap();
    assert!(LocalTransfer::new(&f.source, &file).is_err());
    assert!(LocalTransfer::new(&file, &f.target).is_err());
    f.put("file", b"content");
    let manifest = f.manifest();
    fs::write(f.target.join("bilibili"), b"not directory").unwrap();
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
}

#[test]
fn root_replacement_invalidates_existing_transfer_and_manifest() {
    let f = Fixture::new();
    f.put("file", b"content");
    let manifest = f.manifest();
    fs::rename(&f.source, f.target.join("old-source")).unwrap();
    fs::create_dir(&f.source).unwrap();
    assert!(f.transfer.available_bytes().is_err());
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
    let restart = LocalTransfer::new(&f.source, &f.target).unwrap();
    assert!(restart.copy(NAMESPACE, TOKEN, &manifest).is_err());
}

#[test]
fn existing_target_file_is_a_conflict_not_an_overwrite() {
    let f = Fixture::new();
    f.put("file", b"content");
    let manifest = f.manifest();
    fs::create_dir_all(f.target.join("bilibili/123")).unwrap();
    fs::write(f.target.join(NAMESPACE), b"keep").unwrap();
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
    assert_eq!(fs::read(f.target.join(NAMESPACE)).unwrap(), b"keep");
}

// Keep Path imported for an explicit compile-time assertion that the public
// constructor needs neither a Tokio runtime nor any application filesystem I/O.
#[test]
fn api_is_synchronous_and_send() {
    fn assert_send<T: Send>() {}
    assert_send::<LocalTransfer>();
    assert_send::<TransferManifest>();
    let _: fn(&Path, &Path) -> io::Result<LocalTransfer> = LocalTransfer::new;
}

#[test]
fn concurrent_mutation_is_rejected_by_owned_proof_lock() {
    let f = Fixture::new();
    f.put("file", b"content");
    let manifest = f.manifest();
    f.copy(&manifest);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.proof_path())
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    assert!(f.transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
    assert!(f.cleanup(&manifest).is_err());
    assert!(f.source.join(NAMESPACE).join("file").exists());
    drop(lock);
    f.copy(&manifest);
    f.cleanup(&manifest).unwrap();
}

#[test]
fn adding_a_store_lease_retains_the_existing_provider_lease() {
    use std::sync::Arc;
    let provider = Arc::new(());
    let transfer = Arc::new(());
    let f = Fixture::new();
    let store = recording_storage::RecordingStore::local(&f.source)
        .with_lease(provider.clone())
        .with_lease(transfer.clone());
    assert_eq!(Arc::strong_count(&provider), 2);
    assert_eq!(Arc::strong_count(&transfer), 2);
    let recording = store.recording("bilibili", "123", "live").unwrap();
    drop(store);
    assert_eq!(Arc::strong_count(&provider), 2);
    assert_eq!(Arc::strong_count(&transfer), 2);
    drop(recording);
    assert_eq!(Arc::strong_count(&provider), 1);
    assert_eq!(Arc::strong_count(&transfer), 1);
}
