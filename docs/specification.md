# compare-rs — Specification

## Project Overview

A macOS ARM-targeted File Compare tool built in Rust with GPUI 0.2.2 for GPU-accelerated rendering. Focus is on high-performance file/folder comparison with a split-pane diff UI, merge support, and archive format handling. No version control integration is needed.

## Tech Stack

- **Language**: Rust (edition 2024)
- **GUI**: GPUI 0.2.2 (Zed team framework)
- **Target**: macOS ARM only (M-series, no x86)
- **Workspace**: 6 crates under `crates/`

### Workspace Layout

| Crate       | Purpose |
|-------------|---------|
| `compare-app`    | GPUI frontend, app shell, actions, UI rendering |
| `compare-diff`   | LCS-based text diff engine (line and character level) |
| `compare-core`   | File/folder metadata, comparison logic, data types |
| `compare-archive`| Archive format support (ZIP, TAR, GZ, BZ2, XZ, RAR, 7z) |
| `compare-syntax` | Lightweight line-oriented syntax highlighter (~20 languages) |
| `compare-beautify` | Tolerant reformatter for JSON, Python dicts/reprs, and `\n`-escaped text |

---

## Required Functionality

### 1. File Comparison (Text)

- Select two files (left and right) via system file picker
- Compare files for identity (byte-level)
- Detect binary vs text content (null byte heuristic)
- Perform LCS-based line diff with hunk output
- Render split-pane inline diff with:
  - Line numbers
  - Syntax-colored highlighting for added/deleted/unchanged lines
  - Left file header, right file header
  - Scrollable panes
- Character-level diff within changed lines (inline highlighting)
- Color coding: red for deletions, green for insertions, gray for context

### 2. File Comparison (Binary)

- Detect binary files (presence of null bytes, size mismatch)
- Display hex dump view with:
  - Offset column (hex)
  - Hex bytes (16 per row)
  - ASCII representation column
  - Side-by-side comparison
- Highlight differences in hex view
- Show file sizes, modification times
- Show diff summary (bytes different, percentage)

### 3. Folder Comparison

- Select two directories via system folder picker
- Recursive directory tree scan
- Annotate each entry with status:
  - `Identical` — same path, same content (for folders: every descendant identical)
  - `Different` — same path, different content (for folders: contains a difference)
  - `LeftOnly` — exists only in left tree
  - `RightOnly` — exists only in right tree
  - `Error` — could not be read on at least one side
- Tree view with:
  - Indentation showing hierarchy
  - Status-colored left margin bar
  - File/folder icons (📁 / 📄)
  - Sortable by name, size, modification date
- Click behavior: open file comparison from folder tree selection
- Progress indicator for large folder comparisons

### 4. Merge (Three-Way)

- Three-pane view: left / base / right
- Select left file, base file (common ancestor), right file
- Merge diff combining left-base and right-base hunks
- Mark conflicts (lines changed in both sides)
- Auto-resolve non-conflicting differences
- Color coding: red (left only), green (right only), yellow (conflicts), gray (base)
- Accept left / accept right / manual resolution
- Output merged result to file

### 5. Archive Support

- Recognize archive files by extension and magic bytes:
  - ZIP (PK header, .zip), TAR (.tar), GZ (.gz)
  - BZ2 (.bz2), XZ (.xz), RAR (.rar via `unar`)
  - 7z (.7z via `7z` CLI tool)
- List archive entries
- Extract individual entries
- Compare files inside archives (extract to temp, compare)
- Compare archive vs extracted folder
- Compare two archives

### 6. UI Shell

- Window with:
  - Title bar ("compare-rs")
  - Tab bar (multiple comparison tabs)
  - Toolbar with action buttons
  - Content area (diff / tree / empty state)
  - Status bar (file names, tab count, shortcuts)
- Tab management: create, close, switch, navigate (Cmd+[/])
- Window close / quit handling
- Welcome screen for new/inactive tabs
- Status messages for compare progress and results

### 7. Keyboard Shortcuts

