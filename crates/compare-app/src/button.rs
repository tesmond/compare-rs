//! Buttons and button groups shared by the toolbar, headers, and find bar.

use gpui::{
    Action, AnyElement, App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement,
    IntoElement, ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Window,
    div, px, transparent_black,
};
use gpui_component::{Icon, IconName, Sizable, tooltip::Tooltip};

use crate::theme;

/// Whether a button can be clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Enabled,
    Disabled,
    /// Enabled and drawn as switched on (for toggles).
    Active,
}

impl ButtonState {
    /// `Active` when `on`, otherwise `Enabled`.
    pub fn toggle(on: bool) -> Self {
        if on { ButtonState::Active } else { ButtonState::Enabled }
    }

    /// `Enabled` when `enabled`, otherwise `Disabled`.
    pub fn enabled(enabled: bool) -> Self {
        if enabled { ButtonState::Enabled } else { ButtonState::Disabled }
    }
}

/// Which corners of a button are rounded: all of them, or only the outer
/// ones of a button inside a segmented group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Corners {
    All,
    Left,
    Right,
    None,
}

/// Width of an icon-only button.
const ICON_BUTTON_WIDTH: f32 = 28.0;

/// The size of icons in buttons.
const ICON_SIZE: f32 = 14.0;

/// What a button shows.
pub enum Content {
    Label(SharedString),
    Icon(IconName),
    /// An icon with a short badge after it, e.g. "L" for the left side.
    IconBadge(IconName, &'static str),
    /// One of the app's own icons, by asset path.
    AppIcon(&'static str),
}

impl From<&'static str> for Content {
    fn from(label: &'static str) -> Self {
        Content::Label(label.into())
    }
}

impl From<String> for Content {
    fn from(label: String) -> Self {
        Content::Label(label.into())
    }
}

impl From<SharedString> for Content {
    fn from(label: SharedString) -> Self {
        Content::Label(label)
    }
}

impl From<IconName> for Content {
    fn from(icon: IconName) -> Self {
        Content::Icon(icon)
    }
}

/// Text shown when hovering a button, optionally with the shortcut of an
/// action.
pub struct Hint {
    text: SharedString,
    action: Option<Box<dyn Action>>,
}

impl Hint {
    pub fn text(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), action: None }
    }

    /// Hint text followed by the key binding of `action`.
    pub fn action(text: impl Into<SharedString>, action: &dyn Action) -> Self {
        Self { text: text.into(), action: Some(action.boxed_clone()) }
    }
}

/// A button that runs `on_click` when clicked.
pub fn button(
    id: impl Into<ElementId>,
    content: impl Into<Content>,
    state: ButtonState,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    styled(id.into(), content.into(), state, Corners::All, Palette::Normal, on_click)
}

/// A button that dispatches `action` when clicked, with `hint` (and the
/// action's shortcut) shown on hover.
pub fn action_button(
    content: impl Into<Content>,
    action: Box<dyn Action>,
    state: ButtonState,
    hint: impl Into<SharedString>,
) -> Stateful<Div> {
    action_button_in(content, action, state, hint, Palette::Normal)
}

fn action_button_in(
    content: impl Into<Content>,
    action: Box<dyn Action>,
    state: ButtonState,
    hint: impl Into<SharedString>,
    palette: Palette,
) -> Stateful<Div> {
    let id = ElementId::Name(format!("action-{}", action.name()).into());
    let tooltip = Hint::action(hint, action.as_ref());
    let on_click = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
        window.dispatch_action(action.boxed_clone(), cx);
    };
    with_hint(styled(id, content.into(), state, Corners::All, palette, on_click), tooltip)
}

/// The one prominent button of a toolbar.
pub fn primary_action_button(
    label: &'static str,
    action: Box<dyn Action>,
    state: ButtonState,
    hint: impl Into<SharedString>,
) -> Stateful<Div> {
    action_button_in(label, action, state, hint, Palette::Primary)
        .px_3()
        .font_weight(FontWeight::SEMIBOLD)
}

/// Adds a hover tooltip to a button.
pub fn with_hint(button: Stateful<Div>, hint: Hint) -> Stateful<Div> {
    let Hint { text, action } = hint;
    button.tooltip(move |window, cx| {
        let tooltip = Tooltip::new(text.clone());
        let tooltip = match &action {
            Some(action) => tooltip.action(action.as_ref(), None),
            None => tooltip,
        };
        tooltip.build(window, cx)
    })
}

/// What a button does when clicked.
pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// One button of a [`segmented`] group.
pub struct Segment {
    pub id: ElementId,
    pub content: Content,
    pub state: ButtonState,
    pub hint: Hint,
    pub on_click: ClickHandler,
}

