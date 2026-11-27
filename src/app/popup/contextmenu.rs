//! A context menu: commands beside the thing that was right-clicked. It is a
//! small frameless window of its own, placed at the pointer and owned by its
//! opener, because an in-window menu is covered by the browser pane.
//!
//! It closes itself when it loses focus, which needs no hit-testing.

use std::cell::Cell;
use std::rc::Rc;

use gpui::component::ActiveTheme as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Point, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Task, TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, div, px,
    size,
};
use gpui_rsx::rsx;

use super::{Host, Popup};
use crate::window;

/// One command in the menu.
pub struct MenuItem<T> {
    /// Element id, so the row can be named from outside the render.
    pub id: &'static str,
    pub label: SharedString,
    pub value: T,
    pub disabled: bool,
    /// Draws a separator above this item.
    pub separated: bool,
}

impl<T> MenuItem<T> {
    /// An enabled item.
    pub fn new(id: &'static str, label: impl Into<SharedString>, value: T) -> Self {
        Self {
            id,
            label: label.into(),
            value,
            disabled: false,
            separated: false,
        }
    }

    /// Grey this item out.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Draw a separator above this item.
    pub fn separated(mut self) -> Self {
        self.separated = true;
        self
    }
}

/// How the menu is placed and sized.
pub struct MenuOptions {
    /// Where the pointer was, in the owner's window coordinates.
    pub at: Point<Pixels>,
    pub width: Pixels,
}

impl MenuOptions {
    /// A menu at `at`, at the usual width.
    pub fn at(at: Point<Pixels>) -> Self {
        Self {
            at,
            width: px(180.),
        }
    }
}

/// Show `items` at `options.at` and resolve with the chosen one. `None` means it
/// was dismissed: a click elsewhere, or Escape.
///
pub fn open<T: Clone + 'static>(
    owner: &Window,
    cx: &mut App,
    options: MenuOptions,
    items: Vec<MenuItem<T>>,
) -> Task<Option<T>> {
    let MenuOptions { at, width } = options;
    let owner_native = window::owner_of(owner);
    // The owner's origin, so a point in its coordinates can be placed on screen.
    let origin = owner.bounds().origin;

    super::open(owner, cx, owner_native, false, move |host| ContextMenu {
        focus: None,
        items,
        host,
        _activation: None,
        origin,
        at,
        width,
    })
}

/// One menu window.
struct ContextMenu<T> {
    focus: Option<FocusHandle>,
    items: Vec<MenuItem<T>>,
    host: Host<T>,
    /// Dropping this unsubscribes the activation observer.
    _activation: Option<gpui::Subscription>,
    /// The owner's top-left on screen, so the menu lands under the pointer.
    origin: Point<Pixels>,
    at: Point<Pixels>,
    width: Pixels,
}

impl<T: Clone + 'static> ContextMenu<T> {
    /// The row's height, which sets the menu's.
    const ROW_HEIGHT: Pixels = px(28.);

    fn choose(&mut self, value: T, window: &mut Window) {
        self.host.answer(Some(value), window);
    }

    /// Leave without a choice.
    fn dismiss(&mut self, window: &mut Window) {
        self.host.answer(None, window);
    }

    fn height(&self) -> Pixels {
        let rows = self.items.len() as f32;
        let separators = self.items.iter().filter(|item| item.separated).count() as f32;
        Self::ROW_HEIGHT * rows + px(1.) * separators + px(8.)
    }
}

impl<T: Clone + 'static> Popup for ContextMenu<T> {
    type Answer = T;

    fn options(&self) -> WindowOptions {
        let size = size(self.width, self.height());
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui::Bounds::new(
                self.origin + self.at,
                size,
            ))),
            window_min_size: Some(size),
            // A tool window from birth: no taskbar button and no Alt-Tab entry.
            kind: WindowKind::PopUp,
            is_resizable: false,
            is_minimizable: false,
            is_movable: false,
            titlebar: Some(TitlebarOptions {
                title: None,
                appears_transparent: true,
                traffic_light_position: None,
            }),
            app_owns_titlebar_drag: true,
            ..Default::default()
        }
    }

    fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        self.focus = Some(focus.clone());
        // Losing the activation is the dismissal, as every desktop menu does.
        //
        // Armed from the state the window is already in: it is created shown and
        // active, so that activation may never be reported, and a menu that is
        // never armed never leaves.
        let armed = Rc::new(Cell::new(window.is_window_active()));
        self._activation = Some(cx.observe_window_activation(window, move |_, window, _| {
            if window.is_window_active() {
                armed.set(true);
            } else if armed.get() {
                window.remove_window();
            }
        }));
        window.focus(&focus, cx);
        window.activate_window();
    }
}

impl<T: Clone + 'static> Render for ContextMenu<T> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let background = theme.background;
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let hover = *theme.tokens.secondary_hover;
        let font_family = theme.font_family.clone();
        let font_size = theme.font_size;

        let rows: Vec<AnyElement> = self
            .items
            .iter()
            .map(|item| {
                let value = item.value.clone();
                let disabled = item.disabled;
                let label = item.label.clone();
                let color = if disabled { muted } else { foreground };
                rsx! {
                    <div class="flex flex-col w-full">
                        <div
                            when={(item.separated, |this| this.h(px(1.)).w_full().bg(border))}
                        />
                        <div
                            id={item.id}
                            class="flex flex-row items-center h-7 px-3 text-sm"
                            text_color={color}
                            on_click={cx.listener(move |this, _, window, _| {
                                if !disabled {
                                    this.choose(value.clone(), window)
                                }
                            })}
                            hover={move |style| style.bg(hover)}
                        >
                            {label}
                        </div>
                    </div>
                }
                .into_any_element()
            })
            .collect();

        let mut menu = rsx! {
            <div
                id="context-menu"
                class="flex flex-col size-full py-1 border-1"
                bg={background}
                text_color={foreground}
                font_family={font_family}
                text_size={font_size}
                border_color={border}
                on_action={cx.listener(|this, _: &super::dialog::DismissDialog, window, _| {
                    this.dismiss(window)
                })}
                children={rows}
            />
        };
        if let Some(focus) = self.focus.clone() {
            menu = menu.track_focus(&focus);
        }
        menu
    }
}
