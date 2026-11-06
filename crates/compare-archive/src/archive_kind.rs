//! Archive format detection.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

/// A stream compression format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
}

impl Compression {
    /// Wraps `reader` in the matching decompressor.
    pub(crate) fn decoder<'a, R: Read + 'a>(self, reader: R) -> Box<dyn Read + 'a> {
        match self {
            Compression::None => Box::new(reader),
            Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(reader)),
            Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(reader)),
            Compression::Xz => Box::new(xz2::read::XzDecoder::new_multi_decoder(reader)),
        }
    }

    /// Opens `path` and returns a decompressing reader over its contents.
    pub(crate) fn open(self, path: &Path) -> io::Result<Box<dyn Read>> {
        let file = BufReader::new(File::open(path)?);
        Ok(self.decoder(file))
    }
}

/// A supported archive format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArchiveKind {
    Zip,
    /// A TAR archive, possibly wrapped in a compression stream.
    Tar(Compression),
    /// A single compressed file (not a TAR) such as `notes.txt.gz`.
    SingleFile(Compression),
    Rar,
    SevenZip,
}

impl ArchiveKind {
    /// Detects the archive format from the file name, falling back to the
    /// file's magic bytes. Returns `None` for non-archives.
    pub fn from_path(path: &Path) -> Option<Self> {
        Self::from_file_name(path).or_else(|| Self::from_magic(path).ok().flatten())
    }

    /// Detects the archive format from the file name alone.
    pub fn from_file_name(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        const SUFFIXES: &[(&str, ArchiveKind)] = &[
            (".tar.gz", ArchiveKind::Tar(Compression::Gzip)),
            (".tgz", ArchiveKind::Tar(Compression::Gzip)),
            (".tar.bz2", ArchiveKind::Tar(Compression::Bzip2)),
            (".tbz2", ArchiveKind::Tar(Compression::Bzip2)),
            (".tbz", ArchiveKind::Tar(Compression::Bzip2)),
            (".tar.xz", ArchiveKind::Tar(Compression::Xz)),
            (".txz", ArchiveKind::Tar(Compression::Xz)),
            (".tar", ArchiveKind::Tar(Compression::None)),
            (".zip", ArchiveKind::Zip),
            (".jar", ArchiveKind::Zip),
            (".gz", ArchiveKind::SingleFile(Compression::Gzip)),
            (".bz2", ArchiveKind::SingleFile(Compression::Bzip2)),
            (".xz", ArchiveKind::SingleFile(Compression::Xz)),
            (".rar", ArchiveKind::Rar),
            (".7z", ArchiveKind::SevenZip),
        ];
        SUFFIXES.iter().find(|(suffix, _)| name.ends_with(suffix)).map(|&(_, kind)| kind)
    }

    fn from_magic(path: &Path) -> io::Result<Option<Self>> {
        if !path.is_file() {
            return Ok(None);
        }
        let mut header = [0u8; 512];
        let mut file = File::open(path)?;
        let len = read_up_to(&mut file, &mut header)?;
        let header = &header[..len];

        let kind = if header.starts_with(b"PK\x03\x04") || header.starts_with(b"PK\x05\x06") {
            Some(ArchiveKind::Zip)
        } else if header.starts_with(b"Rar!\x1a\x07") {
            Some(ArchiveKind::Rar)
        } else if header.starts_with(b"7z\xbc\xaf\x27\x1c") {
            Some(ArchiveKind::SevenZip)
        } else if header.starts_with(b"\x1f\x8b") {
            Some(ArchiveKind::SingleFile(Compression::Gzip))
        } else if header.starts_with(b"BZh") {
            Some(ArchiveKind::SingleFile(Compression::Bzip2))
        } else if header.starts_with(b"\xfd7zXZ\x00") {
            Some(ArchiveKind::SingleFile(Compression::Xz))
        } else if header.len() >= 262 && &header[257..262] == b"ustar" {
            Some(ArchiveKind::Tar(Compression::None))
        } else {
            None
        };
        Ok(kind)
    }

    /// Name of the single member of a [`ArchiveKind::SingleFile`] archive:
    /// the file name without its compression extension.
    pub(crate) fn single_file_name(path: &Path) -> String {
        path.file_stem().map_or_else(|| "contents".to_owned(), |s| s.to_string_lossy().into_owned())
    }
}

fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_name() {
        let cases = [
            ("a.tar.gz", ArchiveKind::Tar(Compression::Gzip)),
            ("a.TGZ", ArchiveKind::Tar(Compression::Gzip)),
            ("a.tar", ArchiveKind::Tar(Compression::None)),
            ("a.tar.xz", ArchiveKind::Tar(Compression::Xz)),
            ("a.txt.gz", ArchiveKind::SingleFile(Compression::Gzip)),
            ("a.zip", ArchiveKind::Zip),
            ("a.7z", ArchiveKind::SevenZip),
        ];
        for (name, kind) in cases {
            assert_eq!(ArchiveKind::from_path(Path::new(name)), Some(kind), "{name}");
        }
    }

    #[test]
    fn non_archive_is_none() {
        assert_eq!(ArchiveKind::from_path(Path::new("/nonexistent/readme.md")), None);
    }
}
