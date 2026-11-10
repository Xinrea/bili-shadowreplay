//! Journal-driven local archive transfer. Call on a blocking thread, with archive
//! writers stopped and application-controlled roots (not a hostile-process
//! sandbox). Every operation rechecks paths, but filesystem check/use races with
//! unrelated writers cannot be prevented by a portable path-based API.
//!
//! Persist the manifest and a fresh UUID in the application journal BEFORE copy.
//! After copy, commit the new storage location; then cleanup_source; then persist
//! a terminal finalizing phase. Call finish, then delete the terminal journal.
//! Finalizing retries must never copy or cleanup_source again. Crashes may leave
//! harmless unreferenced staging files, but never authorize
//! deleting an unverified source. Do not manually reuse a transfer token.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod rename;
pub use rename::{is_cross_device_error, RenameManifest};

const PREFIX: &str = ".recording-transfer-";
const BUFFER: usize = 64 * 1024;
const MAX_PROOF_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Identity {
    device: u64,
    inode: u64,
    created: Option<(u64, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileIdentity {
    object: Identity,
    modified: (u64, u32),
    // Unlike mtime, ctime cannot be restored with ordinary timestamp APIs.
    changed: Option<(i64, i64)>,
}

/// A relative file path, byte length, and SHA-256 digest. Source identity is
/// serialized as well, and must survive journal round trips without rebuilding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferFile {
    pub path: String,
    pub size: u64,
    pub sha256: [u8; 32],
    identity: FileIdentity,
}

/// Empty directories are included. The empty path denotes the namespace root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferDirectory {
    pub path: String,
    identity: Identity,
}

/// A snapshot of ALL files, including dotfiles, private log chunks and checkpoints.
/// Do not edit a persisted manifest; obtain a new one for a new transfer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferManifest {
    pub files: Vec<TransferFile>,
    pub directories: Vec<TransferDirectory>,
    namespace: String,
    source_root: PathBuf,
    source_identity: Identity,
    bytes: u64,
}

impl TransferManifest {
    pub fn total_bytes(&self) -> u64 {
        self.bytes
    }
}

/// Explicit local-to-local transfer, separate from arbitrary backend operations.
pub struct LocalTransfer {
    source: PathBuf,
    target: PathBuf,
    source_identity: Identity,
    target_identity: Identity,
}

#[derive(Serialize, Deserialize)]
struct Proof {
    version: u32,
    token: String,
    target_root: PathBuf,
    target_identity: Identity,
    manifest: TransferManifest,
    stage: String,
    stage_identity: Identity,
    archive_identity: Identity,
    scratch_identity: Identity,
}

struct Owned {
    // Held throughout mutation, serializing cooperating transfer instances.
    _lock: File,
    proof: Proof,
    published: bool,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn conflict(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, message)
}

fn metadata(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(invalid("symlinks are not transferable")),
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn required_metadata(path: &Path) -> io::Result<Metadata> {
    metadata(path)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "transfer path missing"))
}

/// Check every component, including ancestors above the configured root.
fn checked_path(path: &Path) -> io::Result<()> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for part in absolute.components() {
        current.push(part);
        if let Some(m) = metadata(&current)? {
            if !m.is_dir() {
                return Err(invalid("transfer ancestor is not a directory"));
            }
        }
    }
    Ok(())
}

fn stamp(time: std::time::SystemTime) -> io::Result<(u64, u32)> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid("timestamp before epoch"))?;
    Ok((duration.as_secs(), duration.subsec_nanos()))
}

#[cfg(unix)]
fn identity(_path: &Path, m: &Metadata) -> io::Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    Ok(Identity {
        device: m.dev(),
        inode: m.ino(),
        created: m.created().ok().map(stamp).transpose()?,
    })
}

#[cfg(windows)]
fn identity(path: &Path, _m: &Metadata) -> io::Result<Identity> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT,
    };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Identity {
        device: info.dwVolumeSerialNumber as u64,
        inode: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        created: Some((
            ((info.ftCreationTime.dwHighDateTime as u64) << 32)
                | info.ftCreationTime.dwLowDateTime as u64,
            0,
        )),
    })
}

#[cfg(not(any(unix, windows)))]
fn identity(_path: &Path, _m: &Metadata) -> io::Result<Identity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stable filesystem identity unavailable",
    ))
}

fn file_identity(path: &Path, m: &Metadata) -> io::Result<FileIdentity> {
    #[cfg(unix)]
    let changed = {
        use std::os::unix::fs::MetadataExt;
        Some((m.ctime(), m.ctime_nsec()))
    };
    #[cfg(not(unix))]
    let changed = None;
    Ok(FileIdentity {
        object: identity(path, m)?,
        modified: stamp(m.modified()?)?,
        changed,
    })
}

fn dir_identity(path: &Path) -> io::Result<Identity> {
    let m = required_metadata(path)?;
    if !m.is_dir() {
        return Err(invalid("expected transfer directory"));
    }
    identity(path, &m)
}

/// Validate a local endpoint and return its durable identity plus free space.
/// Object transfers pin this identity in their journal across retries.
pub fn local_transfer_root(root: &Path) -> io::Result<(String, u64)> {
    checked_path(root)?;
    let root = root.canonicalize()?;
    let identity = dir_identity(&root)?;
    Ok((
        serde_json::to_string(&(root.clone(), identity)).map_err(io::Error::other)?,
        fs2::available_space(root)?,
    ))
}

