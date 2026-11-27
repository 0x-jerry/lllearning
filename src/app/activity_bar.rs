//! The vertical activity strip beside each side dock: one button per pane in
//! the slot, read from the registry. Pressing the pane on show hides the dock;
//! that decision is the shell's, so this only reports which id was pressed.

use gpui::component::{
    Selectable as _,
    button::{Button, ButtonVariants as _},
};
use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_rsx::rsx;

use crate::{
    icons,
    panes::{self, PaneSlot},
};

/// The strip's width, and the width the dock keeps when its pane is put away.
pub(crate) const STRIP_WIDTH: gpui::Pixels = px(36.);

/// What a strip button reports when it is pressed: the provider id.
type SelectHandler = std::rc::Rc<dyn Fn(&SharedString, &mut Window, &mut App) + 'static>;

/// The strip beside one dock.
pub struct ActivityBar {
    slot: PaneSlot,
    providers: Vec<std::rc::Rc<dyn panes::PaneProvider>>,
    /// `None` when the dock is collapsed, so no button claims to be on show.
    active: Option<String>,
    /// `Rc` because every button shares one handler.
    on_select: Option<SelectHandler>,
}

impl ActivityBar {
    pub fn new(
        slot: PaneSlot,
        active: Option<String>,
        providers: Vec<std::rc::Rc<dyn panes::PaneProvider>>,
    ) -> Self {
        Self {
            slot,
            providers,
            active,
            on_select: None,
        }
    }

    /// Called with the provider id when a pane's button is pressed.
    pub fn on_select(
        mut self,
        handler: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(std::rc::Rc::new(handler));
        self
    }

    /// The strip as an element.
    ///
    /// The theme values are passed in because the caller already holds an
    /// immutable borrow of the app while it builds the listeners.
    pub fn render(self, border: gpui::Hsla, sidebar: gpui::Hsla) -> gpui::AnyElement {
        let slot = self.slot;
        let providers = self.providers;
        let active = self.active.clone();
        let on_select = self.on_select;

        let buttons: Vec<gpui::AnyElement> = providers
            .into_iter()
            .map(|provider| {
                let id = provider.id();
                let title = provider.title();
                let is_active = active.as_deref().is_some_and(|current| current == id);
                let glyph = provider.icon();
                let id_for_click = SharedString::from(id.to_owned());
                let on_select = on_select.clone();

                let element_id = SharedString::from(format!("activity-{}-{id}", slot_name(slot)));

                // The lucide font is not an SVG, so the glyph is a child rather
                // than `Button::icon`. `class` sets the square because `small`
                // sizes a labelled button and its padding overflows 36px.
                rsx! {
                    <Button(element_id)
                        ghost
                        class="size-6 p-0"
                        tooltip={title}
                        selected={is_active}
                        onClick={move |_, window, cx| {
                            if let Some(handler) = &on_select {
                                handler(&id_for_click, window, cx);
                            }
                        }}
                    >
                        {icons::icon(glyph, 16.)}
                    </Button>
                }
                .into_any_element()
            })
            .collect();

        rsx! {
            <div
                id={SharedString::from(format!("activity-bar-{}", slot_name(slot)))}
                class="flex flex-col items-center gap-1 py-1"
                w={STRIP_WIDTH}
                h_full
                flex_none
                bg={sidebar}
                border_color={border}
                when={(slot == PaneSlot::Left, |el| el.border_r_1())}
                when={(slot == PaneSlot::Right, |el| el.border_l_1())}
            >
                {...buttons.into_iter()}
            </div>
        }
        .into_any_element()
    }
}

/// The name of a slot, for element ids.
fn slot_name(slot: PaneSlot) -> &'static str {
    match slot {
        PaneSlot::Left => "left",
        PaneSlot::Right => "right",
    }
}
