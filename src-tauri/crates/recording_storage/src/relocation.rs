//! Explicit relocation of the default local backend. This is not an operation
//! on an arbitrary object store; named/remote backends are never selected here.
use std::{io, path::Path};

use crate::{validate_key, LocalBackend};

impl LocalBackend {
    /// Move owned top-level namespaces, preserving unrelated entries. A move
    /// across filesystems stages a complete copy before deleting its source.
    /// This is not a transaction across namespaces; on error completed moves
    /// remain at the destination and must be reported by the configuration UI.
    pub fn relocate_prefixes(&self, target: &Path, prefixes: &[&str]) -> io::Result<usize> {
        if target == self.root || target.starts_with(&self.root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination is inside source",
            ));
        }
        let mut sources = Vec::new();
        for prefix in prefixes {
            validate_key(prefix)?;
            if prefix.contains('/') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "expected a top-level namespace",
                ));
            }
            let source = self.root.join(prefix);
            let metadata = match std::fs::symlink_metadata(&source) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "recording namespace is not a directory",
                ));
            }
            let destination = target.join(prefix);
            match std::fs::symlink_metadata(&destination) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "destination namespace already exists",
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            sources.push((source, destination));
        }
        if sources.is_empty() {
            return Ok(0);
        }
        std::fs::create_dir_all(target)?;
        let target = target.canonicalize()?;
        let source_root = self.root.canonicalize()?;
        if target.starts_with(source_root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination resolves inside source",
            ));
        }
        let mut moved = 0;
        for (source, destination) in sources {
            if std::fs::rename(&source, &destination).is_err() {
                let staging = tempfile::tempdir_in(&target)?;
                copy_tree(&source, staging.path())?;
                // No partial namespace is published if copying fails.
                std::fs::rename(staging.path(), &destination)?;
                std::fs::remove_dir_all(&source)?;
            }
            moved += 1;
        }
        Ok(moved)
    }
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "recording relocation cannot follow symlinks",
            ));
        }
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            std::fs::create_dir(&target)?;
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported recording object type",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocation_moves_only_the_selected_local_namespaces() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let target = directory.path().join("target");
        std::fs::create_dir_all(source.join("bilibili/123/live")).unwrap();
        std::fs::write(source.join("bilibili/123/live/playlist.m3u8"), b"playlist").unwrap();
        std::fs::write(source.join("unrelated"), b"keep").unwrap();
        let backend = LocalBackend::new(&source);
        assert_eq!(
            backend
                .relocate_prefixes(&target, &["bilibili", "rtmp"])
                .unwrap(),
            1
        );
        assert_eq!(
            std::fs::read(target.join("bilibili/123/live/playlist.m3u8")).unwrap(),
            b"playlist"
        );
        assert_eq!(std::fs::read(source.join("unrelated")).unwrap(), b"keep");
        assert!(!source.join("bilibili").exists());
    }

    #[test]
    fn conflicting_target_is_not_overwritten_or_counted_as_success() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let target = directory.path().join("target");
        std::fs::create_dir_all(source.join("rtmp")).unwrap();
        std::fs::create_dir_all(target.join("rtmp")).unwrap();
        let backend = LocalBackend::new(&source);
        assert_eq!(
            backend
                .relocate_prefixes(&target, &["rtmp"])
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(source.join("rtmp").exists());
    }
}