| Shortcut       | Action             |
|----------------|---------------------|
| Cmd+O          | Open left file      |
| Cmd+Shift+O    | Open right file     |
| Cmd+Enter      | Compare             |
| Cmd+W          | Close tab           |
| Cmd+[           | Previous tab        |
| Cmd+]           | Next tab            |
| Cmd+Shift+E    | Open left folder    |
| Cmd+Shift+R    | Open right folder   |
| Cmd+Shift+L    | Compare folders     |
| Cmd+Q          | Quit                |

### 8. Crash Logging

- Local crash log file (no external telemetry)
- Log path: `~/Library/Application Support/CompareRS/crash.log`
- Capture panics and unhandled errors
- Present crash dialog with option to view log

---

## Current Implementation State

### Build Status: COMPILES, 0 clippy warnings, 82 tests passing

`cargo build --release` produces `target/release/compare-rs` (`make app` builds
`target/macos/CompareRS.app`). Usage:
`compare-rs [LEFT RIGHT | LEFT BASE RIGHT]` (two paths compare, three paths merge).

The original GPUI async blocker was caused by using `BackgroundExecutor::spawn`
(which requires `Send`) to update entities. All handlers now use the GPUI 0.2.2
pattern: CPU/IO work runs in `cx.background_executor().spawn(...)` (pure, `Send`
data only), and its result is applied with
`cx.spawn(async move |this, cx| this.update(cx, ...))`. Results carry a tab id
and generation counter so stale results are dropped.

### Implemented

