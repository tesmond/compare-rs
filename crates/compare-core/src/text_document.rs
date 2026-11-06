//! Decoded text split into lines.

use std::ops::Range;

/// How a text file's characters were encoded on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Encoding {
    #[default]
    Utf8,
    /// UTF-8 starting with a byte order mark.
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    /// Encodes `text` in this encoding, including any byte order mark.
    pub fn encode(self, text: &str) -> Vec<u8> {
        match self {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Utf8Bom => [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
            Encoding::Utf16Le => encode_utf16(text, [0xff, 0xfe], u16::to_le_bytes),
            Encoding::Utf16Be => encode_utf16(text, [0xfe, 0xff], u16::to_be_bytes),
        }
    }

    /// Short name for display, e.g. `UTF-16 LE`.
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
        }
    }
}

fn encode_utf16(text: &str, bom: [u8; 2], to_bytes: fn(u16) -> [u8; 2]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(2 + text.len() * 2);
    bytes.extend_from_slice(&bom);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&to_bytes(unit));
    }
    bytes
}

/// UTF-8 text plus the byte range of every line (without its terminator).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextDocument {
    text: String,
    lines: Vec<Range<usize>>,
    encoding: Encoding,
    /// True if some bytes could not be decoded and were replaced.
    lossy: bool,
}

impl TextDocument {
    /// Splits `text` on `\n`, also stripping a `\r` before it. A trailing
    /// newline does not start an extra empty line.
    pub fn new(text: String) -> Self {
        let bytes = text.as_bytes();
        let mut lines: Vec<Range<usize>> = Vec::with_capacity(bytes.len() / 40 + 1);
        let mut start = 0;
        for newline in memchr::memchr_iter(b'\n', bytes) {
            let end =
                if newline > start && bytes[newline - 1] == b'\r' { newline - 1 } else { newline };
            lines.push(start..end);
            start = newline + 1;
        }
        if start < bytes.len() {
            lines.push(start..bytes.len());
        }
        Self { text, lines, encoding: Encoding::Utf8, lossy: false }
    }

    /// Decodes raw bytes as UTF-8 (lossily), or as UTF-16 when a UTF-16 byte
    /// order mark is present. A UTF-8 byte order mark is dropped. The
    /// encoding is recorded so edited text can be saved the same way.
    pub fn decode(bytes: &[u8]) -> Self {
        let (text, encoding, lossy) = if let Some(rest) = bytes.strip_prefix(b"\xef\xbb\xbf") {
            let (text, lossy) = decode_utf8(rest);
            (text, Encoding::Utf8Bom, lossy)
        } else if let Some(rest) = bytes.strip_prefix(b"\xff\xfe") {
            let (text, lossy) = decode_utf16(rest, u16::from_le_bytes);
            (text, Encoding::Utf16Le, lossy)
        } else if let Some(rest) = bytes.strip_prefix(b"\xfe\xff") {
            let (text, lossy) = decode_utf16(rest, u16::from_be_bytes);
            (text, Encoding::Utf16Be, lossy)
        } else {
            let (text, lossy) = decode_utf8(bytes);
            (text, Encoding::Utf8, lossy)
        };
        Self { encoding, lossy, ..Self::new(text) }
    }

    /// The encoding the document was decoded from.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Returns true if decoding replaced invalid bytes, so saving the text
    /// would not reproduce the original file.
    pub fn is_lossy(&self) -> bool {
        self.lossy
    }

    /// Returns true if `bytes` starts with a UTF-16 byte order mark.
    pub fn has_utf16_bom(bytes: &[u8]) -> bool {
        bytes.starts_with(b"\xff\xfe") || bytes.starts_with(b"\xfe\xff")
    }

    /// Number of lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Returns line `index` without its terminator.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    pub fn line(&self, index: usize) -> &str {
        &self.text[self.lines[index].clone()]
    }

    /// Returns all lines as slices into the document.
    pub fn lines(&self) -> Vec<&str> {
        self.lines.iter().map(|range| &self.text[range.clone()]).collect()
    }

    /// The full decoded text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

fn decode_utf8(bytes: &[u8]) -> (String, bool) {
    match String::from_utf8_lossy(bytes) {
        std::borrow::Cow::Borrowed(text) => (text.to_owned(), false),
        std::borrow::Cow::Owned(text) => (text, true),
    }
}

fn decode_utf16(bytes: &[u8], to_unit: fn([u8; 2]) -> u16) -> (String, bool) {
    let (pairs, odd_byte) = bytes.as_chunks::<2>();
    let mut lossy = !odd_byte.is_empty();
    let units = pairs.iter().map(|&pair| to_unit(pair));
    let text = char::decode_utf16(units)
        .map(|c| {
            c.unwrap_or_else(|_| {
                lossy = true;
                char::REPLACEMENT_CHARACTER
            })
        })
        .collect();
    (text, lossy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines() {
        let doc = TextDocument::new("a\r\nb\n\nc".to_owned());
        assert_eq!(doc.lines(), ["a", "b", "", "c"]);
    }

    #[test]
    fn trailing_newline_adds_no_line() {
        assert_eq!(TextDocument::new("a\nb\n".to_owned()).lines(), ["a", "b"]);
        assert!(TextDocument::new(String::new()).is_empty());
    }

    #[test]
    fn decodes_utf16_le() {
        let bytes = b"\xff\xfeh\x00i\x00\n\x00";
        let doc = TextDocument::decode(bytes);
        assert_eq!(doc.lines(), ["hi"]);
        assert_eq!(doc.encoding(), Encoding::Utf16Le);
        assert_eq!(doc.encoding().encode(doc.text()), bytes);
    }

    #[test]
    fn records_bom_and_lossy_decoding() {
        let doc = TextDocument::decode(b"\xef\xbb\xbfok");
        assert_eq!((doc.encoding(), doc.is_lossy()), (Encoding::Utf8Bom, false));
        assert_eq!(doc.encoding().encode(doc.text()), b"\xef\xbb\xbfok");
        assert!(TextDocument::decode(b"bad \xff byte").is_lossy());
    }
}