pub(crate) fn local_file_revision(path: &Path) -> io::Result<String> {
    checked_path(
        path.parent()
            .ok_or_else(|| invalid("object has no parent"))?,
    )?;
    let metadata = required_metadata(path)?;
    if !metadata.is_file() {
        return Err(invalid("transfer object is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(invalid("hardlinked objects are not transferable"));
        }
    }
    serde_json::to_string(&file_identity(path, &metadata)?).map_err(io::Error::other)
}

pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    // Repeat ancestor barriers on retries, including mkdirs acknowledged before
    // a crash. The namespace may be several levels below an existing root.
    checked_path(path)?;
    for parent in path.ancestors() {
        sync_dir(parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(test)]
    SYNC_FAILURE.with(|failure| {
        if failure.borrow().as_deref() == Some(path) {
            Err(io::Error::other("injected directory sync failure"))
        } else {
            Ok(())
        }
    })?;
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> io::Result<()> {
    // Windows publication uses MOVEFILE_WRITE_THROUGH; file contents are flushed
    // separately. Opening a directory for FlushFileBuffers is not supported.
    Ok(())
}

/// Atomic publication must NEVER replace even an empty directory.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(from.as_os_str().as_bytes()).map_err(|_| invalid("NUL path"))?;
    let to = CString::new(to.as_os_str().as_bytes()).map_err(|_| invalid("NUL path"))?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if result != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    windows
)))]
fn rename_new(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no atomic no-replace rename",
    ))
}

fn validate_namespace(namespace: &str) -> io::Result<()> {
    crate::validate_key(namespace)?;
    if namespace.split('/').any(|part| part.starts_with(PREFIX)) {
        return Err(invalid("reserved transfer namespace"));
    }
    Ok(())
}

fn validate_token(token: &str) -> io::Result<()> {
    if token.len() != 36
        || !token.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected UUID transfer token",
        ));
    }
    Ok(())
}

fn key(path: &Path) -> io::Result<String> {
    let mut result = Vec::new();
    for component in path.components() {
        if let Component::Normal(name) = component {
            let name = name
                .to_str()
                .ok_or_else(|| invalid("non-UTF8 archive name"))?;
            crate::validate_component(name)?;
            result.push(name);
        } else {
            return Err(invalid("invalid relative archive path"));
        }
    }
    Ok(result.join("/"))
}

/// Stream exactly the recorded size, never reading a growing file indefinitely.
fn hash_file(path: &Path, size: u64) -> io::Result<([u8; 32], FileIdentity)> {
    #[cfg(test)]
    HASH_FAILURE.with(|failure| {
        if failure.get() {
            Err(io::Error::other("injected payload hash failure"))
        } else {
            Ok(())
        }
    })?;
    let before = required_metadata(path)?;
    if !before.is_file() || before.len() != size {
        return Err(invalid("archive file type or size changed"));
    }
    let id = file_identity(path, &before)?;
    let mut file = File::open(path)?;
    if file_identity(path, &file.metadata()?)? != id {
        return Err(invalid("archive file replaced while opening"));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; BUFFER];
    let mut remaining = size;
    while remaining != 0 {
        let limit = remaining.min(BUFFER as u64) as usize;
        file.read_exact(&mut buffer[..limit])?;
        hash.update(&buffer[..limit]);
        remaining -= limit as u64;
    }
    if file.read(&mut buffer[..1])? != 0
        || file_identity(path, &file.metadata()?)? != id
        || file_identity(path, &required_metadata(path)?)? != id
    {
        return Err(invalid("archive file changed while hashing"));
    }
    Ok((hash.finalize().into(), id))
}

impl LocalTransfer {
    pub fn new(source_root: &Path, target_root: &Path) -> io::Result<Self> {
        checked_path(source_root)?;
        checked_path(target_root)?;
        let source = source_root.canonicalize()?;
        let target = target_root.canonicalize()?;
        let source_identity = dir_identity(&source)?;
        let target_identity = dir_identity(&target)?;
        if source.starts_with(&target)
            || target.starts_with(&source)
            || source_identity == target_identity
        {
            return Err(invalid("transfer roots overlap"));
        }
        // Also detect aliases exposed as a mount of either root in the other's
        // ancestry. Canonical paths alone do not detect such aliases.
        for (path, other) in [(&source, &target_identity), (&target, &source_identity)] {
            for ancestor in path.ancestors() {
                if dir_identity(ancestor)? == *other {
                    return Err(invalid("transfer roots alias an ancestor"));
                }
            }
        }
        Ok(Self {
            source,
            target,
            source_identity,
            target_identity,
        })
    }

    fn roots(&self) -> io::Result<()> {
        checked_path(&self.source)?;
        checked_path(&self.target)?;
        if dir_identity(&self.source)? != self.source_identity
            || dir_identity(&self.target)? != self.target_identity
        {
            return Err(invalid("transfer root replaced"));
        }
        Ok(())
    }

    pub fn available_bytes(&self) -> io::Result<u64> {
        self.roots()?;
        fs2::available_space(&self.target)
    }

    /// Read-only capacity on the destination namespace's filesystem, including
    /// nested mounts. Missing parents use their nearest existing ancestor.
    pub fn copy_available_bytes(&self, namespace: &str) -> io::Result<u64> {
        fs2::available_space(self.destination_ancestor(namespace)?)
    }

    fn destination_ancestor(&self, namespace: &str) -> io::Result<PathBuf> {
        self.roots()?;
        validate_namespace(namespace)?;
        let destination = self.target.join(namespace);
        let parent = destination.parent().unwrap();
        checked_path(parent)?;
        for ancestor in parent.ancestors() {
            if metadata(ancestor)?.is_some() {
                dir_identity(ancestor)?;
                return Ok(ancestor.to_owned());
            }
        }
        Err(invalid("destination root missing"))
    }

