//! Extracting archive members.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;

use crate::archive_error::ArchiveError;
use crate::archive_kind::{ArchiveKind, Compression};
use crate::external_tool;

/// Writes the contents of the member `entry_name` to `output`.
pub fn extract_entry<W: Write>(
    path: &Path,
    entry_name: &str,
    output: &mut W,
) -> Result<(), ArchiveError> {
    let not_found = || ArchiveError::EntryNotFound(entry_name.to_owned());
    match ArchiveKind::from_path(path).ok_or(ArchiveError::UnknownFormat)? {
        ArchiveKind::Zip => {
            let mut archive = zip::ZipArchive::new(BufReader::new(File::open(path)?))?;
            let mut entry = match archive.by_name(entry_name) {
                Ok(entry) => entry,
                Err(zip::result::ZipError::FileNotFound) => return Err(not_found()),
                Err(e) => return Err(e.into()),
            };
            io::copy(&mut entry, output)?;
            Ok(())
        }
        ArchiveKind::Tar(compression) => {
            let mut archive = tar::Archive::new(compression.open(path)?);
            for entry in archive.entries()? {
                let mut entry = entry?;
                if entry.path()?.to_string_lossy().trim_end_matches('/') == entry_name {
                    io::copy(&mut entry, output)?;
                    return Ok(());
                }
            }
            Err(not_found())
        }
        ArchiveKind::SingleFile(compression) => {
            if entry_name != ArchiveKind::single_file_name(path) {
                return Err(not_found());
            }
            io::copy(&mut compression.open(path)?, output)?;
            Ok(())
        }
        ArchiveKind::Rar | ArchiveKind::SevenZip => {
            external_tool::extract_entry_7z(path, entry_name, output)
        }
    }
}

/// Extracts every member of the archive at `path` into the directory `dest`,
/// creating it if needed.
///
/// Members whose paths would escape `dest` (absolute paths or `..`
/// components) are skipped.
pub fn extract_all(path: &Path, dest: &Path) -> Result<(), ArchiveError> {
    fs::create_dir_all(dest)?;
    match ArchiveKind::from_path(path).ok_or(ArchiveError::UnknownFormat)? {
        ArchiveKind::Zip => {
            let mut archive = zip::ZipArchive::new(BufReader::new(File::open(path)?))?;
            archive.extract(dest)?;
            Ok(())
        }
        ArchiveKind::Tar(compression) => {
            let mut archive = tar::Archive::new(compression.open(path)?);
            archive.set_preserve_permissions(false);
            for entry in archive.entries()? {
                // `unpack_in` refuses paths that escape `dest`.
                entry?.unpack_in(dest)?;
            }
            Ok(())
        }
        ArchiveKind::SingleFile(compression) => extract_single(path, compression, dest),
        ArchiveKind::Rar => external_tool::extract_all_rar(path, dest),
        ArchiveKind::SevenZip => external_tool::extract_all_7z(path, dest),
    }
}

fn extract_single(path: &Path, compression: Compression, dest: &Path) -> Result<(), ArchiveError> {
    let target = dest.join(ArchiveKind::single_file_name(path));
    let mut writer = BufWriter::new(File::create(target)?);
    io::copy(&mut compression.open(path)?, &mut writer)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::listing::list_entries;

    fn write_tar_gz(path: &Path) {
        let file = File::create(path).expect("create archive");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        let data = b"hello tar";
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, "dir/hello.txt", &data[..]).expect("append");
        builder.into_inner().expect("finish tar").finish().expect("finish gzip");
    }

    fn write_zip(path: &Path) {
        let mut writer = zip::ZipWriter::new(File::create(path).expect("create zip"));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("a/b.txt", options).expect("start file");
        writer.write_all(b"zip body").expect("write");
        writer.finish().expect("finish zip");
    }

    #[test]
    fn tar_gz_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("test.tar.gz");
        write_tar_gz(&archive);

        let entries = list_entries(&archive).expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "dir/hello.txt");

        let mut contents = Vec::new();
        extract_entry(&archive, "dir/hello.txt", &mut contents).expect("extract entry");
        assert_eq!(contents, b"hello tar");

        let out = dir.path().join("out");
        extract_all(&archive, &out).expect("extract all");
        assert_eq!(fs::read(out.join("dir/hello.txt")).expect("read"), b"hello tar");
    }

    #[test]
    fn zip_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("test.zip");
        write_zip(&archive);

        let entries = list_entries(&archive).expect("list");
        assert!(entries.iter().any(|e| e.name == "a/b.txt" && e.size == 8));

        let mut contents = Vec::new();
        extract_entry(&archive, "a/b.txt", &mut contents).expect("extract entry");
        assert_eq!(contents, b"zip body");

        let out = dir.path().join("out");
        extract_all(&archive, &out).expect("extract all");
        let mut text = String::new();
        File::open(out.join("a/b.txt")).expect("open").read_to_string(&mut text).expect("read");
        assert_eq!(text, "zip body");
    }

    #[test]
    fn single_gzip_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("notes.txt.gz");
        let mut encoder = flate2::write::GzEncoder::new(
            File::create(&archive).expect("create"),
            flate2::Compression::fast(),
        );
        encoder.write_all(b"gz body").expect("write");
        encoder.finish().expect("finish");

        let out = dir.path().join("out");
        extract_all(&archive, &out).expect("extract all");
        assert_eq!(fs::read(out.join("notes.txt")).expect("read"), b"gz body");
    }
}
