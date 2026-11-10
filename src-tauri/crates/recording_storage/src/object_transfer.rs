//! Recoverable, no-clobber transfers across object backends.
//!
//! The application owns the durable journal and exclusive archive leases. It
//! must persist inspect()'s manifest and token before copy(), commit the binding
//! before cleanup_source(), and persist finalizing before finish(). Endpoints
//! must be application-controlled: S3 has no portable multi-object transaction
//! or conditional delete, so independent external writers must be stopped.
use std::{collections::BTreeSet, io, path::Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::{validate_key, RecordingStorage};

const PROOF_KEY: &str = ".recording-transfer.json";
const MAX_FILES: usize = 250_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ObjectFile {
    key: String,
    size: u64,
    sha256: [u8; 32],
    revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectTransferManifest {
    version: u32,
    namespace: String,
    source_identity: String,
    target_identity: String,
    files: Vec<ObjectFile>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Proof {
    version: u32,
    token: String,
    manifest_digest: [u8; 32],
}

pub struct ObjectTransfer {
    source: RecordingStorage,
    target: RecordingStorage,
    namespace: String,
    source_identity: String,
    target_identity: String,
}

fn conflict(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, message)
}

async fn keys(storage: &RecordingStorage) -> io::Result<Vec<String>> {
    let mut keys = Vec::new();
    let mut cursor = None;
    loop {
        let page = storage.list("", cursor.as_deref(), 256).await?;
        if page.keys.is_empty() && page.next_cursor.is_some() {
            return Err(io::Error::other("empty nonterminal transfer page"));
        }
        for key in page.keys {
            if keys.last().is_some_and(|last| last >= &key) || keys.len() == MAX_FILES {
                return Err(io::Error::other("unordered or oversized transfer listing"));
            }
            keys.push(key);
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok(keys);
        }
    }
}

async fn hash_file(path: &Path) -> io::Result<(u64, [u8; 32])> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            return Ok((size, hash.finalize().into()));
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("transfer size overflow"))?;
        hash.update(&buffer[..count]);
    }
}

async fn snapshot(storage: &RecordingStorage, key: &str, path: &Path) -> io::Result<ObjectFile> {
    let revision = storage.backend.transfer_revision(key).await?;
    let size = storage.size(key).await?;
    if size
        > fs2::available_space(
            path.parent()
                .ok_or_else(|| io::Error::other("missing scratch parent"))?,
        )?
    {
        return Err(io::Error::other(
            "系统临时目录空间不足，须容纳单个待转移对象",
        ));
    }
    storage.backend.download_to_file(key, path).await?;
    let (actual_size, sha256) = hash_file(path).await?;
    if actual_size != size || storage.backend.transfer_revision(key).await? != revision {
        return Err(conflict("object changed while reading transfer snapshot"));
    }
    Ok(ObjectFile {
        key: key.into(),
        size,
        sha256,
        revision,
    })
}

impl ObjectTransfer {
    pub fn new(
        source: RecordingStorage,
        target: RecordingStorage,
        namespace: String,
        source_identity: String,
        target_identity: String,
    ) -> io::Result<Self> {
        validate_key(&namespace)?;
        Ok(Self {
            source,
            target,
            namespace,
            source_identity,
            target_identity,
        })
    }

    fn validate(&self, manifest: &ObjectTransferManifest) -> io::Result<()> {
        if manifest.version != 1
            || manifest.namespace != self.namespace
            || manifest.source_identity != self.source_identity
            || manifest.target_identity != self.target_identity
            || manifest.files.is_empty()
            || manifest.files.len() > MAX_FILES
        {
            return Err(conflict(
                "transfer manifest does not match its endpoints or namespace",
            ));
        }
        let mut last = None;
        for file in &manifest.files {
            validate_key(&file.key)?;
            if file.key == PROOF_KEY
                || file.key.starts_with(&format!("{PROOF_KEY}/"))
                || file.revision.is_empty()
                || last.is_some_and(|last: &str| last >= file.key.as_str())
            {
                return Err(io::Error::other("invalid object transfer manifest"));
            }
            last = Some(file.key.as_str());
        }
        Ok(())
    }

    fn proof(&self, token: &str, manifest: &ObjectTransferManifest) -> io::Result<Proof> {
        self.validate(manifest)?;
        validate_key(token)?;
        Ok(Proof {
            version: 1,
            token: token.into(),
            manifest_digest: Sha256::digest(
                serde_json::to_vec(manifest).map_err(io::Error::other)?,
            )
            .into(),
        })
    }