    fn copy_stage_parent(&self, namespace: &str) -> io::Result<PathBuf> {
        self.destination_ancestor(namespace)?;
        // Equal device IDs do not imply that rename can cross mount boundaries
        // (notably Linux bind mounts). Always stage beside the destination.
        Ok(self.target.join(namespace).parent().unwrap().to_owned())
    }

    /// One validator for every use of a persisted staging path. Staging must
    /// be a namespace-parent sibling, on the same mount as its destination.
    fn stage_path(&self, proof: &Proof) -> io::Result<PathBuf> {
        crate::validate_key(&proof.stage)?;
        let relative = Path::new(&proof.stage);
        let name = relative.file_name().and_then(|name| name.to_str());
        if !name.is_some_and(|name| name.starts_with(&format!("{PREFIX}stage-"))) {
            return Err(invalid("invalid staging path"));
        }
        let stage = self.target.join(relative);
        if proof.version != 2
            || stage.parent() != self.target.join(&proof.manifest.namespace).parent()
        {
            return Err(invalid("invalid staging path or proof version"));
        }
        checked_path(stage.parent().unwrap())?;
        Ok(stage)
    }

    pub fn inspect(&self, namespace: &str) -> io::Result<TransferManifest> {
        self.roots()?;
        validate_namespace(namespace)?;
        let root = self.source.join(namespace);
        checked_path(&root)?;
        let mut manifest = TransferManifest {
            files: Vec::new(),
            directories: Vec::new(),
            namespace: namespace.into(),
            source_root: self.source.clone(),
            source_identity: self.source_identity.clone(),
            bytes: 0,
        };
        // Iterative traversal avoids overflowing the stack for deep archives.
        let mut pending = vec![PathBuf::new()];
        let mut seen_directories = BTreeSet::new();
        while let Some(relative) = pending.pop() {
            let directory = root.join(&relative);
            let identity = dir_identity(&directory)?;
            if !seen_directories.insert((identity.device, identity.inode)) {
                return Err(invalid("archive directory aliases another directory"));
            }
            manifest.directories.push(TransferDirectory {
                path: key(&relative)?,
                identity,
            });
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                let relative = relative.join(entry.file_name());
                let name = key(&relative)?;
                let m = required_metadata(&entry.path())?;
                if m.is_dir() {
                    pending.push(relative);
                } else if m.is_file() {
                    let (sha256, identity) = hash_file(&entry.path(), m.len())?;
                    manifest.bytes = manifest
                        .bytes
                        .checked_add(m.len())
                        .ok_or_else(|| invalid("archive size overflow"))?;
                    manifest.files.push(TransferFile {
                        path: name,
                        size: m.len(),
                        sha256,
                        identity,
                    });
                } else {
                    return Err(invalid("unsupported archive object"));
                }
            }
        }
        manifest.files.sort_by(|a, b| a.path.cmp(&b.path));
        manifest.directories.sort_by(|a, b| a.path.cmp(&b.path));
        self.validate(namespace, &manifest)?;
        self.verify_tree(&root, &manifest, true, false)?;
        Ok(manifest)
    }

    fn validate(&self, namespace: &str, manifest: &TransferManifest) -> io::Result<()> {
        self.roots()?;
        validate_namespace(namespace)?;
        if manifest.namespace != namespace
            || manifest.source_root != self.source
            || manifest.source_identity != self.source_identity
        {
            return Err(invalid("manifest belongs to another source"));
        }
        let mut paths = BTreeMap::new();
        let mut objects = BTreeSet::new();
        let mut bytes = 0u64;
        for directory in &manifest.directories {
            if !directory.path.is_empty() {
                crate::validate_key(&directory.path)?;
            }
            if paths.insert(directory.path.as_str(), true).is_some() {
                return Err(invalid("duplicate manifest path"));
            }
        }
        for file in &manifest.files {
            crate::validate_key(&file.path)?;
            if !objects.insert((file.identity.object.device, file.identity.object.inode)) {
                // Unlink changes ctime for every remaining hardlink. Reject
                // these archives rather than weaken cleanup identity checks.
                return Err(invalid("hardlinked archive entries are unsupported"));
            }
            if paths.insert(file.path.as_str(), false).is_some() {
                return Err(invalid("duplicate manifest path"));
            }
            bytes = bytes
                .checked_add(file.size)
                .ok_or_else(|| invalid("manifest size overflow"))?;
        }
        if paths.get("") != Some(&true) || bytes != manifest.bytes {
            return Err(invalid("invalid manifest root or total"));
        }
        for path in paths.keys().filter(|path| !path.is_empty()) {
            let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
            if paths.get(parent) != Some(&true) {
                return Err(invalid("manifest parent missing"));
            }
        }
        checked_path(self.source.join(namespace).parent().unwrap())?;
        checked_path(self.target.join(namespace).parent().unwrap())?;
        Ok(())
    }

    fn proof_path(&self, namespace: &str) -> PathBuf {
        self.target.join(format!(
            "{PREFIX}{:x}.json",
            Sha256::digest(namespace.as_bytes())
        ))
    }

    /// Read-only preflight. An unowned namespace (even empty or byte-identical)
    /// is a conflict. No directories, lock files or markers are created.
    pub fn check_destination(
        &self,
        namespace: &str,
        token: &str,
        manifest: &TransferManifest,
    ) -> io::Result<()> {
        self.validate(namespace, manifest)?;
        validate_token(token)?;
        self.owned(namespace, token, Some(manifest))?;
        Ok(())
    }

    /// Conservative requirement for a partial stage; a verified, published copy
    /// needs no additional payload space on retry.
    pub fn required_bytes(
        &self,
        namespace: &str,
        token: &str,
        manifest: &TransferManifest,
    ) -> io::Result<u64> {
        self.validate(namespace, manifest)?;
        validate_token(token)?;
        Ok(match self.owned(namespace, token, Some(manifest))? {
            Some(owned) if owned.published => 0,
            _ => manifest.total_bytes(),
        })
    }

    fn owned(
        &self,
        namespace: &str,
        token: &str,
        manifest: Option<&TransferManifest>,
    ) -> io::Result<Option<Owned>> {
        let path = self.proof_path(namespace);
        let Some(m) = metadata(&path)? else {
            if metadata(&self.target.join(namespace))?.is_some() {
                return Err(conflict("destination exists without transfer ownership"));
            }
            return Ok(None);
        };
        if !m.is_file() || m.len() > MAX_PROOF_BYTES {
            return Err(conflict("invalid ownership proof"));
        }
        let lock = OpenOptions::new().read(true).write(true).open(&path)?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        if file_identity(&path, &lock.metadata()?)?
            != file_identity(&path, &required_metadata(&path)?)?
        {
            return Err(conflict("ownership proof replaced"));
        }
        let proof: Proof =
            serde_json::from_reader(BufReader::new((&lock).take(MAX_PROOF_BYTES + 1)))
                .map_err(|e| invalid(&format!("invalid ownership proof: {e}")))?;
        if proof.version != 2
            || proof.token != token
            || proof.target_root != self.target
            || proof.target_identity != self.target_identity
            || proof.manifest.namespace != namespace
            || manifest.is_some_and(|manifest| *manifest != proof.manifest)
        {
            return Err(conflict("destination belongs to another transfer"));
        }
        self.validate(namespace, &proof.manifest)?;
        let stage = self.stage_path(&proof)?;
        if dir_identity(&stage)? != proof.stage_identity
            || dir_identity(&stage.join("scratch"))? != proof.scratch_identity
        {
            return Err(conflict("owned staging directory replaced"));
        }
        let destination = self.target.join(namespace);
        let published = metadata(&destination)?.is_some();
        let archive = if published {
            destination
        } else {
            stage.join("archive")
        };
        if dir_identity(&archive)? != proof.archive_identity {
            return Err(conflict("archive directory ownership changed"));
        }
        if published && metadata(&stage.join("archive"))?.is_some() {
            return Err(conflict("both staging and published archive exist"));
        }
        self.verify_tree(&archive, &proof.manifest, false, !published)?;
        Ok(Some(Owned {
            _lock: lock,
            proof,
            published,
        }))
    }

    fn create_owned(
        &self,
        namespace: &str,
        token: &str,
        manifest: &TransferManifest,
    ) -> io::Result<Owned> {
        let parent = self.copy_stage_parent(namespace)?;
        self.create_parents(&parent)?;
        let stage = tempfile::Builder::new()
            .prefix(&format!("{PREFIX}stage-"))
            .tempdir_in(&parent)?;
        fs::create_dir(stage.path().join("archive"))?;
        fs::create_dir(stage.path().join("scratch"))?;
        let proof = Proof {
            version: 2,
            token: token.into(),
            target_root: self.target.clone(),
            target_identity: self.target_identity.clone(),
            manifest: manifest.clone(),
            stage: key(stage.path().strip_prefix(&self.target).unwrap())?,
            stage_identity: dir_identity(stage.path())?,
            archive_identity: dir_identity(&stage.path().join("archive"))?,
            scratch_identity: dir_identity(&stage.path().join("scratch"))?,
        };
        sync_dir(&stage.path().join("archive"))?;
        sync_dir(&stage.path().join("scratch"))?;
        sync_dir(stage.path())?;
        // Keep BEFORE publishing proof: errors after proof publication must not
        // remove the directory to which a durable journal/proof refers.
        let _stage = stage.keep();
        for ancestor in parent.ancestors() {
            sync_dir(ancestor)?;
            if ancestor == self.target {
                break;
            }
        }
        let mut file = tempfile::NamedTempFile::new_in(&self.target)?;
        serde_json::to_writer(file.as_file_mut(), &proof).map_err(io::Error::other)?;
        if file.as_file().metadata()?.len() > MAX_PROOF_BYTES {
            return Err(invalid(
                "transfer manifest exceeds 64 MiB ownership proof limit",
            ));
        }
        file.as_file().sync_all()?;
        fs2::FileExt::try_lock_exclusive(file.as_file())?;
        let lock = file
            .persist_noclobber(self.proof_path(namespace))
            .map_err(|e| e.error)?;
        sync_dir(&self.target)?;
        Ok(Owned {
            _lock: lock,
            proof,
            published: false,
        })
    }

    /// Stream into private staging, sync and verify all bytes, then atomically
    /// publish without replacement. Failures keep source intact and are retryable.
    pub fn copy(
        &self,
        namespace: &str,
        token: &str,
        manifest: &TransferManifest,
    ) -> io::Result<()> {
        self.validate(namespace, manifest)?;
        validate_token(token)?;
        let existing = self.owned(namespace, token, Some(manifest))?;
        if let Some(owned) = existing.as_ref().filter(|owned| owned.published) {
            return self.sync_published(namespace, owned);
        }
        let source = self.source.join(namespace);
        self.verify_tree(&source, manifest, true, false)?;
        let owned = match existing {
            Some(owned) => owned,
            None => self.create_owned(namespace, token, manifest)?,
        };
        let stage = self.stage_path(&owned.proof)?;
        let archive = stage.join("archive");
        let mut directories: Vec<_> = manifest.directories.iter().collect();
        directories.sort_by_key(|d| d.path.split('/').count());
        for directory in &directories {
            let path = archive.join(&directory.path);
            if metadata(&path)?.is_none() {
                fs::create_dir(&path)?;
                sync_dir(path.parent().unwrap())?;
            } else {
                dir_identity(&path)?;
            }
        }
        for entry in &manifest.files {
            let destination = archive.join(&entry.path);
            if metadata(&destination)?.is_some() {
                verify_file(&destination, entry, false)?;
                continue;
            }
            let original = source.join(&entry.path);
            verify_file(&original, entry, true)?;
            let mut input = File::open(&original)?;
            if file_identity(&original, &input.metadata()?)? != entry.identity {
                return Err(invalid("source replaced before copy"));
            }
            let mut output = tempfile::NamedTempFile::new_in(stage.join("scratch"))?;
            let mut buffer = [0; BUFFER];
            let mut hash = Sha256::new();
            let mut remaining = entry.size;
            while remaining != 0 {
                let count = remaining.min(BUFFER as u64) as usize;
                if fs2::available_space(&stage)? < count as u64 {
                    return Err(io::Error::other("insufficient destination space"));
                }
                input.read_exact(&mut buffer[..count])?;
                output.write_all(&buffer[..count])?;
                #[cfg(test)]
                COPY_FAILURE.with(|threshold| {
                    if threshold
                        .get()
                        .is_some_and(|threshold| entry.size - remaining + count as u64 >= threshold)
                    {
                        Err(io::Error::other("injected staged write failure"))
                    } else {
                        Ok(())
                    }
                })?;
                hash.update(&buffer[..count]);
                remaining -= count as u64;
            }
            let digest: [u8; 32] = hash.finalize().into();
            if input.read(&mut buffer[..1])? != 0
                || digest != entry.sha256
                || file_identity(&original, &input.metadata()?)? != entry.identity
                || file_identity(&original, &required_metadata(&original)?)? != entry.identity
            {
                return Err(invalid("source changed while copying"));
            }
            output.as_file().sync_all()?;
            output
                .persist_noclobber(&destination)
                .map_err(|e| e.error)?;
            sync_dir(destination.parent().unwrap())?;
            verify_file(&destination, entry, false)?;
        }
        self.verify_tree(&source, manifest, true, false)?;
        self.verify_tree(&archive, manifest, false, false)?;
        for directory in directories.into_iter().rev() {
            sync_dir(&archive.join(&directory.path))?;
        }
        let destination = self.target.join(namespace);
        self.create_parents(destination.parent().unwrap())?;
        // No replacing rename, including races with a newly-created empty dir.
        rename_new(&archive, &destination)?;
        self.sync_published(namespace, &owned)
    }

    /// Re-establish durability even after a previous rename/mkdir succeeded but
    /// its sync failed. Hash verification alone proves visibility, not durability.
    /// Also flush currently verified files: a byte-identical replacement by an
    /// external writer must not become the only unsynced copy after cleanup.
    fn sync_published(&self, namespace: &str, owned: &Owned) -> io::Result<()> {
        let destination = self.target.join(namespace);
        for file in &owned.proof.manifest.files {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(destination.join(&file.path))?
                .sync_all()?;
        }
        let mut directories: Vec<_> = owned.proof.manifest.directories.iter().collect();
        directories.sort_by_key(|d| std::cmp::Reverse(d.path.len()));
        for directory in directories {
            sync_dir(&destination.join(&directory.path))?;
        }
        owned._lock.sync_all()?;
        sync_dir(&self.stage_path(&owned.proof)?)?;
        for ancestor in destination.parent().unwrap().ancestors() {
            sync_dir(ancestor)?;
            if ancestor == self.target {
                break;
            }
        }
        Ok(())
    }

    fn create_parents(&self, parent: &Path) -> io::Result<()> {
        let relative = parent
            .strip_prefix(&self.target)
            .map_err(|_| invalid("invalid destination"))?;
        let mut current = self.target.clone();
        for component in relative.components() {
            current.push(component);
            match fs::create_dir(&current) {
                Ok(()) => sync_dir(current.parent().unwrap())?,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    dir_identity(&current)?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Verify the owned destination on EVERY invocation, then preflight all
    /// remaining source entries before deleting any. Missing entries are allowed
    /// for interrupted cleanup; new, modified or recreated entries are not.
    pub fn cleanup_source(
        &self,
        namespace: &str,
        token: &str,
        manifest: &TransferManifest,
    ) -> io::Result<()> {
        self.validate(namespace, manifest)?;
        validate_token(token)?;
        let owned = self
            .owned(namespace, token, Some(manifest))?
            .ok_or_else(|| conflict("cleanup requires published ownership proof"))?;
        if !owned.published {
            return Err(invalid("copy is not published"));
        }
        self.sync_published(namespace, &owned)?;
        let source = self.source.join(namespace);
        self.verify_tree(&source, manifest, true, true)?;
        for entry in &manifest.files {
            let path = source.join(&entry.path);
            checked_path(path.parent().unwrap())?;
            if metadata(&path)?.is_some() {
                verify_file(&path, entry, true)?;
                fs::remove_file(&path)?;
                sync_dir(path.parent().unwrap())?;
            }
        }
        let mut directories: Vec<_> = manifest.directories.iter().collect();
        // Root's empty path and a first-level directory have the same split
        // count; force root last, independent of serialized manifest order.
        directories.sort_by_key(|d| {
            std::cmp::Reverse(if d.path.is_empty() {
                0
            } else {
                d.path.split('/').count()
            })
        });
        for directory in directories {
            let path = source.join(&directory.path);
            checked_path(path.parent().unwrap())?;
            if metadata(&path)?.is_some() {
                if dir_identity(&path)? != directory.identity {
                    return Err(invalid("source directory replaced during cleanup"));
                }
                fs::remove_dir(&path)?; // Never recursively delete source.
                sync_dir(path.parent().unwrap())?;
            }
        }
        Ok(())
    }

    /// Release proof only after the app durably enters a terminal finalizing
    /// phase which will never repeat copy/source cleanup. Retain that journal
    /// and the archive reservation until this operation succeeds.
    /// Idempotent when no proof remains, including after an interrupted unlink.
    pub fn finish(&self, namespace: &str, token: &str) -> io::Result<()> {
        self.roots()?;
        validate_namespace(namespace)?;
        validate_token(token)?;
        if metadata(&self.proof_path(namespace))?.is_none() {
            // A prior finalization may have removed the proof and nested stage
            // before its parent sync failed. Repeat that barrier without needing
            // to recover a deleted proof or infer ownership of any directory.
            for ancestor in self.destination_ancestor(namespace)?.ancestors() {
                sync_dir(ancestor)?;
                if ancestor == self.target {
                    break;
                }
            }
            return Ok(());
        }
        let owned = self
            .owned(namespace, token, None)?
            .ok_or_else(|| invalid("ownership proof missing"))?;
        if !owned.published {
            return Err(invalid("cannot finish unpublished copy"));
        }
        if metadata(&self.source.join(namespace))?.is_some() {
            return Err(invalid("cannot finish before source cleanup"));
        }
        let stage = self.stage_path(&owned.proof)?;
        // Only scratch files created inside our private owned container may be
        // discarded. Refuse unknown directories/symlinks/entries.
        let scratch = stage.join("scratch");
        for entry in fs::read_dir(&stage)? {
            if entry?.file_name() != "scratch" {
                return Err(conflict("unknown staging entry"));
            }
        }
        for entry in fs::read_dir(&scratch)? {
            let entry = entry?;
            if !required_metadata(&entry.path())?.is_file() {
                return Err(conflict("unknown scratch entry"));
            }
        }
        // Remove proof first. Crash recovery cannot mistake a partially-removed
        // staging container for a still-live proof. Orphans are safe to retain.
        fs::remove_file(self.proof_path(namespace))?;
        sync_dir(&self.target)?;
        for entry in fs::read_dir(&scratch)? {
            fs::remove_file(entry?.path())?;
        }
        fs::remove_dir(&scratch)?;
        fs::remove_dir(&stage)?;
        sync_dir(stage.parent().unwrap())
    }

    fn verify_tree(
        &self,
        root: &Path,
        manifest: &TransferManifest,
        source: bool,
        partial: bool,
    ) -> io::Result<()> {
        checked_path(root.parent().unwrap())?;
        if metadata(root)?.is_none() {
            return if partial {
                Ok(())
            } else {
                Err(invalid("archive missing"))
            };
        }
        let directories: BTreeMap<_, _> = manifest
            .directories
            .iter()
            .map(|d| (d.path.as_str(), d))
            .collect();
        let files: BTreeMap<_, _> = manifest
            .files
            .iter()
            .map(|f| (f.path.as_str(), f))
            .collect();
        let mut found_dirs = 0;
        let mut found_files = 0;
        let mut pending = vec![PathBuf::new()];
        while let Some(relative) = pending.pop() {
            let name = key(&relative)?;
            let expected = directories
                .get(name.as_str())
                .ok_or_else(|| conflict("unlisted archive directory"))?;
            let directory = root.join(&relative);
            let id = dir_identity(&directory)?;
            if source && id != expected.identity {
                return Err(invalid("source directory replaced"));
            }
            found_dirs += 1;
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                let relative = relative.join(entry.file_name());
                let m = required_metadata(&entry.path())?;
                if m.is_dir() {
                    pending.push(relative);
                } else if m.is_file() {
                    let name = key(&relative)?;
                    let expected = files
                        .get(name.as_str())
                        .ok_or_else(|| conflict("unlisted archive file"))?;
                    verify_file(&entry.path(), expected, source)?;
                    found_files += 1;
                } else {
                    return Err(invalid("unsupported archive object"));
                }
            }
        }
        if !partial && (found_dirs != directories.len() || found_files != files.len()) {
            return Err(invalid("archive entries missing"));
        }
        Ok(())
    }
}

fn verify_file(path: &Path, expected: &TransferFile, source: bool) -> io::Result<()> {
    let (hash, identity) = hash_file(path, expected.size)?;
    if hash != expected.sha256 || (source && identity != expected.identity) {
        return Err(invalid("archive bytes or source identity changed"));
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    static SYNC_FAILURE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    static COPY_FAILURE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static HASH_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "24af58ca-c3d9-4b2a-9d44-81035b29ad00";
    pub(super) const NAMESPACE: &str = "platform/room/live";

    struct FaultGuard;
    impl Drop for FaultGuard {
        fn drop(&mut self) {
            SYNC_FAILURE.with(|failure| *failure.borrow_mut() = None);
            COPY_FAILURE.with(|failure| failure.set(None));
        }
    }

    pub(super) fn fixture() -> (tempfile::TempDir, LocalTransfer, TransferManifest) {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path().canonicalize().unwrap();
        let source = base.join("source");
        let target = base.join("target");
        fs::create_dir_all(source.join(NAMESPACE)).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(
            source.join(NAMESPACE).join("file"),
            vec![91; 2 * BUFFER + 13],
        )
        .unwrap();
        let transfer = LocalTransfer::new(&source, &target).unwrap();
        let manifest = transfer.inspect(NAMESPACE).unwrap();
        (temporary, transfer, manifest)
    }

    #[test]
    fn interrupted_stream_never_publishes_and_can_retry() {
        let (_temporary, transfer, manifest) = fixture();
        let fault = FaultGuard;
        COPY_FAILURE.with(|failure| failure.set(Some(BUFFER as u64)));
        let error = transfer.copy(NAMESPACE, TOKEN, &manifest).unwrap_err();
        assert!(error.to_string().contains("injected staged write failure"));
        assert!(!transfer.target.join(NAMESPACE).exists());
        let owned = transfer
            .owned(NAMESPACE, TOKEN, Some(&manifest))
            .unwrap()
            .unwrap();
        let stage = transfer.target.join(&owned.proof.stage);
        assert!(!stage.join("archive/file").exists());
        assert_eq!(fs::read_dir(stage.join("scratch")).unwrap().count(), 0);
        assert_eq!(
            fs::read(transfer.source.join(NAMESPACE).join("file")).unwrap(),
            vec![91; 2 * BUFFER + 13]
        );
        drop(owned);
        drop(fault);
        transfer.copy(NAMESPACE, TOKEN, &manifest).unwrap();
        transfer
            .cleanup_source(NAMESPACE, TOKEN, &manifest)
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn published_retry_and_cleanup_repeat_failed_publication_barrier() {
        let (_temporary, transfer, manifest) = fixture();
        // Establish durable staging before injecting the publication barrier.
        drop(transfer.create_owned(NAMESPACE, TOKEN, &manifest).unwrap());
        let fault = FaultGuard;
        SYNC_FAILURE
            .with(|failure| *failure.borrow_mut() = Some(transfer.target.join("platform/room")));
        assert!(transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
        assert!(transfer.target.join(NAMESPACE).is_dir()); // Rename succeeded.
        assert!(transfer.copy(NAMESPACE, TOKEN, &manifest).is_err()); // Must sync again.
        assert!(transfer
            .cleanup_source(NAMESPACE, TOKEN, &manifest)
            .is_err());
        assert!(transfer.source.join(NAMESPACE).join("file").is_file());
        drop(fault);
        transfer.copy(NAMESPACE, TOKEN, &manifest).unwrap();
        transfer
            .cleanup_source(NAMESPACE, TOKEN, &manifest)
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ancestor_mkdir_sync_failure_is_retried_before_source_deletion() {
        let (_temporary, transfer, manifest) = fixture();
        let fault = FaultGuard;
        SYNC_FAILURE.with(|failure| *failure.borrow_mut() = Some(transfer.target.join("platform")));
        assert!(transfer.copy(NAMESPACE, TOKEN, &manifest).is_err()); // mkdir(room) sync fails.
        assert!(!transfer.target.join(NAMESPACE).exists());
        assert!(transfer.copy(NAMESPACE, TOKEN, &manifest).is_err()); // Existing ancestors still sync.
        assert!(!transfer.target.join(NAMESPACE).exists()); // Staging is not durable yet.
        assert!(transfer
            .cleanup_source(NAMESPACE, TOKEN, &manifest)
            .is_err());
        assert!(transfer.source.join(NAMESPACE).join("file").is_file());
        drop(fault);
        transfer.copy(NAMESPACE, TOKEN, &manifest).unwrap();
        transfer
            .cleanup_source(NAMESPACE, TOKEN, &manifest)
            .unwrap();
    }

    #[test]
    fn no_replace_publication_preserves_a_racing_empty_destination() {
        let (_temporary, transfer, _manifest) = fixture();
        let from = transfer.target.join("from");
        let to = transfer.target.join("to");
        fs::create_dir(&from).unwrap();
        fs::create_dir(&to).unwrap();
        let original = dir_identity(&to).unwrap();
        assert!(rename_new(&from, &to).is_err());
        assert_eq!(dir_identity(&to).unwrap(), original);
        assert!(from.is_dir());
    }

    #[test]
    fn new_stage_is_adjacent_even_when_parent_and_root_share_device() {
        let (_temporary, transfer, manifest) = fixture();
        let owned = transfer.create_owned(NAMESPACE, TOKEN, &manifest).unwrap();
        let parent = transfer.target.join("platform/room");
        assert_eq!(
            dir_identity(&parent).unwrap().device,
            transfer.target_identity.device
        );
        assert_eq!(owned.proof.version, 2);
        assert_eq!(
            transfer.stage_path(&owned.proof).unwrap().parent(),
            Some(parent.as_path())
        );
    }

    #[test]
    fn adjacent_stage_resumes_partial_copy_across_restart() {
        let (_temporary, transfer, manifest) = fixture();
        let fault = FaultGuard;
        COPY_FAILURE.with(|failure| failure.set(Some(BUFFER as u64)));
        assert!(transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
        let owned = transfer
            .owned(NAMESPACE, TOKEN, Some(&manifest))
            .unwrap()
            .unwrap();
        assert_eq!(owned.proof.version, 2);
        let stage = transfer.stage_path(&owned.proof).unwrap();
        assert_eq!(
            stage.parent().unwrap(),
            transfer.target.join("platform/room")
        );
        assert!(!owned.published);
        drop(owned);
        drop(fault);

        // Recovery uses the exact owned stage, never a newly selected path.
        let restart = LocalTransfer::new(&transfer.source, &transfer.target).unwrap();
        restart
            .check_destination(NAMESPACE, TOKEN, &manifest)
            .unwrap();
        restart.copy(NAMESPACE, TOKEN, &manifest).unwrap();
        restart.copy(NAMESPACE, TOKEN, &manifest).unwrap();
        let owned = restart
            .owned(NAMESPACE, TOKEN, Some(&manifest))
            .unwrap()
            .unwrap();
        assert_eq!(restart.stage_path(&owned.proof).unwrap(), stage);
        drop(owned);
        restart.cleanup_source(NAMESPACE, TOKEN, &manifest).unwrap();
        restart.finish(NAMESPACE, TOKEN).unwrap();
        restart.finish(NAMESPACE, TOKEN).unwrap();
        assert!(!stage.exists());
        assert!(transfer.target.join(NAMESPACE).join("file").is_file());
        assert!(!transfer.proof_path(NAMESPACE).exists());
    }

    #[test]
    fn persisted_stage_paths_reject_traversal_other_parents_and_unknown_versions() {
        let (_temporary, transfer, manifest) = fixture();
        let mut owned = transfer.create_owned(NAMESPACE, TOKEN, &manifest).unwrap();
        let original = owned.proof.stage.clone();
        for version in [1, 2, 3] {
            owned.proof.version = version;
            for stage in [
                "/.recording-transfer-stage-absolute",
                "../.recording-transfer-stage-escape",
                "platform/room/../.recording-transfer-stage-escape",
                "platform//room/.recording-transfer-stage-empty",
                "platform/room/./.recording-transfer-stage-dot",
                "platform\\room\\.recording-transfer-stage-windows",
                "elsewhere/.recording-transfer-stage-foreign",
                "platform/room/live/.recording-transfer-stage-payload",
                "platform/room/unreserved",
            ] {
                owned.proof.stage = stage.into();
                assert!(
                    transfer.stage_path(&owned.proof).is_err(),
                    "v{version}: {stage}"
                );
            }
            owned.proof.stage = original.clone();
            assert_eq!(transfer.stage_path(&owned.proof).is_ok(), version == 2);
        }
    }

    #[cfg(unix)]
    #[test]
    fn nested_stage_rejects_symlink_ancestor_and_replaced_stage() {
        use std::os::unix::fs::symlink;
        for replace_stage in [false, true] {
            let (_temporary, transfer, manifest) = fixture();
            let owned = transfer.create_owned(NAMESPACE, TOKEN, &manifest).unwrap();
            let stage = transfer.stage_path(&owned.proof).unwrap();
            drop(owned);
            let path = if replace_stage {
                stage.clone()
            } else {
                transfer.target.join("platform")
            };
            let original = transfer.target.join("original-kept");
            fs::rename(&path, &original).unwrap();
            if replace_stage {
                fs::create_dir(&path).unwrap();
            } else {
                symlink(&original, &path).unwrap();
                assert!(transfer.copy_available_bytes(NAMESPACE).is_err());
            }
            assert!(transfer
                .check_destination(NAMESPACE, TOKEN, &manifest)
                .is_err());
            assert!(transfer.copy(NAMESPACE, TOKEN, &manifest).is_err());
            assert!(transfer
                .cleanup_source(NAMESPACE, TOKEN, &manifest)
                .is_err());
            assert!(transfer.finish(NAMESPACE, TOKEN).is_err());
            assert!(original.is_dir());
            assert!(transfer.source.join(NAMESPACE).join("file").is_file());
        }
    }

    #[test]
    fn capacity_preflight_is_read_only_and_uses_nearest_existing_parent() {
        let (_temporary, transfer, _) = fixture();
        assert_eq!(
            transfer.destination_ancestor(NAMESPACE).unwrap(),
            transfer.target
        );
        assert!(transfer.copy_available_bytes(NAMESPACE).unwrap() > 0);
        assert_eq!(fs::read_dir(&transfer.target).unwrap().count(), 0);
        fs::create_dir(transfer.target.join("platform")).unwrap();
        assert_eq!(
            transfer.destination_ancestor(NAMESPACE).unwrap(),
            transfer.target.join("platform")
        );
        assert!(transfer.copy_available_bytes(NAMESPACE).unwrap() > 0);
        assert!(!transfer.target.join("platform/room").exists());
        fs::write(transfer.target.join("platform/room"), b"not directory").unwrap();
        assert!(transfer.copy_available_bytes(NAMESPACE).is_err());
        for namespace in [
            "",
            "../escape",
            "/absolute",
            "a//b",
            ".recording-transfer-private",
        ] {
            assert!(transfer.copy_available_bytes(namespace).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn nested_stage_finalization_retries_parent_sync_after_proof_removal() {
        let (_temporary, transfer, manifest) = fixture();
        let fault = FaultGuard;
        transfer.copy(NAMESPACE, TOKEN, &manifest).unwrap();
        transfer
            .cleanup_source(NAMESPACE, TOKEN, &manifest)
            .unwrap();
        SYNC_FAILURE.with(|failure| {
            *failure.borrow_mut() = Some(transfer.target.join("platform/room"));
        });
        assert!(transfer.finish(NAMESPACE, TOKEN).is_err());
        assert!(!transfer.proof_path(NAMESPACE).exists());
        assert!(transfer.finish(NAMESPACE, TOKEN).is_err());
        drop(fault);
        transfer.finish(NAMESPACE, TOKEN).unwrap();
        assert_eq!(
            fs::read_dir(transfer.target.join("platform/room"))
                .unwrap()
                .count(),
            1
        );
    }
}
