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
- Both sides are editable; the comparison updates as you type
- Save each side back to disk (keeping encoding and line endings), revert,
  and ask about unsaved changes before closing a tab or quitting
- Copy a changed block or a single line from one side to the other
- Find and replace across both sides, including multi-line patterns
- Optional word wrap that keeps the two sides aligned

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

### Build Status: COMPILES, 0 clippy warnings, 117 tests passing

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
| Text compare | Side-by-side aligned rows, line numbers, red/green line backgrounds, word-level highlights, syntax highlighting, horizontal scrolling (trackpad or Shift+wheel), tab expansion, UTF-8/UTF-16 BOM decoding, next/previous difference (moves the cursor there). A file that ends with a newline shows an empty last line, as editors do |
| Editing | Both sides are editable in place: cursor, mouse and keyboard selection (double-click word, triple-click line, drag), word/line/page movement, smart Home, auto-indent on Enter, Tab inserts a tab or spaces to the next stop, cut/copy/paste (whole line when nothing is selected), undo/redo per side (typing merges into one step), macOS input methods. After each edit the changed region is re-diffed immediately and the whole comparison is re-diffed in the background 150 ms after typing stops, so lines that start matching or diverging are re-highlighted. Read-only: files that are not valid UTF-8/UTF-16 and files inside archives (the header says why) |
| Saving | ● in the side header and tab title marks unsaved changes. Save (header button, ⌘S for the side with the cursor, ⌥⌘S for both) writes atomically (temp file + rename, permissions kept, symlinks followed) in the original encoding (UTF-8, UTF-8 BOM, UTF-16 LE/BE) with each line's original terminator; new lines use the file's most common one. If the file changed on disk since it was loaded, Save asks before overwriting. Revert (header button or File menu) reloads from disk. Closing a tab, closing the window, or quitting asks Save / Don't Save / Cancel. Picking a new file into a tab with unsaved edits opens it in a new tab instead. Saving a file shown in a folder comparison re-checks that row (and its folders) |
| File details | Each file header shows size and modification time. Hovering the size says how much larger or smaller the file is than the other side (with exact byte counts); hovering the date gives a human-readable difference such as "1 hour newer" or "3 days older". In the merge view, left and right are compared with the base |
| Context menu (text) | Right-click puts the cursor on the clicked line (unless inside the selection) and shows: Insert Line (an empty line at that position, or filling a blank spacer row), Copy Line Left/Right (overwrite the matching line on that side), Copy Section Left/Right (copy the whole run of differing lines, replacing that side's lines), each disabled when it does not apply. A separator divides them from the copy-path and reveal-in-Finder items. The same commands are in the Edit menu |
| Copy between sides | The column between the sides has « » on the first row of each changed block (copy the whole block left or right) and ‹ › on hover for a single line. ⌃⌥←/→ copies the block at the cursor, ⌃⌥⇧←/→ the line. Copying a line that exists on only one side inserts it; copying from a blank spacer deletes the other side's line. Each copy is one undo step |
| Find and replace | ⌘F / ⌥⌘F (also Edit menu) opens a find bar seeded with the selection. Options: match case, whole word, regular expression. Matches are highlighted on both sides with an "n of m" count; Enter / ⌘G / ⌘⇧G move through them down the comparison. ⇧Enter adds a line break in the find or replace field, so patterns and replacements can span lines. Replace / Replace All act on the side with the cursor by default, or Left / Right / Both; in regex mode `$1`, `\n`, `\t` are expanded. Replace All is one undo step per side. Esc closes the bar |
| Word wrap | ↩ Wrap toolbar toggle, ⌥Z, or Compare menu; applies to text comparisons, the merge view, and its output editor, and is saved in the session. Rows grow to the taller side so both stay aligned; Up/Down move by visual line within a wrapped line |
| Binary compare | NUL-byte sniffing (first 8 KB), hex + ASCII view (16 bytes/row), differing bytes highlighted, byte count and percentage, next/previous difference |
| Folder compare | Parallel scan on a dedicated 4-thread pool, size short-circuit then streaming early-exit content compare, merged left/right tree, folders-first sorting by name/size/date, expand/collapse, "differences only" filter, keyboard navigation, double-click/Enter opens file pair in a new tab, live progress, cancellation on tab close |
| Merge | Left/base/right panes, conflict (yellow) / left (red) / right (green) / both (blue) coloring, auto-merge of non-conflicting regions, take left/right/both/base per conflict, merged output pane, manual resolution by editing the output (code editor), save with conflict markers for unresolved regions, syntax highlighting |
| Archives (`compare-archive`) | ZIP, TAR, TAR.GZ/BZ2/XZ, single GZ/BZ2/XZ, RAR (`unar`/`lsar`, falls back to `7z`), 7z (`7z`); list, extract entry, extract all (path traversal guarded). Archives chosen as compare targets are extracted to a temp dir and compared as folders |
| UI shell | Tabs (new/close/switch from the tab bar), a grouped toolbar (inputs as icon buttons, Compare as the one primary action, Prev/Next and the Whitespace/Case/Wrap options as segmented controls, Beautify), hover tooltips with shortcuts, a path bar with File…/Folder… pickers and drag-and-drop shown until a comparison loads (then two-line file headers name the files), status bar summary, welcome/empty/loading/error states, menu bar, window title. Monochrome icons come from `gpui-component-assets`; buttons have distinct enabled, hover, pressed, active, and disabled states |
| Crash log | Panic hook appends to `~/Library/Application Support/CompareRS/crash.log`, along with comparison and save errors; if a crash was recorded, the next launch offers to reveal the log in Finder |
| Session | Window size/position, open tabs, active tab, and diff options are saved to `session.json` in the same folder and restored at launch (command-line paths replace saved tabs) |
| Context menus | Right-click in folder, text, hex, and merge views: compare/expand, copy left/right path, reveal left/right in Finder |
| Times | Shown in the local time zone |
| Text compare (pasted) | File → New Text Compare (⌘N) or "📝 New Text" opens an empty editable comparison with "Untitled" on each side; paste or type and the diff updates live. Saving an untitled side asks where to save it. Untitled tabs are not restored at launch |
| Beautify | Toolbar "{ } Beautify" / ⌘⇧B / Compare menu reformats the text in each editable side as an undoable edit: JSON, Python `repr` output (dicts, lists, tuples, `Name(...)`), and JSON payloads at the end of log lines are spread one element per line, and literal `\n`, `\t`, `\r` escapes become real line breaks (wrapping quotes are dropped). The result can be saved, undone (⌘Z), or reverted |
| Clipboard | Cmd+Shift+C copies the compared paths (or selected folder entry's paths) |

Rows are rendered with GPUI `list` (text and merge views, which need variable
row heights for word wrap) or `uniform_list` (folder and hex views), so only
visible rows are laid out regardless of file size.

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
by unit tests only.

Editing was checked in the app: typing that makes lines match or diverge,
Enter with auto-indent, undo/redo, double-click and drag selection, word wrap
with movement inside a wrapped line, copying a block and a line, saving (file
contents and permissions checked on disk), Revert, multi-line Replace All on
both sides, Beautify on files and on new text, and the close-tab and quit
prompts. Not yet tried by hand: Save As for untitled text, the "changed on
disk" prompt, macOS input methods, folder-row refresh after saving, read-only
archive members, and word wrap in the merge view (these have unit tests or
share code with the checked paths).

### Out of Scope / Possible Future Work

| Feature | Notes |
|---------|-------|
| Find in selection | Find and replace always search whole sides |
| Live reload | Files changed by other programs are only noticed when saving |
| Tree-sitter grammars | `compare-syntax` is a fast lexer, not a parser; nested languages (e.g. JS inside HTML) are not highlighted |

### Keyboard Shortcuts (in addition to section 7)

| Shortcut | Action |
|----------|--------|
| Cmd+T | New tab |
| Cmd+Shift+M | Three-way merge |
| Alt+Down / F8 | Next difference |
| Alt+Up / F7 | Previous difference |
| Cmd+Shift+C | Copy paths |
| Cmd+N | New text compare |
| Cmd+Shift+B | Beautify (undoable edit) |
| Cmd+S / Alt+Cmd+S | Save side with cursor / save both |
| Cmd+F / Alt+Cmd+F | Find / find and replace |
| Cmd+G / Cmd+Shift+G | Next / previous match |
| Shift+Enter (find bar) | Line break in the find or replace text |
| Alt+Cmd+Enter (replace field) | Replace all |
| Ctrl+Alt+Left/Right | Copy changed block to the left/right |
| Ctrl+Alt+Shift+Left/Right | Copy line to the left/right |
| Alt+Z | Toggle word wrap |
| Cmd+Z / Cmd+Shift+Z | Undo / redo (per side) |
| Up/Down/Left/Right/Enter | Navigate the folder tree (when focused) |

---

## Architecture Notes

### Crates

- `compare-diff`: pure algorithms, no I/O. `diff_lines`, `diff_inline`, `merge3`.
- `compare-core`: comparison model. `compare_files` → `FileComparison` (text or
  binary), `compare_folders` → `FolderComparison` (flat node arena, with
  `refresh_file` after a save), `MergeDocument`, `CompareTarget` (archive
  extraction to temp dirs, shared by every tab opened from it). Editing:
  `EditBuffer` (lines, per-line terminators, encoding), `History` (undo/redo
  with merged typing and a saved marker), `LiveDiff` (aligned rows patched
  locally after each edit), `SearchQuery` (regex-backed find/replace across
  lines), `write_atomically`.
- `compare-archive`: archive detection, listing, and extraction.
- `compare-beautify`: bracket-aware pretty-printer and escape expansion; never
  parses values, so non-JSON syntax (single quotes, `None`, trailing commas)
  is fine.
- `compare-syntax`: language detection by file name and per-line lexing with
  carried-over state, so only visible rows are highlighted.
- `compare-app`: GPUI views. `Workspace` (root) owns `Tab`s; each tab shows one of
  `TextDiffView`, `HexDiffView`, `FolderDiffView`, or `MergeView`.
  `TextDiffView` is split into submodules (editing, motion, copying, find,
  saving, input handler, render); each painted line is an `EditorLine` element
  that wraps `StyledText`, draws the cursor, and records its text layout for
  mouse hit-testing and input-method placement.

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
- gpui-component 0.5.1 (built for GPUI 0.2.2) for the merge output editor, the
  find bar fields, and context menus; its tree-sitter feature is off to keep
  the build light. Its code editor cannot color lines, insert spacer lines, or
  scroll in step with another editor, so the side-by-side editor is custom
- The text model is line-based (`Vec<String>`); edits re-diff only the changed
  region at once and the whole text after a 150 ms pause
- Archives are compared as folders
