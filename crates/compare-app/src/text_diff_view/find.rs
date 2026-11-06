//! Find and replace across both sides of a comparison.

use std::ops::Range;
use std::time::Instant;

use gpui::{
    AppContext, ClickEvent, Context, Div, Entity, InteractiveElement, ParentElement, SharedString,
    Styled, Subscription, Window, div, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{IconName, Sizable};

use compare_core::{DiffSide, Position, SearchOptions, SearchQuery};

use super::TextDiffView;
use super::side::Selection;
use crate::actions::{FindNext, FindPrev, OpenReplace, ReplaceAll, ReplaceNext};
use crate::button::{self, ButtonState, Hint, Segment};
use crate::theme;

/// Narrowest and widest the find and replace fields may be.
const FIELD_MIN_WIDTH: f32 = 240.0;
const FIELD_MAX_WIDTH: f32 = 520.0;

/// Width of the controls beside each field, the same on both rows.
const CONTROLS_WIDTH: f32 = 372.0;

/// Width of the match count, so the buttons after it do not move.
const STATUS_WIDTH: f32 = 72.0;

/// Widest the find bar's contents may be.
const BAR_MAX_WIDTH: f32 = 900.0;

/// Key context around the find field.
pub const FIND_QUERY_CONTEXT: &str = "FindQuery";
/// Key context around the replace field.
pub const REPLACE_QUERY_CONTEXT: &str = "ReplaceQuery";

/// Which sides Replace and Replace All change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaceTarget {
    Left,
    Right,
    Both,
}

impl ReplaceTarget {
    fn includes(self, side: DiffSide) -> bool {
        match self {
            ReplaceTarget::Left => side == DiffSide::Left,
            ReplaceTarget::Right => side == DiffSide::Right,
            ReplaceTarget::Both => true,
        }
    }
}

