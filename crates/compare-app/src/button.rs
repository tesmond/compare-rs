//! Small reusable UI pieces.

use gpui::{
    Action, App, ClickEvent, Div, ElementId, InteractiveElement, ParentElement, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::theme;

/// Whether a button can be clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Enabled,
    Disabled,
    /// Enabled and drawn as switched on (for toggles).
    Active,
}

/// A toolbar button that dispatches `action` when clicked.
pub fn action_button(
    label: impl Into<SharedString>,
    action: Box<dyn Action>,
    state: ButtonState,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    let id = ElementId::Name(label.clone());
    button(id, label, state, move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
}

/// A button that runs `on_click` when clicked.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    state: ButtonState,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex_none()
        .text_size(px(11.0))
        .px_2()
        .py(px(3.0))
        .rounded(px(4.0))
        .text_color(theme::text())
        .whitespace_nowrap()
        .child(label.into());
    match state {
        ButtonState::Disabled => base.opacity(0.35).bg(theme::surface()),
        ButtonState::Enabled | ButtonState::Active => {
            let bg = if state == ButtonState::Active {
                theme::accent_translucent()
            } else {
                theme::surface()
            };
            base.bg(bg)
                .cursor_pointer()
                .hover(|style| style.bg(theme::surface_bright()))
                .on_click(on_click)
        }
    }
}

/// A thin vertical divider for toolbars.
pub fn separator() -> Div {
    div().flex_none().w(px(1.0)).h(px(18.0)).mx_1().bg(theme::surface_bright())
}
