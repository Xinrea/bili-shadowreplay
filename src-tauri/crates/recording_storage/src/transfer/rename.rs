//! Metadata-only rename. The durable application journal is the ownership proof.
use super::*;

/// Constant-size (apart from paths) directory identity, not a payload snapshot.
/// Persist this exact value before rename and retain it until DB finalization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameManifest {
    namespace: String,
    source_root: PathBuf,
    target_root: PathBuf,
    source_identity: Identity,
    target_identity: Identity,
    directory_identity: Identity,
}

/// Only the OS's explicit cross-device error permits a copy fallback. In
/// particular, permission, conflict, unsupported-operation and sync errors do not.
pub fn is_cross_device_error(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::EXDEV)
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(17) // ERROR_NOT_SAME_DEVICE
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = error;
        false
    }
}

// Only path components, never namespace contents. Include ancestors ABOVE the
// configured roots: a bind mount can alias one of them without a lexical overlap.
fn ancestry(path: &Path) -> io::Result<Vec<Identity>> {
    checked_path(path)?;
    let mut result = Vec::new();
    for ancestor in path.ancestors() {
        if let Some(m) = metadata(ancestor)? {
            let id = identity(ancestor, &m)?;
            if result.contains(&id) {
                return Err(invalid("rename path aliases its own ancestor"));
            }
            result.push(id);
        }
    }
    Ok(result)
}

impl LocalTransfer {
    /// Root device/volume comparison captured at construction, not a guarantee:
    /// nested mounts may still make rename return EXDEV. Operations recheck roots.
    pub fn same_filesystem(&self) -> bool {
        self.source_identity.device == self.target_identity.device
    }

    /// Read-only O(1) preparation in archive size; O(path depth) metadata checks.
    /// Does not enumerate, open, hash, or follow anything inside the namespace.
    pub fn prepare_rename(&self, namespace: &str) -> io::Result<RenameManifest> {
        self.roots()?;
        validate_namespace(namespace)?;
        checked_path(&self.source.join(namespace))?;
        let manifest = RenameManifest {
            namespace: namespace.to_owned(),
            source_root: self.source.clone(),
            target_root: self.target.clone(),
            source_identity: self.source_identity.clone(),
            target_identity: self.target_identity.clone(),
            directory_identity: dir_identity(&self.source.join(namespace))?,
        };
        self.check_rename(namespace, &manifest)?;
        Ok(manifest)
    }

    /// Read-only recovery preflight. Accept exactly original source + absent
    /// target, or absent source + the exact moved directory at the target.
    pub fn check_rename(&self, namespace: &str, manifest: &RenameManifest) -> io::Result<()> {
        self.rename_moved(namespace, manifest).map(|_| ())
    }

    fn rename_moved(&self, namespace: &str, manifest: &RenameManifest) -> io::Result<bool> {
        self.roots()?;
        validate_namespace(namespace)?;
        if manifest.namespace != namespace
            || manifest.source_root != self.source
            || manifest.target_root != self.target
            || manifest.source_identity != self.source_identity
            || manifest.target_identity != self.target_identity
        {
            return Err(invalid("rename manifest roots or namespace changed"));
        }
        // A missing namespace can still be reserved by an interrupted copy.
        // Rename's DB proof must not overtake that ownership.
        if metadata(&self.proof_path(namespace))?.is_some() {
            return Err(conflict("destination reserved by a copy transfer"));
        }
        let source = self.source.join(namespace);
        let target = self.target.join(namespace);
        let source_ancestors = ancestry(source.parent().unwrap())?;
        let target_ancestors = ancestry(target.parent().unwrap())?;
        // Root aliases and namespace/ancestor aliases must not permit moving an
        // ancestor into itself, including aliases hidden by bind mounts.
        if source_ancestors.contains(&self.target_identity)
            || target_ancestors.contains(&self.source_identity)
            || source_ancestors.contains(&manifest.directory_identity)
            || target_ancestors.contains(&manifest.directory_identity)
        {
            return Err(invalid("rename namespace or roots alias an ancestor"));
        }
        let source_id = optional_directory(&source)?;
        let target_id = optional_directory(&target)?;
        match (source_id, target_id) {
            (Some(id), None) if id == manifest.directory_identity => Ok(false),
            (None, Some(id)) if id == manifest.directory_identity => Ok(true),
            _ => Err(conflict("rename source or destination identity conflict")),
        }
    }

    /// Atomic no-replace rename followed by durable parent barriers. A failed
    /// barrier may leave the directory moved: retry using the SAME manifest.
    /// Callers must stop writers and durably journal the manifest before calling.
    pub fn rename(&self, namespace: &str, manifest: &RenameManifest) -> io::Result<()> {
        if !self.rename_moved(namespace, manifest)? {
            let target = self.target.join(namespace);
            self.create_parents(target.parent().unwrap())?;
            // Recheck after mkdir, including any newly visible ancestor aliases.
            if !self.rename_moved(namespace, manifest)? {
                rename_new(&self.source.join(namespace), &target)?;
            }
        }
        self.finish_rename(namespace, manifest)
    }