    async fn read_proof(&self) -> io::Result<Proof> {
        let bytes = self.target.read_bounded(PROOF_KEY, 4096).await?;
        serde_json::from_slice(&bytes).map_err(|_| conflict("invalid destination transfer proof"))
    }

    pub async fn inspect(&self) -> io::Result<ObjectTransferManifest> {
        if !self.target.list("", None, 1).await?.keys.is_empty() {
            return Err(conflict("目标录播命名空间已存在，不能覆盖或接管已有文件"));
        }
        let scratch = tempfile::tempdir()?;
        let path = scratch.path().join("object");
        let mut files = Vec::new();
        for key in keys(&self.source).await? {
            files.push(snapshot(&self.source, &key, &path).await?);
            tokio::fs::remove_file(&path).await?;
        }
        let manifest = ObjectTransferManifest {
            version: 1,
            namespace: self.namespace.clone(),
            source_identity: self.source_identity.clone(),
            target_identity: self.target_identity.clone(),
            files,
        };
        self.validate(&manifest)?;
        Ok(manifest)
    }

    async fn claim(&self, proof: &Proof) -> io::Result<()> {
        match self.read_proof().await {
            Ok(existing) if existing == *proof => return Ok(()),
            Ok(_) => return Err(conflict("destination belongs to a different transfer")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if !self.target.list("", None, 1).await?.keys.is_empty() {
            return Err(conflict("目标已有文件且没有本次转移凭据，不能覆盖"));
        }
        self.target.prepare().await?;
        self.target
            .backend
            .create(
                PROOF_KEY,
                &serde_json::to_vec(proof).map_err(io::Error::other)?,
            )
            .await
    }

    async fn check_keys(
        &self,
        manifest: &ObjectTransferManifest,
        source: bool,
        allow_missing: bool,
    ) -> io::Result<()> {
        let mut expected: BTreeSet<_> =
            manifest.files.iter().map(|file| file.key.clone()).collect();
        if !source {
            expected.insert(PROOF_KEY.into());
        }
        let actual: BTreeSet<_> = keys(if source { &self.source } else { &self.target })
            .await?
            .into_iter()
            .collect();
        if (!allow_missing && actual != expected) || !actual.is_subset(&expected) {
            return Err(conflict("转移文件清单发生变化或出现未知文件"));
        }
        Ok(())
    }

    pub async fn copy(&self, token: &str, manifest: &ObjectTransferManifest) -> io::Result<()> {
        let proof = self.proof(token, manifest)?;
        self.check_keys(manifest, true, false).await?;
        self.claim(&proof).await?;
        self.check_keys(manifest, false, true).await?;
        let scratch = tempfile::tempdir()?;
        let path = scratch.path().join("object");
        for file in &manifest.files {
            let actual = snapshot(&self.source, &file.key, &path).await?;
            if actual != *file {
                return Err(conflict("来源对象已修改，不能继续原转移"));
            }
            if !self.target.exists(&file.key).await? {
                self.target
                    .backend
                    .create_from_file(&file.key, &path)
                    .await?;
            }
            tokio::fs::remove_file(&path).await?;
            self.verify_target_file(file, &path).await?;
            tokio::fs::remove_file(&path).await?;
        }
        self.check_keys(manifest, true, false).await?;
        for file in &manifest.files {
            if self.source.backend.transfer_revision(&file.key).await? != file.revision {
                return Err(conflict("source object changed before binding commit"));
            }
        }
        self.verify_destination(token, manifest).await
    }

    async fn verify_target_file(&self, expected: &ObjectFile, path: &Path) -> io::Result<()> {
        let actual = snapshot(&self.target, &expected.key, path).await?;
        if actual.size != expected.size || actual.sha256 != expected.sha256 {
            return Err(conflict("目标对象校验失败，保留来源；不会覆盖已有目标对象"));
        }
        Ok(())
    }

    pub async fn verify_destination(
        &self,
        token: &str,
        manifest: &ObjectTransferManifest,
    ) -> io::Result<()> {
        let proof = self.proof(token, manifest)?;
        if self.read_proof().await? != proof {
            return Err(conflict("destination transfer proof changed"));
        }
        self.check_keys(manifest, false, false).await?;
        let scratch = tempfile::tempdir()?;
        let path = scratch.path().join("object");
        for file in &manifest.files {
            self.verify_target_file(file, &path).await?;
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }

    pub async fn cleanup_source(
        &self,
        token: &str,
        manifest: &ObjectTransferManifest,
    ) -> io::Result<()> {
        // Verify ALL target bytes and remaining source objects before deleting
        // any source. Missing source keys are allowed only in this post-commit phase.
        self.verify_destination(token, manifest).await?;
        self.check_keys(manifest, true, true).await?;
        let scratch = tempfile::tempdir()?;
        let path = scratch.path().join("object");
        for file in &manifest.files {
            if self.source.exists(&file.key).await? {
                if snapshot(&self.source, &file.key, &path).await? != *file {
                    return Err(conflict("来源对象已修改，拒绝清理"));
                }
                tokio::fs::remove_file(&path).await?;
            }
        }
        for file in &manifest.files {
            if self.source.exists(&file.key).await? {
                if self.source.backend.transfer_revision(&file.key).await? != file.revision {
                    return Err(conflict("source object changed before removal"));
                }
                self.source.remove(&file.key).await?;
            }
        }
        if !keys(&self.source).await?.is_empty() {
            return Err(conflict("source acquired unknown objects during cleanup"));
        }
        Ok(())
    }

    pub async fn finish(&self, token: &str, manifest: &ObjectTransferManifest) -> io::Result<()> {
        let expected = self.proof(token, manifest)?;
        match self.read_proof().await {
            Ok(proof) if proof == expected => self.target.remove(PROOF_KEY).await,
            Ok(_) => Err(conflict("destination transfer proof changed")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RecordingStore;

    async fn fixture() -> (tempfile::TempDir, ObjectTransfer) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = RecordingStore::local(root.join("source"))
            .scope("bilibili/room/live")
            .unwrap();
        let target = RecordingStore::local(root.join("target"))
            .scope("bilibili/room/live")
            .unwrap();
        source.prepare().await.unwrap();
        for (key, bytes) in [
            ("a.ts", b"segment".as_slice()),
            ("b.ts", b"another"),
            ("empty", b""),
            (".private/checkpoint", b"resume"),
        ] {
            source.write(key, bytes).await.unwrap();
        }
        source
            .open_log("events.jsonl")
            .await
            .unwrap()
            .append(b"event\n")
            .await
            .unwrap();
        (
            temp,
            ObjectTransfer::new(
                source,
                target,
                "bilibili/room/live".into(),
                "a".into(),
                "b".into(),
            )
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn partial_copy_resumes_without_overwriting_and_cleanup_can_restart() {
        let (_temp, transfer) = fixture().await;
        let manifest = transfer.inspect().await.unwrap();
        let manifest: ObjectTransferManifest =
            serde_json::from_str(&serde_json::to_string(&manifest).unwrap()).unwrap();
        let proof = transfer.proof("token", &manifest).unwrap();
        transfer.claim(&proof).await.unwrap();
        transfer
            .target
            .backend
            .create("a.ts", b"segment")
            .await
            .unwrap();
        let revision = transfer
            .target
            .backend
            .transfer_revision("a.ts")
            .await
            .unwrap();
        assert_eq!(
            transfer
                .target
                .backend
                .create("a.ts", b"overwrite")
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        transfer.copy("token", &manifest).await.unwrap();
        assert_eq!(
            transfer
                .target
                .backend
                .transfer_revision("a.ts")
                .await
                .unwrap(),
            revision
        );
        assert_eq!(
            transfer.target.read_log("events.jsonl").await.unwrap(),
            b"event\n"
        );
        // Simulate a committed binding and an interrupted source cleanup.
        transfer.source.remove("a.ts").await.unwrap();
        transfer.cleanup_source("token", &manifest).await.unwrap();
        assert!(keys(&transfer.source).await.unwrap().is_empty());
        transfer.finish("token", &manifest).await.unwrap();
        transfer.finish("token", &manifest).await.unwrap();
        assert_eq!(
            transfer.target.read(".private/checkpoint").await.unwrap(),
            b"resume"
        );
        assert!(!transfer.target.exists(PROOF_KEY).await.unwrap());
    }

    #[tokio::test]
    async fn process_exit_during_local_publication_does_not_poison_transfer_recovery() {
        for crash_key in [PROOF_KEY, "a.ts", ".private/checkpoint"] {
            let (temp, transfer) = fixture().await;
            let manifest = transfer.inspect().await.unwrap();
            if crash_key != PROOF_KEY {
                transfer
                    .claim(&transfer.proof("token", &manifest).unwrap())
                    .await
                    .unwrap();
            }
            let root = temp.path().canonicalize().unwrap();
            let target_root = root.join("target");
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "local_publication::tests::exit_during_publication",
                    "--nocapture",
                ])
                .env("BSR_TEST_PUBLICATION_ROOT", &target_root)
                .env("BSR_TEST_PUBLICATION_KEY", crash_key)
                .status()
                .unwrap();
            assert_eq!(child.code(), Some(73));
            assert!(!transfer.target.exists(crash_key).await.unwrap());
            let orphan_count = std::fs::read_dir(&target_root)
                .unwrap()
                .filter(|entry| {
                    entry
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".recording-transfer-object-")
                })
                .count();
            assert_eq!(orphan_count, 1, "the subprocess left real crash debris");
            drop(transfer);

            // Reopen both endpoints and deserialize the journal as on restart.
            let manifest: ObjectTransferManifest =
                serde_json::from_slice(&serde_json::to_vec(&manifest).unwrap()).unwrap();
            let resumed = ObjectTransfer::new(
                RecordingStore::local(root.join("source"))
                    .scope("bilibili/room/live")
                    .unwrap(),
                RecordingStore::local(target_root)
                    .scope("bilibili/room/live")
                    .unwrap(),
                "bilibili/room/live".into(),
                "a".into(),
                "b".into(),
            )
            .unwrap();
            resumed.copy("token", &manifest).await.unwrap();
            resumed
                .verify_destination("token", &manifest)
                .await
                .unwrap();
            resumed.cleanup_source("token", &manifest).await.unwrap();
            resumed.finish("token", &manifest).await.unwrap();
            assert_eq!(resumed.target.read("a.ts").await.unwrap(), b"segment");
            assert_eq!(
                resumed.target.read(".private/checkpoint").await.unwrap(),
                b"resume"
            );
            assert!(keys(&resumed.source).await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn unowned_targets_and_changed_payloads_never_get_overwritten_or_deleted() {
        let (_temp, transfer) = fixture().await;
        transfer.target.write("a.ts", b"segment").await.unwrap();
        assert!(
            transfer.inspect().await.is_err(),
            "identical unowned objects are still conflicts"
        );
        transfer.target.remove("a.ts").await.unwrap();
        let manifest = transfer.inspect().await.unwrap();
        transfer.copy("owner", &manifest).await.unwrap();
        assert!(transfer.copy("other-token", &manifest).await.is_err());
        transfer.target.write("b.ts", b"corrupted").await.unwrap();
        assert!(transfer.cleanup_source("owner", &manifest).await.is_err());
        assert_eq!(transfer.source.read("a.ts").await.unwrap(), b"segment");
        assert!(transfer.copy("owner", &manifest).await.is_err());
        assert_eq!(transfer.target.read("b.ts").await.unwrap(), b"corrupted");
    }

    #[tokio::test]
    async fn changed_source_and_extra_keys_stop_the_transfer_without_source_loss() {
        let (_temp, transfer) = fixture().await;
        let manifest = transfer.inspect().await.unwrap();
        transfer.source.write("b.ts", b"new bytes").await.unwrap();
        assert!(transfer.copy("owner", &manifest).await.is_err());
        assert_eq!(transfer.source.read("b.ts").await.unwrap(), b"new bytes");

        let (_temp, transfer) = fixture().await;
        let manifest = transfer.inspect().await.unwrap();
        transfer.copy("owner", &manifest).await.unwrap();
        transfer.source.write("new-file", b"unknown").await.unwrap();
        assert!(transfer.cleanup_source("owner", &manifest).await.is_err());
        assert!(transfer.source.exists("a.ts").await.unwrap());

        let (_temp, transfer) = fixture().await;
        let manifest = transfer.inspect().await.unwrap();
        transfer
            .claim(&transfer.proof("owner", &manifest).unwrap())
            .await
            .unwrap();
        transfer
            .target
            .write(".tmp-user-file", b"unowned")
            .await
            .unwrap();
        assert!(transfer.copy("owner", &manifest).await.is_err());
        assert_eq!(
            transfer.target.read(".tmp-user-file").await.unwrap(),
            b"unowned"
        );
        assert!(transfer.source.exists("a.ts").await.unwrap());
    }

    #[tokio::test]
    async fn manifests_are_endpoint_scoped_and_missing_sources_are_not_empty_successes() {
        let (_temp, transfer) = fixture().await;
        let mut manifest = transfer.inspect().await.unwrap();
        manifest.target_identity = "different".into();
        assert!(transfer.copy("owner", &manifest).await.is_err());
        transfer.source.remove_all().await.unwrap();
        assert!(transfer.inspect().await.is_err());
    }
}
