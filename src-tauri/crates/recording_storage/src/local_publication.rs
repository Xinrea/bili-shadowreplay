//! No-clobber object publication for local transfer destinations.
use std::{fs::File, io, path::Path};

use crate::{invalid_key, transfer};

pub(super) fn create(
    root: &Path,
    destination: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let parent = destination.parent().ok_or_else(invalid_key)?;
    std::fs::create_dir_all(parent)?;
    // Stage on the destination storage, but OUTSIDE its platform/room/live
    // namespace. A process crash may leave this private directory behind;
    // it must not become an unknown object in the transfer's exact manifest.
    // Do not filter arbitrary .tmp files from listings or delete unowned files.
    let staging = tempfile::Builder::new()
        .prefix(".recording-transfer-object-")
        .tempdir_in(root)?;
    let mut file = tempfile::NamedTempFile::new_in(staging.path())?;
    write(file.as_file_mut())?;
    file.as_file().sync_all()?;
    let path = file.into_temp_path();
    // Unlike a link-then-unlink publication, atomic no-replace rename cannot
    // leave an extra hardlink after a crash (transfer verification rejects it).
    transfer::rename_new(&path, destination)?;
    transfer::sync_directory(parent)?;
    transfer::sync_directory(staging.path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn publication_does_not_overwrite_a_racing_destination() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("platform/room/live/file");
        let error = create(root.path(), &target, |file| {
            file.write_all(b"new")?;
            std::fs::write(&target, b"existing")
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&target).unwrap(), b"existing");
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            1,
            "normal errors clean private staging"
        );
    }

    /// Invoked only by the object-transfer crash test in a subprocess. _exit's
    /// equivalent is intentional: normal stack unwinding would hide this bug.
    #[test]
    fn exit_during_publication() {
        let Some(root) = std::env::var_os("BSR_TEST_PUBLICATION_ROOT") else {
            return;
        };
        let key = std::env::var("BSR_TEST_PUBLICATION_KEY").unwrap();
        let root = std::path::PathBuf::from(root);
        let destination = root.join("bilibili/room/live").join(key);
        create(&root, &destination, |file| {
            file.write_all(b"interrupted partial object").unwrap();
            file.sync_all().unwrap();
            std::process::exit(73);
        })
        .unwrap();
        panic!("subprocess must exit without dropping the staging file");
    }
}
