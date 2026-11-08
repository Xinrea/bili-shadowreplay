//! Files needed only by this application session, never recording persistence.
//!
//! Every clone is a lease: the randomly named OS temporary directory is removed
//! when the last owner drops. Keep a lease in application state and in services
//! using it, so uploads survive a request and queued work can still read them.
use std::{io, path::Path, sync::Arc};

/// Public compatibility URL namespaces. Processing scratch is never served.
pub fn public_namespace(namespace: &str) -> bool {
    matches!(namespace, "uploads" | "preview_audio" | "exports")
}

#[derive(Clone, Debug)]
pub struct TempStorage {
    directory: Arc<tempfile::TempDir>,
}

impl TempStorage {
    pub fn new() -> io::Result<Self> {
        Ok(Self::from_directory(
            tempfile::Builder::new()
                .prefix("bili-shadowreplay-")
                .tempdir()?,
        ))
    }

    /// Take ownership of a temporary directory, also allowing isolated tests.
    pub fn from_directory(directory: tempfile::TempDir) -> Self {
        Self {
            directory: Arc::new(directory),
        }
    }

    pub fn root(&self) -> &Path {
        self.directory.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_are_unique_and_last_lease_removes_only_owned_files() {
        let recordings = tempfile::tempdir().unwrap();
        let recording = recordings.path().join("recording.ts");
        std::fs::write(&recording, "keep").unwrap();
        let first = TempStorage::new().unwrap();
        let second = TempStorage::new().unwrap();
        assert_ne!(first.root(), second.root());
        assert!(!first.root().starts_with(recordings.path()));
        let root = first.root().to_path_buf();
        std::fs::write(root.join("upload.mp4"), "upload").unwrap();
        let lease = first.clone();
        drop(first);
        assert!(root.join("upload.mp4").is_file());
        drop(lease);
        assert!(!root.exists());
        assert!(second.root().is_dir());
        assert_eq!(std::fs::read_to_string(recording).unwrap(), "keep");
    }
}
