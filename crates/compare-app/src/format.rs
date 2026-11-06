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

/// Formats a byte count with thousands separators, e.g. `12,345 bytes`.
pub fn format_bytes(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let unit = if bytes == 1 { "byte" } else { "bytes" };
    format!("{grouped} {unit}")
}

/// Describes how `size` compares with `other`'s size, e.g.
/// `312 B smaller than the right file (1,024 vs 1,336 bytes)`.
pub fn size_difference(size: u64, other: u64, other_name: &str) -> String {
    let exact = format!("({} vs {})", format_bytes(size), format_bytes(other));
    match size.cmp(&other) {
        std::cmp::Ordering::Equal => {
            format!("Same size as the {other_name} ({})", format_bytes(size))
        }
        std::cmp::Ordering::Greater => {
            format!("{} larger than the {other_name} {exact}", format_size(size - other))
        }
        std::cmp::Ordering::Less => {
            format!("{} smaller than the {other_name} {exact}", format_size(other - size))
        }
    }
}

/// Describes how `time` compares with `other`, e.g. `3 days older than the
/// right file`.
pub fn time_difference(
    time: Option<SystemTime>,
    other: Option<SystemTime>,
    other_name: &str,
) -> String {
    let (Some(time), Some(other)) = (time, other) else {
        return format!("No modification time to compare with the {other_name}");
    };
    let (gap, direction) = match time.duration_since(other) {
        Ok(gap) => (gap, "newer"),
        Err(error) => (error.duration(), "older"),
    };
    if gap.as_secs() == 0 {
        return format!("Modified at the same time as the {other_name}");
    }
    format!("{} {direction} than the {other_name}", human_duration(gap))
}

/// A duration in the largest whole unit that fits, e.g. `1 hour`, `3 days`.
pub fn human_duration(duration: std::time::Duration) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const MONTH: u64 = 30 * DAY;
    const YEAR: u64 = 365 * DAY;
    let seconds = duration.as_secs();
    let (count, unit) = match seconds {
        s if s >= YEAR => (s / YEAR, "year"),
        s if s >= MONTH => (s / MONTH, "month"),
        s if s >= 7 * DAY => (s / (7 * DAY), "week"),
        s if s >= DAY => (s / DAY, "day"),
        s if s >= HOUR => (s / HOUR, "hour"),
        s if s >= MINUTE => (s / MINUTE, "minute"),
        s => (s, "second"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural}")
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

/// The final component of `path`, or the whole path if it has none.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The folder holding `path`, with the home directory shown as `~`.
pub fn parent_label(path: &Path) -> String {
    let parent = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && parent.starts_with(&home) => {
            format!("~{}", &parent[home.len()..])
        }
        _ => parent,
    }
}

/// Shortens `text` to at most `max_chars` characters by replacing its start
/// with "…", cutting at a `/` where possible so whole folder names remain.
pub fn elide_start(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let skip = count - max_chars.saturating_sub(1);
    let tail_start = text.char_indices().nth(skip).map_or(text.len(), |(index, _)| index);
    let tail = &text[tail_start..];
    let tail = tail.find('/').map_or(tail, |slash| &tail[slash..]);
    format!("…{tail}")
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
    fn describes_size_differences() {
        assert_eq!(format_bytes(1_234_567), "1,234,567 bytes");
        assert_eq!(format_bytes(1), "1 byte");
        assert_eq!(
            size_difference(1024, 1336, "right file"),
            "312 B smaller than the right file (1,024 bytes vs 1,336 bytes)"
        );
        assert_eq!(
            size_difference(3072, 1024, "left file"),
            "2.0 KB larger than the left file (3,072 bytes vs 1,024 bytes)"
        );
        assert_eq!(size_difference(5, 5, "base"), "Same size as the base (5 bytes)");
    }

    #[test]
    fn describes_time_differences() {
        let base = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let hour = Duration::from_secs(3600);
        assert_eq!(
            time_difference(Some(base + hour), Some(base), "right file"),
            "1 hour newer than the right file"
        );
        assert_eq!(
            time_difference(Some(base), Some(base + hour * 72), "left file"),
            "3 days older than the left file"
        );
        assert_eq!(
            time_difference(Some(base), Some(base), "left file"),
            "Modified at the same time as the left file"
        );
        assert!(time_difference(None, Some(base), "base").starts_with("No modification time"));
        assert_eq!(human_duration(Duration::from_secs(90)), "1 minute");
        assert_eq!(human_duration(Duration::from_secs(14 * 86_400)), "2 weeks");
        assert_eq!(human_duration(Duration::from_secs(400 * 86_400)), "1 year");
    }

    #[test]
    fn elides_long_folders_from_the_start() {
        assert_eq!(elide_start("/a/b", 10), "/a/b");
        assert_eq!(elide_start("/one/two/three/four", 12), "…/three/four");
        assert_eq!(file_name(Path::new("/x/y.txt")), "y.txt");
    }

    #[test]
    fn tabs_expand_to_stops() {
        assert_eq!(display_line("a\tb"), "a   b");
        assert_eq!(display_line("\tx"), "    x");
    }
}