/// The open find bar and its matches.
pub struct FindState {
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    show_replace: bool,
    options: SearchOptions,
    target: ReplaceTarget,
    /// Matches on each side, in text order.
    matches: [Vec<Range<Position>>; 2],
    /// Every match as (side, index into `matches`), ordered down the
    /// comparison.
    order: Vec<(DiffSide, usize)>,
    /// Index into `order` of the selected match.
    current: Option<usize>,
    /// Why the pattern could not be used, if it could not.
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl FindState {
    /// The matches on `side` that touch `line`.
    pub fn matches_on_line(&self, side: DiffSide, line: usize) -> &[Range<Position>] {
        let matches = &self.matches[side.index()];
        let first = matches.partition_point(|m| m.end.line < line);
        let count = matches[first..].iter().take_while(|m| m.start.line <= line).count();
        &matches[first..first + count]
    }

    /// The selected match, if any.
    pub fn current_match(&self) -> Option<(DiffSide, &Range<Position>)> {
        let &(side, index) = self.order.get(self.current?)?;
        Some((side, &self.matches[side.index()][index]))
    }
}

impl TextDiffView {
    /// Opens the find bar (with the replace row if `replace`), seeded with
    /// the selected text, and focuses the find field.
    pub fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.side(self.active).selection;
        let seed = (!selection.is_empty())
            .then(|| self.side(self.active).buffer.text_in(selection.range()));
        if self.find.is_none() {
            let field = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
                cx.new(|cx| InputState::new(window, cx).auto_grow(1, 6).placeholder(placeholder))
            };
            let query = field("Find (⇧↩ adds a line break)", window, cx);
            let replacement = field("Replace", window, cx);
            let subscription = cx.subscribe_in(&query, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.update_matches(true, cx);
                }
            });
            let target = match self.active {
                DiffSide::Left => ReplaceTarget::Left,
                DiffSide::Right => ReplaceTarget::Right,
            };
            self.find = Some(FindState {
                query,
                replacement,
                show_replace: replace,
                options: SearchOptions::default(),
                target,
                matches: Default::default(),
                order: Vec::new(),
                current: None,
                error: None,
                _subscriptions: vec![subscription],
            });
        }
        let Some(find) = &mut self.find else {
            return;
        };
        find.show_replace |= replace;
        let query = find.query.clone();
        if let Some(seed) = seed {
            query.update(cx, |state, cx| state.set_value(seed, window, cx));
        }
        query.update(cx, |state, cx| state.focus(window, cx));
        self.update_matches(true, cx);
    }

    /// Closes the find bar and returns focus to the text.
    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            window.focus(&self.focus);
            cx.notify();
        }
    }

    /// Recomputes the matches after the pattern, its options, or the text
    /// changed. With `jump`, selects the first match at or after the cursor.
    pub(super) fn update_matches(&mut self, jump: bool, cx: &mut Context<Self>) {
        let Some(find) = &self.find else {
            return;
        };
        let pattern = find.query.read(cx).value().to_string();
        let options = find.options;
        let previous = find.current_match().map(|(side, range)| (side, range.start));
        let mut matches: [Vec<Range<Position>>; 2] = Default::default();
        let mut error = None;
        if !pattern.is_empty() {
            match SearchQuery::new(&pattern, options) {
                Ok(query) => {
                    for side in [DiffSide::Left, DiffSide::Right] {
                        matches[side.index()] = query.find_all(self.side(side).buffer.lines());
                    }
                }
                Err(_) => error = Some("Invalid regular expression".to_owned()),
            }
        }
        let mut order: Vec<(DiffSide, usize)> = Vec::new();
        for side in [DiffSide::Left, DiffSide::Right] {
            order.extend((0..matches[side.index()].len()).map(|index| (side, index)));
        }
        let row_of = |&(side, index): &(DiffSide, usize)| {
            let start = matches[side.index()][index].start;
            (self.diff.row_of_line(side, start.line), side.index(), start.column)
        };
        order.sort_by_key(row_of);

        let current = if jump {
            let cursor = (self.cursor_row(), self.active.index(), 0);
            let at = order.partition_point(|entry| row_of(entry) < cursor);
            (!order.is_empty()).then(|| at % order.len())
        } else {
            previous.and_then(|(side, start)| {
                order.iter().position(|&(s, i)| s == side && matches[s.index()][i].start >= start)
            })
        };
        if let Some(find) = &mut self.find {
            find.matches = matches;
            find.order = order;
            find.current = current;
            find.error = error;
        }
        if jump {
            self.select_current_match();
        }
        cx.notify();
    }

    /// Selects the current match in the text and scrolls to it.
    fn select_current_match(&mut self) {
        let Some((side, range)) =
            self.find.as_ref().and_then(|f| f.current_match()).map(|(s, r)| (s, r.clone()))
        else {
            return;
        };
        self.active = side;
        let state = self.side_mut(side);
        state.selection = Selection { anchor: range.start, head: range.end, goal_column: None };
        state.history.break_coalescing();
        self.reveal_cursor();
    }

    /// Moves to the next (or previous) match, wrapping around.
    pub fn find_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        let count = find.order.len();
        if count == 0 {
            return;
        }
        find.current = Some(match (find.current, forward) {
            (None, _) => 0,
            (Some(current), true) => (current + 1) % count,
            (Some(current), false) => (current + count - 1) % count,
        });
        self.select_current_match();
        cx.notify();
    }

    /// The compiled pattern and replacement text, if the pattern is valid.
    fn replace_inputs(&self, cx: &Context<Self>) -> Option<(SearchQuery, String, ReplaceTarget)> {
        let find = self.find.as_ref()?;
        let pattern = find.query.read(cx).value().to_string();
        if pattern.is_empty() {
            return None;
        }
        let query = SearchQuery::new(&pattern, find.options).ok()?;
        Some((query, find.replacement.read(cx).value().to_string(), find.target))
    }

    /// Replaces the current match (if it is on a side being replaced) and
    /// moves to the next one.
    pub(super) fn replace_next(&mut self, cx: &mut Context<Self>) {
        let Some((query, replacement, target)) = self.replace_inputs(cx) else {
            return;
        };
        let current =
            self.find.as_ref().and_then(|f| f.current_match()).map(|(s, r)| (s, r.clone()));
        if let Some((side, range)) = current
            && target.includes(side)
            && self.side(side).read_only.is_none()
        {
            let replacement = query
                .replacements(self.side(side).buffer.lines(), &replacement)
                .into_iter()
                .find(|(found, _)| *found == range)
                .map(|(_, text)| text);
            if let Some(text) = replacement {
                self.edit(side, range, &text, compare_core::EditKind::Other, cx);
                self.update_matches(false, cx);
                self.find_step_from_cursor(cx);
                return;
            }
        }
        self.find_step(true, cx);
    }

    /// After a replacement, selects the first match after the cursor.
    fn find_step_from_cursor(&mut self, cx: &mut Context<Self>) {
        let cursor = (self.active, self.side(self.active).selection.head);
        let Some(find) = &mut self.find else {
            return;
        };
        let next = find.order.iter().position(|&(side, index)| {
            let start = find.matches[side.index()][index].start;
            side == cursor.0 && start >= cursor.1
        });
        find.current = next.or((!find.order.is_empty()).then_some(0));
        self.select_current_match();
        cx.notify();
    }

    /// Replaces every match on the target sides, as one undo step per side.
    pub(super) fn replace_all(&mut self, cx: &mut Context<Self>) {
        let Some((query, replacement, target)) = self.replace_inputs(cx) else {
            return;
        };
        for side in [DiffSide::Left, DiffSide::Right] {
            if !target.includes(side) || self.side(side).read_only.is_some() {
                continue;
            }
            let replacements = query.replacements(self.side(side).buffer.lines(), &replacement);
            if replacements.is_empty() {
                continue;
            }
            let before = self.side(side).selection;
            // Work backwards so earlier positions stay valid.
            let edits: Vec<_> = replacements
                .into_iter()
                .rev()
                .map(|(range, text)| self.apply_edit(side, range, &text, cx))
                .collect();
            let after = Selection::caret(self.side(side).buffer.clamp(before.head));
            let state = self.side_mut(side);
            state.selection = after;
            state.history.record_group(edits, before, after, Instant::now());
        }
        self.update_matches(false, cx);
    }

    fn toggle_option(&mut self, toggle: fn(&mut SearchOptions), cx: &mut Context<Self>) {
        if let Some(find) = &mut self.find {
            toggle(&mut find.options);
            self.update_matches(true, cx);
        }
    }

    /// The find bar. Both rows share a two-column layout, a growing field
    /// and a fixed control column, so their controls line up.
    pub(super) fn render_find_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let find = self.find.as_ref()?;
        let option = |id: &'static str,
                      label: &'static str,
                      hint: &'static str,
                      on: bool,
                      flip: fn(&mut SearchOptions)| {
            Segment::new(
                id,
                label,
                ButtonState::toggle(on),
                Hint::text(hint),
                cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_option(flip, cx)),
            )
        };
        let status: SharedString = match (&find.error, find.order.len(), find.current) {
            (Some(error), _, _) => error.clone().into(),
            (None, 0, _) if find.query.read(cx).value().is_empty() => "".into(),
            (None, 0, _) => "No results".into(),
            (None, count, Some(current)) => format!("{} of {count}", current + 1).into(),
            (None, count, None) => format!("{count} found").into(),
        };
        let status_color = if find.error.is_some() { theme::red() } else { theme::subtext() };
        let field = |context: &'static str, input: &Entity<InputState>| {
            div()
                .key_context(context)
                .flex_1()
                .min_w(px(FIELD_MIN_WIDTH))
                .max_w(px(FIELD_MAX_WIDTH))
                .child(Input::new(input).small())
        };
        let controls = || {
            div()
                .flex()
                .flex_row()
                .flex_none()
                .w(px(CONTROLS_WIDTH))
                .items_center()
                .gap(px(6.0))
                .text_size(theme::ui_text_size())
        };
        let row = || div().flex().flex_row().items_start().gap(px(8.0));

        let find_controls = controls()
            .child(button::segmented(vec![
                option("find-case", "Aa", "Match case", find.options.match_case, |o| {
                    o.match_case = !o.match_case
                }),
                option("find-word", "ab", "Whole word", find.options.whole_word, |o| {
                    o.whole_word = !o.whole_word
                }),
                option("find-regex", ".*", "Regular expression", find.options.regex, |o| {
                    o.regex = !o.regex
                }),
            ]))
            .child(
                div()
                    .flex_none()
                    .w(px(STATUS_WIDTH))
                    .text_right()
                    .text_color(status_color)
                    .truncate()
                    .child(status),
            )
            .child(button::segmented(vec![
                Segment::new(
                    "find-prev",
                    IconName::ArrowUp,
                    ButtonState::Enabled,
                    Hint::action("Previous match", &FindPrev),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.find_step(false, cx)),
                ),
                Segment::new(
                    "find-next",
                    IconName::ArrowDown,
                    ButtonState::Enabled,
                    Hint::action("Next match", &FindNext),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.find_step(true, cx)),
                ),
            ]))
            .child(button::with_hint(
                button::button(
                    "find-replace-toggle",
                    "Replace",
                    ButtonState::toggle(find.show_replace),
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(find) = &mut this.find {
                            find.show_replace = !find.show_replace;
                            cx.notify();
                        }
                    }),
                ),
                Hint::action("Show replace", &OpenReplace),
            ))
            .child(button::with_hint(
                button::button(
                    "find-close",
                    IconName::Close,
                    ButtonState::Enabled,
                    cx.listener(|this, _: &ClickEvent, window, cx| this.close_find(window, cx)),
                ),
                Hint::text("Close (Esc)"),
            ));

        let replace_row = find.show_replace.then(|| {
            let target = |id: &'static str, label: &'static str, value: ReplaceTarget| {
                Segment::new(
                    id,
                    label,
                    ButtonState::toggle(find.target == value),
                    Hint::text(match value {
                        ReplaceTarget::Left => "Replace on the left side",
                        ReplaceTarget::Right => "Replace on the right side",
                        ReplaceTarget::Both => "Replace on both sides",
                    }),
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(find) = &mut this.find {
                            find.target = value;
                            cx.notify();
                        }
                    }),
                )
            };
            row().child(field(REPLACE_QUERY_CONTEXT, &find.replacement)).child(
                controls()
                    .child(button::action_button(
                        "Replace",
                        Box::new(ReplaceNext),
                        ButtonState::Enabled,
                        "Replace and find next (↩)",
                    ))
                    .child(button::action_button(
                        "Replace All",
                        Box::new(ReplaceAll),
                        ButtonState::Enabled,
                        "Replace all matches",
                    ))
                    .child(div().text_color(theme::muted()).child("in"))
                    .child(button::segmented(vec![
                        target("replace-left", "Left", ReplaceTarget::Left),
                        target("replace-right", "Right", ReplaceTarget::Right),
                        target("replace-both", "Both", ReplaceTarget::Both),
                    ])),
            )
        });

        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .px_3()
                .py(px(6.0))
                .bg(theme::mantle())
                .border_b_1()
                .border_color(theme::surface())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .max_w(px(BAR_MAX_WIDTH))
                        .child(
                            row()
                                .child(field(FIND_QUERY_CONTEXT, &find.query))
                                .child(find_controls),
                        )
                        .children(replace_row),
                ),
        )
    }
}