| Area | Details |
|------|---------|
| Diff engine (`compare-diff`) | Linear-space Myers with line interning (FxHash) and removal of lines unique to one side; cost limit for pathological inputs; word-level inline diff; diff3-style `merge3`; ignore whitespace / case options |
| Text compare | Side-by-side aligned rows, line numbers, red/green line backgrounds, word-level highlights, syntax highlighting, horizontal scrolling (trackpad or Shift+wheel), tab expansion, UTF-8/UTF-16 BOM decoding, next/previous difference |
| Binary compare | NUL-byte sniffing (first 8 KB), hex + ASCII view (16 bytes/row), differing bytes highlighted, byte count and percentage, next/previous difference |
| Folder compare | Parallel scan on a dedicated 4-thread pool, size short-circuit then streaming early-exit content compare, merged left/right tree, folders-first sorting by name/size/date, expand/collapse, "differences only" filter, keyboard navigation, double-click/Enter opens file pair in a new tab, live progress, cancellation on tab close |
| Merge | Left/base/right panes, conflict (yellow) / left (red) / right (green) / both (blue) coloring, auto-merge of non-conflicting regions, take left/right/both/base per conflict, merged output pane, manual resolution by editing the output (code editor), save with conflict markers for unresolved regions, syntax highlighting |
| Archives (`compare-archive`) | ZIP, TAR, TAR.GZ/BZ2/XZ, single GZ/BZ2/XZ, RAR (`unar`/`lsar`, falls back to `7z`), 7z (`7z`); list, extract entry, extract all (path traversal guarded). Archives chosen as compare targets are extracted to a temp dir and compared as folders |
| UI shell | Tabs (new/close/switch), toolbar, path bar with File…/Folder… pickers and drag-and-drop, status bar summary, welcome/empty/loading/error states, menu bar, window title |
| Crash log | Panic hook appends to `~/Library/Application Support/CompareRS/crash.log`, along with comparison and save errors; if a crash was recorded, the next launch offers to reveal the log in Finder |
| Session | Window size/position, open tabs, active tab, and diff options are saved to `session.json` in the same folder and restored at launch (command-line paths replace saved tabs) |
| Context menus | Right-click in folder, text, hex, and merge views: compare/expand, copy left/right path, reveal left/right in Finder |
| Times | Shown in the local time zone |
| Text compare (pasted) | File → New Text Compare (⌘N) or "📝 New Text" opens a tab with two code editors; paste or type on each side and press Compare (⌘↩). Edit Text returns to the editors, Swap Sides and Clear are available. Pasted tabs are not saved in the session |
| Beautify | Toolbar "{ } Beautify" / ⌘⇧B / Compare menu. Before diffing, JSON, Python `repr` output (dicts, lists, tuples, `Name(...)`), and JSON payloads at the end of log lines are spread one element per line, and literal `\n`, `\t`, `\r` escapes become real line breaks (wrapping quotes are dropped). Applies to file and pasted-text comparisons; a banner notes that line numbers refer to the reformatted text. Saved in the session |
| Clipboard | Cmd+Shift+C copies the compared paths (or selected folder entry's paths) |

All rendering of rows uses GPUI `uniform_list`, so only visible rows are laid
out regardless of file size.

### Performance (M-series Mac, release build)

| Scenario | Time |
|----------|------|
| 200k-line files, ~5k scattered edits | 12 ms |
| 300k-line files, completely different | 17 ms |
| 100k random lines from a 20-word alphabet (worst case) | 1.2 s |
| 20k files / 232 MB per side, full content compare | ~0.4 s warm |

Folder comparison deliberately uses 4 threads: on macOS, file-system calls from
more threads contend on kernel locks (14 threads measured 2.4 s vs 0.31–0.38 s
for 3–4 threads on the same tree).

### Verification Status

All features above were exercised in the running app (release build) except
drag-and-drop from Finder: synthetic mouse events could not start a Finder
drag, so dropping files onto the path bar is implemented (GPUI
`ExternalPaths`) but not yet confirmed by hand. Horizontal scrolling is covered
by unit tests only. The pasted-text tab and the Beautify toggle are covered by
unit tests of their comparison logic but were not yet clicked through in the
app (the screen was locked during testing).

### Out of Scope / Possible Future Work

| Feature | Notes |
|---------|-------|
| Copying hunks between sides of a two-way diff | Text panes are read-only; edits happen in the merge output editor |
| Tree-sitter grammars | `compare-syntax` is a fast lexer, not a parser; nested languages (e.g. JS inside HTML) are not highlighted |

### Keyboard Shortcuts (in addition to section 7)

| Shortcut | Action |
|----------|--------|
| Cmd+T | New tab |
| Cmd+Shift+M | Three-way merge |
| Alt+Down / F8 | Next difference |
| Alt+Up / F7 | Previous difference |
| Cmd+Shift+C | Copy paths |
| Cmd+N | New text compare (paste) |
| Cmd+Shift+B | Toggle Beautify |
| Up/Down/Left/Right/Enter | Navigate the folder tree (when focused) |

---

## Architecture Notes

### Crates

- `compare-diff`: pure algorithms, no I/O. `diff_lines`, `diff_inline`, `merge3`.
- `compare-core`: comparison model. `compare_files` → `FileComparison` (text or
  binary), `compare_folders` → `FolderComparison` (flat node arena),
  `MergeDocument`, `CompareTarget` (archive extraction to temp dirs).
- `compare-archive`: archive detection, listing, and extraction.
- `compare-beautify`: bracket-aware pretty-printer and escape expansion; never
  parses values, so non-JSON syntax (single quotes, `None`, trailing commas)
  is fine.
- `compare-syntax`: language detection by file name and per-line lexing with
  carried-over state, so only visible rows are highlighted.
- `compare-app`: GPUI views. `Workspace` (root) owns `Tab`s; each tab shows one of
  `TextDiffView`, `HexDiffView`, `FolderDiffView`, or `MergeView`.

### Data Flow

```
User action (button / shortcut / drop / CLI args)
  → Workspace sets a path on a tab
  → once all inputs are set: start_compare
      → background executor: compare_files / compare_folders / MergeDocument::new
      → cx.spawn awaits the result → finish_compare builds the view entity
  → cx.notify() → re-render
```

### Code Style

Follows the Google Rust style guide
(https://google.github.io/styleguide/rustguide.html): 100-column rustfmt
(`rustfmt.toml`), one primary item per file named after it, no glob imports,
documented public APIs, `expect` over `unwrap`, no `unsafe`.

### Design Decisions

- macOS ARM only
- GPUI 0.2.2 for rendering (Zed team framework)
- gpui-component 0.5.1 (built for GPUI 0.2.2) for the merge output editor and
  context menus; its tree-sitter feature is off to keep the build light
- Archives are compared as folders