impl Segment {
    pub fn new(
        id: impl Into<ElementId>,
        content: impl Into<Content>,
        state: ButtonState,
        hint: Hint,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), content: content.into(), state, hint, on_click: Box::new(on_click) }
    }

    /// A segment that dispatches `action`, showing its shortcut on hover.
    pub fn action(
        content: impl Into<Content>,
        action: Box<dyn Action>,
        state: ButtonState,
        hint: impl Into<SharedString>,
    ) -> Self {
        let id = ElementId::Name(format!("segment-{}", action.name()).into());
        let hint = Hint::action(hint, action.as_ref());
        Self::new(id, content, state, hint, move |_, window, cx| {
            window.dispatch_action(action.boxed_clone(), cx);
        })
    }
}

/// Buttons joined into one control: no gaps, outer corners rounded, and a
/// 1px divider between neighbors.
pub fn segmented(segments: Vec<Segment>) -> Div {
    let last = segments.len().saturating_sub(1);
    let mut group = div().flex().flex_row().flex_none().items_center();
    for (index, segment) in segments.into_iter().enumerate() {
        let corners = match (index == 0, index == last) {
            (true, true) => Corners::All,
            (true, false) => Corners::Left,
            (false, true) => Corners::Right,
            (false, false) => Corners::None,
        };
        if index > 0 {
            group = group.child(div().w(px(1.0)).h(px(16.0)).bg(theme::surface_bright()));
        }
        let Segment { id, content, state, hint, on_click } = segment;
        group = group
            .child(with_hint(styled(id, content, state, corners, Palette::Normal, on_click), hint));
    }
    group
}

/// A thin vertical divider for toolbars.
pub fn separator() -> Div {
    div().flex_none().w(px(1.0)).h(px(18.0)).mx_1().bg(theme::surface_bright())
}

fn content_element(content: Content) -> AnyElement {
    match content {
        Content::Label(label) => label.into_any_element(),
        Content::Icon(icon) => Icon::new(icon).with_size(px(ICON_SIZE)).into_any_element(),
        Content::AppIcon(path) => {
            Icon::empty().path(path).with_size(px(ICON_SIZE)).into_any_element()
        }
        Content::IconBadge(icon, badge) => div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(Icon::new(icon).with_size(px(ICON_SIZE)))
            .child(
                div().text_size(theme::meta_text_size()).font_weight(FontWeight::BOLD).child(badge),
            )
            .into_any_element(),
    }
}

/// Colors of an enabled button.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Palette {
    Normal,
    /// The one prominent button of a toolbar.
    Primary,
}

/// Builds a button in one of the three states. Every state keeps a 1px
/// border (transparent unless active) so toggling never shifts the layout.
fn styled(
    id: ElementId,
    content: Content,
    state: ButtonState,
    corners: Corners,
    palette: Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let icon_only = matches!(content, Content::Icon(_) | Content::AppIcon(_));
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(theme::control_height())
        .border_1()
        .border_color(transparent_black())
        .text_size(theme::ui_text_size())
        .whitespace_nowrap()
        .child(content_element(content));
    let base = if icon_only { base.w(px(ICON_BUTTON_WIDTH)) } else { base.px_2() };
    let radius = px(4.0);
    let base = match corners {
        Corners::All => base.rounded(radius),
        Corners::Left => base.rounded_l(radius),
        Corners::Right => base.rounded_r(radius),
        Corners::None => base,
    };
    // Each style (hover, active) may be set only once per element.
    match (state, palette) {
        (ButtonState::Disabled, _) => base.text_color(theme::muted()).cursor_default(),
        (ButtonState::Enabled | ButtonState::Active, Palette::Primary) => base
            .bg(theme::accent())
            .text_color(theme::crust())
            .cursor_pointer()
            .hover(|style| style.bg(theme::text()).text_color(theme::crust()))
            .active(|style| style.bg(theme::crust()))
            .on_click(on_click),
        (ButtonState::Enabled, Palette::Normal) => base
            .bg(theme::surface())
            .text_color(theme::subtext())
            .cursor_pointer()
            .hover(|style| style.bg(theme::surface_bright()).text_color(theme::text()))
            .active(|style| style.bg(theme::crust()))
            .on_click(on_click),
        (ButtonState::Active, Palette::Normal) => base
            .bg(theme::active_bg())
            .border_color(theme::accent())
            .text_color(theme::accent())
            .cursor_pointer()
            .hover(|style| style.bg(theme::accent_translucent()))
            .active(|style| style.bg(theme::crust()))
            .on_click(on_click),
    }
}
