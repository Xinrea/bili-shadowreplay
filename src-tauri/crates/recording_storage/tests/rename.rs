use std::{fs, io, path::PathBuf};

use recording_storage::{is_cross_device_error, LocalTransfer, RenameManifest};

const NAMESPACE: &str = "platform/room/live";

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    target: PathBuf,
    transfer: LocalTransfer,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let source = base.join("source");
        let target = base.join("target");
        fs::create_dir_all(source.join(NAMESPACE).join("empty")).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(source.join(NAMESPACE).join("file"), b"payload").unwrap();
        let transfer = LocalTransfer::new(&source, &target).unwrap();
        Self {
            _temp: temp,
            source,
            target,
            transfer,
        }
    }

    fn manifest(&self) -> RenameManifest {
        self.transfer.prepare_rename(NAMESPACE).unwrap()
    }

    fn restart(&self) -> LocalTransfer {
        LocalTransfer::new(&self.source, &self.target).unwrap()
    }
}

#[test]
fn preflight_is_read_only_and_manifest_is_small_serializable_and_namespace_bound() {
    let f = Fixture::new();
    assert!(f.transfer.same_filesystem());
    let manifest = f.manifest();
    let json = serde_json::to_vec(&manifest).unwrap();
    let saved: RenameManifest = serde_json::from_slice(&json).unwrap();
    assert_eq!(saved, manifest);
    assert!(json.len() < 4096);
    f.transfer.check_rename(NAMESPACE, &saved).unwrap();
    assert_eq!(fs::read_dir(&f.target).unwrap().count(), 0);
    assert!(f.transfer.finish_rename(NAMESPACE, &saved).is_err());
    for namespace in ["", "../escape", "other", ".recording-transfer-private"] {
        assert!(f.transfer.check_rename(namespace, &saved).is_err());
        assert!(f.transfer.rename(namespace, &saved).is_err());
        assert!(f.transfer.finish_rename(namespace, &saved).is_err());
    }
    for i in 0..100 {
        fs::write(f.source.join(NAMESPACE).join(format!("file-{i}")), b"x").unwrap();
    }
    // Directory mtime/ctime changes are not identity changes or payload snapshots.
    assert_eq!(f.manifest(), manifest);
}

#[test]
fn recovers_after_atomic_move_before_db_commit_and_round_trips_without_proofs() {
    let f = Fixture::new();
    let manifest = f.manifest();
    let saved = serde_json::to_vec(&manifest).unwrap();
    // Simulate the process exiting immediately after the filesystem rename.
    fs::create_dir_all(f.target.join("platform/room")).unwrap();
    fs::rename(f.source.join(NAMESPACE), f.target.join(NAMESPACE)).unwrap();
    let restart = f.restart();
    let saved = serde_json::from_slice(&saved).unwrap();
    restart.check_rename(NAMESPACE, &saved).unwrap();
    restart.rename(NAMESPACE, &saved).unwrap();
    restart.finish_rename(NAMESPACE, &saved).unwrap();
    restart.finish_rename(NAMESPACE, &saved).unwrap();
    assert!(!f.source.join(NAMESPACE).exists());
    assert_eq!(
        fs::read(f.target.join(NAMESPACE).join("file")).unwrap(),
        b"payload"
    );
    assert!(f.target.join(NAMESPACE).join("empty").is_dir());
    assert_eq!(fs::read_dir(&f.target).unwrap().count(), 1);

    let reverse = LocalTransfer::new(&f.target, &f.source).unwrap();
    let back = reverse.prepare_rename(NAMESPACE).unwrap();
    reverse.rename(NAMESPACE, &back).unwrap();
    reverse.finish_rename(NAMESPACE, &back).unwrap();
    assert_eq!(f.manifest(), manifest); // Same exact directory identity.
    let next = f.manifest();
    f.transfer.rename(NAMESPACE, &next).unwrap();
    f.transfer.finish_rename(NAMESPACE, &next).unwrap();
}

#[test]
fn recreating_either_root_invalidates_manifest_across_restart() {
    for moved in [false, true] {
        for source in [false, true] {
            let f = Fixture::new();
            let manifest = f.manifest();
            if moved {
                f.transfer.rename(NAMESPACE, &manifest).unwrap();
            }
            let root = if source { &f.source } else { &f.target };
            let old = root.with_extension("original");
            fs::rename(root, &old).unwrap();
            fs::create_dir(root).unwrap();
            // Move the exact archive into the replacement root as well: root
            // identity, not just namespace identity, must invalidate recovery.
            if old.join(NAMESPACE).exists() {
                fs::create_dir_all(root.join("platform/room")).unwrap();
                fs::rename(old.join(NAMESPACE), root.join(NAMESPACE)).unwrap();
            }
            for transfer in [&f.transfer, &f.restart()] {
                assert!(transfer.check_rename(NAMESPACE, &manifest).is_err());
                assert!(transfer.rename(NAMESPACE, &manifest).is_err());
                assert!(transfer.finish_rename(NAMESPACE, &manifest).is_err());
            }
        }
    }
}

