//! Human-readable formatting of sizes, times, and line text.

use std::path::Path;
use std::time::SystemTime;

/// Lines longer than this many characters are truncated for display.
const MAX_DISPLAY_CHARS: usize = 2000;
const TAB_WIDTH: usize = 4;

/// Formats a byte count, e.g. `1.5 MB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Formats a timestamp as `YYYY-MM-DD HH:MM` in the local time zone.
pub fn format_time(time: Option<SystemTime>) -> String {
    time.map(|time| {
        chrono::DateTime::<chrono::Local>::from(time).format("%Y-%m-%d %H:%M").to_string()
    })
    .unwrap_or_default()
}

/// Formats a path as `name — parent`, with the home directory shown as `~`,
/// so the file name stays visible when the label is truncated.
pub fn path_label(path: &Path) -> String {
    let Some(name) = path.file_name() else {
        return path.display().to_string();
    };
    let parent = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    if parent.is_empty() {
        return name.to_string_lossy().into_owned();
    }
    let parent = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && parent.starts_with(&home) => {
            format!("~{}", &parent[home.len()..])
        }
        _ => parent,
    };
    format!("{} — {parent}", name.to_string_lossy())
}

/// Widest line of `document` in display columns (after tab expansion),
/// capped at the display limit.
pub fn max_display_columns(document: &compare_core::TextDocument) -> usize {
    (0..document.len())
        .map(|i| {
            let line = document.line(i);
            // Byte length bounds the column count; only expand when needed.
            if line.contains('\t') {
                display_line(line).chars().count()
            } else {
                line.chars().count()
            }
        })
        .max()
        .unwrap_or(0)
        .min(MAX_DISPLAY_CHARS + 1)
}

/// Prepares a line for display: expands tabs to tab stops and truncates very
/// long lines.
pub fn display_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len().min(MAX_DISPLAY_CHARS));
    let mut column = 0;
    for ch in line.chars() {
        if column >= MAX_DISPLAY_CHARS {
            out.push('…');
            break;
        }
        if ch == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            out.push(ch);
            column += 1;
        }
    }
    out
}

/// One character of a line as displayed: where it starts in the line, in
/// the display text, and in display columns.
#[derive(Clone, Copy, Debug)]
struct DisplayStep {
    byte: usize,
    display_byte: usize,
    column: usize,
}

/// Walks the characters of `line` in step with [`display_line`], ending with
/// a step for the end of the line (or the truncation point).
fn display_steps(line: &str) -> impl Iterator<Item = DisplayStep> + '_ {
    let mut display_byte = 0;
    let mut column = 0;
    let mut done = false;
    line.char_indices().map(Some).chain(std::iter::once(None)).map_while(move |item| {
        if done {
            return None;
        }
        let step = DisplayStep { byte: item.map_or(line.len(), |(i, _)| i), display_byte, column };
        match item {
            Some(_) if column >= MAX_DISPLAY_CHARS => done = true,
            Some((_, '\t')) => {
                let spaces = TAB_WIDTH - column % TAB_WIDTH;
                display_byte += spaces;
                column += spaces;
            }
            Some((_, ch)) => {
                display_byte += ch.len_utf8();
                column += 1;
            }
            None => done = true,
        }
        Some(step)
    })
}

/// Converts a byte offset in `line` to the matching byte offset in its
/// [`display_line`] text.
pub fn to_display_offset(line: &str, byte: usize) -> usize {
    display_steps(line).take_while(|s| s.byte <= byte).last().map_or(0, |s| s.display_byte)
}

/// Converts a byte offset in the [`display_line`] text of `line` back to a
/// byte offset in `line`. Offsets inside an expanded tab snap to the nearer
/// side of it.
pub fn from_display_offset(line: &str, display_byte: usize) -> usize {
    let mut previous: Option<DisplayStep> = None;
    for step in display_steps(line) {
        if step.display_byte >= display_byte {
            return match previous {
                Some(p) if display_byte - p.display_byte < step.display_byte - display_byte => {
                    p.byte
                }
                _ => step.byte,
            };
        }
        previous = Some(step);
    }
    previous.map_or(0, |s| s.byte)
}

/// The display column of byte offset `byte` in `line`.
pub fn display_column(line: &str, byte: usize) -> usize {
    display_steps(line).take_while(|s| s.byte <= byte).last().map_or(0, |s| s.column)
}

/// The byte offset in `line` nearest to display column `column`.
pub fn offset_at_column(line: &str, column: usize) -> usize {
    display_steps(line).take_while(|s| s.column <= column).last().map_or(0, |s| s.byte)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn times() {
        let time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(format_time(None), "");
        assert_eq!(format_time(Some(time)).len(), "2023-11-14 22:13".len());
    }

    #[test]
    fn path_labels_lead_with_the_name() {
        assert_eq!(path_label(Path::new("/opt/data/file.txt")), "file.txt — /opt/data");
    }

    #[test]
    fn maps_offsets_through_tabs() {
        let line = "a\tbé";
        assert_eq!(display_line(line), "a   bé");
        assert_eq!(to_display_offset(line, 2), 4);
        assert_eq!(to_display_offset(line, line.len()), "a   bé".len());
        assert_eq!(from_display_offset(line, 4), 2);
        assert_eq!(from_display_offset(line, 2), 1);
        assert_eq!(from_display_offset(line, 3), 2);
        assert_eq!(display_column(line, 3), 5);
        assert_eq!(offset_at_column(line, 2), 1);
        assert_eq!(offset_at_column(line, 99), line.len());
    }

    #[test]
    fn tabs_expand_to_stops() {
        assert_eq!(display_line("a\tb"), "a   b");
        assert_eq!(display_line("\tx"), "    x");
    }
}