    /// Verify the moved directory and repeat durability barriers. Never deletes
    /// anything; even a recreated empty source conflicts. No separate proof file
    /// exists: the application's durable journal authorizes recovery.
    pub fn finish_rename(&self, namespace: &str, manifest: &RenameManifest) -> io::Result<()> {
        if !self.rename_moved(namespace, manifest)? {
            return Err(conflict("rename has not published the destination"));
        }
        // Sync both sides on EVERY retry. Also persist created destination
        // ancestors, including those whose earlier mkdir succeeded but sync failed.
        for root in [&self.source, &self.target] {
            let directory = root.join(namespace);
            for ancestor in directory.parent().unwrap().ancestors() {
                sync_dir(ancestor)?;
                if ancestor == root {
                    break;
                }
            }
        }
        Ok(())
    }
}

fn optional_directory(path: &Path) -> io::Result<Option<Identity>> {
    match metadata(path)? {
        Some(m) if m.is_dir() => identity(path, &m).map(Some),
        Some(_) => Err(conflict("rename namespace is not a directory")),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FaultGuard;
    impl Drop for FaultGuard {
        fn drop(&mut self) {
            HASH_FAILURE.with(|failure| failure.set(false));
            SYNC_FAILURE.with(|failure| *failure.borrow_mut() = None);
        }
    }

    #[test]
    fn refuses_namespace_reserved_by_an_unpublished_copy() {
        let (_temp, transfer, manifest) = super::super::tests::fixture();
        let namespace = super::super::tests::NAMESPACE;
        let token = "00000000-0000-0000-0000-000000000001";
        let owned = transfer.create_owned(namespace, token, &manifest).unwrap();
        assert!(transfer.prepare_rename(namespace).is_err());
        assert!(transfer.source.join(namespace).exists());
        assert!(!transfer.target.join(namespace).exists());
        drop(owned);
        transfer.copy(namespace, token, &manifest).unwrap();
    }

    #[test]
    fn rename_never_hashes_payload_and_rejects_namespace_ancestor_alias_identity() {
        let (_temp, transfer, _) = super::super::tests::fixture();
        let _fault = FaultGuard;
        HASH_FAILURE.with(|failure| failure.set(true));
        assert!(transfer.inspect(super::super::tests::NAMESPACE).is_err());
        let namespace = super::super::tests::NAMESPACE;
        let manifest = transfer.prepare_rename(namespace).unwrap();
        // Exercise the same identity comparison used for a bind-mount alias,
        // without requiring privileged mount operations in the test suite.
        for ancestor in [&transfer.source, &transfer.target] {
            let mut alias = manifest.clone();
            alias.directory_identity = dir_identity(ancestor).unwrap();
            let error = transfer.check_rename(namespace, &alias).unwrap_err();
            assert!(error.to_string().contains("alias an ancestor"));
        }
        transfer.check_rename(namespace, &manifest).unwrap();
        transfer.rename(namespace, &manifest).unwrap();
        transfer.finish_rename(namespace, &manifest).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn retry_and_finish_repeat_both_parent_barriers_after_move() {
        for source in [false, true] {
            let (_temp, transfer, _) = super::super::tests::fixture();
            let fault = FaultGuard;
            let namespace = super::super::tests::NAMESPACE;
            let manifest = transfer.prepare_rename(namespace).unwrap();
            fs::create_dir_all(transfer.target.join("platform/room")).unwrap();
            let root = if source {
                &transfer.source
            } else {
                &transfer.target
            };
            SYNC_FAILURE.with(|failure| *failure.borrow_mut() = Some(root.join("platform/room")));
            assert!(transfer.rename(namespace, &manifest).is_err());
            assert!(!transfer.source.join(namespace).exists());
            assert!(transfer.target.join(namespace).is_dir());
            // Read-only preflight must not try to sync.
            transfer.check_rename(namespace, &manifest).unwrap();
            let restart = LocalTransfer::new(&transfer.source, &transfer.target).unwrap();
            assert!(restart.rename(namespace, &manifest).is_err());
            assert!(restart.finish_rename(namespace, &manifest).is_err());
            drop(fault);
            restart.rename(namespace, &manifest).unwrap();
            restart.finish_rename(namespace, &manifest).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn mkdir_barrier_failure_is_not_lost_when_ancestor_already_exists_on_retry() {
        let (_temp, transfer, _) = super::super::tests::fixture();
        let fault = FaultGuard;
        let namespace = super::super::tests::NAMESPACE;
        let manifest = transfer.prepare_rename(namespace).unwrap();
        SYNC_FAILURE.with(|failure| *failure.borrow_mut() = Some(transfer.target.join("platform")));
        assert!(transfer.rename(namespace, &manifest).is_err());
        assert!(transfer.source.join(namespace).is_dir());
        assert!(transfer.rename(namespace, &manifest).is_err());
        assert!(transfer.target.join(namespace).is_dir());
        assert!(transfer.finish_rename(namespace, &manifest).is_err());
        drop(fault);
        transfer.rename(namespace, &manifest).unwrap();
    }
}