#[test]
fn recreated_source_or_target_is_never_removed_or_accepted() {
    for moved in [false, true] {
        for source in [false, true] {
            let f = Fixture::new();
            let manifest = f.manifest();
            if moved {
                f.transfer.rename(NAMESPACE, &manifest).unwrap();
            }
            let root = if source { &f.source } else { &f.target };
            let path = root.join(NAMESPACE);
            if path.exists() {
                fs::rename(&path, root.join("original")).unwrap();
            }
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("foreign"), b"keep").unwrap();
            let restart = f.restart();
            assert!(restart.check_rename(NAMESPACE, &manifest).is_err());
            assert!(restart.rename(NAMESPACE, &manifest).is_err());
            assert!(restart.finish_rename(NAMESPACE, &manifest).is_err());
            assert_eq!(fs::read(path.join("foreign")).unwrap(), b"keep");
        }
    }
}

#[test]
fn empty_destination_and_file_destination_are_never_replaced() {
    for directory in [false, true] {
        let f = Fixture::new();
        let manifest = f.manifest();
        fs::create_dir_all(f.target.join("platform/room")).unwrap();
        let path = f.target.join(NAMESPACE);
        if directory {
            fs::create_dir(&path).unwrap();
        } else {
            fs::write(&path, b"foreign").unwrap();
        }
        assert!(f.transfer.prepare_rename(NAMESPACE).is_err());
        assert!(f.transfer.rename(NAMESPACE, &manifest).is_err());
        assert!(path.exists());
        assert_eq!(
            fs::read(f.source.join(NAMESPACE).join("file")).unwrap(),
            b"payload"
        );
    }
}

#[test]
fn cross_device_classification_uses_only_raw_platform_code() {
    #[cfg(unix)]
    let code = libc::EXDEV;
    #[cfg(windows)]
    let code = 17;
    #[cfg(any(unix, windows))]
    {
        assert!(is_cross_device_error(&io::Error::from_raw_os_error(code)));
        for raw in 0..256 {
            assert_eq!(
                is_cross_device_error(&io::Error::from_raw_os_error(raw)),
                raw == code
            );
        }
    }
    for kind in [
        io::ErrorKind::CrossesDevices,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::AlreadyExists,
        io::ErrorKind::Unsupported,
        io::ErrorKind::Other,
    ] {
        assert!(!is_cross_device_error(&io::Error::new(
            kind,
            "EXDEV error 17"
        )));
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

    #[test]
    fn preserves_directory_and_payload_inode_and_moves_opaque_unreadable_payload() {
        let f = Fixture::new();
        let from = f.source.join(NAMESPACE);
        let to = f.target.join(NAMESPACE);
        let original_directory = fs::metadata(&from).unwrap();
        let original_file = fs::metadata(from.join("file")).unwrap();
        fs::set_permissions(from.join("file"), fs::Permissions::from_mode(0o0)).unwrap();
        // A dangling link cannot be read and would be rejected by a tree scan.
        symlink("/nonexistent/opaque", from.join("opaque-link")).unwrap();
        f.transfer.inspect(NAMESPACE).unwrap_err();
        let manifest = f.manifest();
        f.transfer.rename(NAMESPACE, &manifest).unwrap();
        f.transfer.finish_rename(NAMESPACE, &manifest).unwrap();
        assert_eq!(fs::metadata(&to).unwrap().ino(), original_directory.ino());
        assert_eq!(fs::metadata(&to).unwrap().dev(), original_directory.dev());
        assert_eq!(
            fs::metadata(to.join("file")).unwrap().ino(),
            original_file.ino()
        );
        assert_eq!(
            fs::read_link(to.join("opaque-link")).unwrap(),
            PathBuf::from("/nonexistent/opaque")
        );
    }

    #[test]
    fn rejects_namespace_and_ancestor_symlinks_on_both_sides() {
        for source in [false, true] {
            for ancestor in [false, true] {
                let f = Fixture::new();
                let manifest = f.manifest();
                let root = if source { &f.source } else { &f.target };
                let path = root.join(if ancestor { "platform" } else { NAMESPACE });
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                let referent = root.join("original");
                if path.exists() {
                    fs::rename(&path, &referent).unwrap();
                } else {
                    fs::create_dir(&referent).unwrap();
                }
                symlink(&referent, &path).unwrap();
                assert!(f.transfer.prepare_rename(NAMESPACE).is_err());
                assert!(f.transfer.check_rename(NAMESPACE, &manifest).is_err());
                assert!(f.transfer.rename(NAMESPACE, &manifest).is_err());
                assert!(f.transfer.finish_rename(NAMESPACE, &manifest).is_err());
                assert!(referent.is_dir());
            }
        }
    }
}
