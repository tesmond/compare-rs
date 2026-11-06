//! Writing files so that a crash never leaves a half-written file.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::file_info::FileInfo;

/// Writes `bytes` to `path` by writing a temporary file in the same folder
/// and renaming it over the original, keeping the original's permissions.
/// A symbolic link is followed, so the file it points to is replaced rather
/// than the link.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<FileInfo> {
    let target = match fs::canonicalize(path) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let folder = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut temp = tempfile::Builder::new().prefix(".compare-rs-save-").tempfile_in(folder)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    if let Ok(metadata) = fs::metadata(&target) {
        fs::set_permissions(temp.path(), metadata.permissions())?;
    }
    temp.persist(&target).map_err(|error| error.error)?;
    Ok(FileInfo::from_path(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_contents_and_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("script.sh");
        fs::write(&path, "old").expect("write");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        let info = write_atomically(&path, b"new contents").expect("save");
        assert_eq!(fs::read(&path).expect("read"), b"new contents");
        assert_eq!(info.size, 12);
        let mode = fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        let leftovers = fs::read_dir(dir.path()).expect("list").count();
        assert_eq!(leftovers, 1, "no temporary files are left behind");
    }

    #[test]
    fn writes_through_symlinks_and_creates_new_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let real = dir.path().join("real.txt");
        fs::write(&real, "a").expect("write");
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        write_atomically(&link, b"b").expect("save");
        assert!(fs::symlink_metadata(&link).expect("stat").file_type().is_symlink());
        assert_eq!(fs::read(&real).expect("read"), b"b");

        let fresh = dir.path().join("new.txt");
        write_atomically(&fresh, b"c").expect("create");
        assert_eq!(fs::read(&fresh).expect("read"), b"c");
    }
}
