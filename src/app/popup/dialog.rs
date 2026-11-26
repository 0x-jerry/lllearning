//! A prompt: a question with footer buttons.
//!
//! Its own window rather than `window.open_dialog`, because a dialog drawn into
//! the main window is covered by the browser pane's native child window (risk
//! R1, risk R17). [`open`] resolves with the action the user chose; `None` means
//! it was dismissed.
//!
//! The window has no OS caption. Its title row is drawn here and doubles as the
//! drag handle, and the window controls it offers are [`WindowButtons`] — a
//! prompt that must be answered can hide all three.

use gpui::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, Context, FocusHandle, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Pixels, Render, SharedString, Size, Styled as _, Task,
    TitlebarOptions, Window, WindowBounds, WindowControlArea, WindowKind, WindowOptions,
    WindowVisibility, div, px, size,
};
use gpui_rsx::rsx;

use super::{Host, Popup};
use crate::window;

gpui::actions!(lllearning, [DismissDialog, ConfirmDialog]);

/// Key context the prompt's root element claims, so Escape and Enter reach it
/// and nothing in the owner window can take them first.
const CONTEXT: &str = "DialogWindow";

/// Bind Escape and Enter for every prompt.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("escape", DismissDialog, Some(CONTEXT)),
        gpui::KeyBinding::new("enter", ConfirmDialog, Some(CONTEXT)),
    ]);
}

/// Which window controls the prompt's title row offers.
#[derive(Clone, Copy, Debug)]
pub struct WindowButtons {
    pub minimize: bool,
    pub maximize: bool,
    /// Draws the close button and makes Escape dismiss the prompt. Without it
    /// the prompt is a choice the user has to make with a footer button.
    pub close: bool,
}

impl WindowButtons {
    /// Minimize, maximize and close.
    pub const fn all() -> Self {
        Self {
            minimize: true,
            maximize: true,
            close: true,
        }
    }

    /// Just the close button.
    pub const fn close_only() -> Self {
        Self {
            minimize: false,
            maximize: false,
            close: true,
        }
    }

    /// No controls: the prompt is answered only through its footer buttons.
    ///
    /// Not used by the prompts this build ships, which all offer a close button.
    #[allow(dead_code)]
    pub const fn none() -> Self {
        Self {
            minimize: false,
            maximize: false,
            close: false,
        }
    }
}

impl Default for WindowButtons {
    fn default() -> Self {
        Self::all()
    }
}

/// How a prompt window is placed and behaves.
pub struct DialogOptions {
    pub title: SharedString,
    /// The body text. A prompt with none is just its buttons.
    pub message: Option<SharedString>,
    /// Refuse input to the owner while the prompt is up.
    pub modal: bool,
    /// Keep the prompt out of the taskbar and Alt-Tab, tied to its owner.
    pub owned: bool,
    pub buttons: WindowButtons,
    /// The window size; it opens centred on the owner.
    pub size: Size<Pixels>,
}

impl Default for DialogOptions {
    fn default() -> Self {
        Self {
            title: SharedString::default(),
            message: None,
            modal: true,
            owned: true,
            buttons: WindowButtons::default(),
            size: size(px(420.), px(180.)),
        }
    }
}

/// One footer button, and the answer it stands for.
pub struct DialogAction<T> {
    /// Element id, so a driver script and a test can name the button.
    pub id: &'static str,
    pub label: SharedString,
    /// Drawn as the default button, and what Enter activates.
    pub primary: bool,
    pub value: T,
}

/// Open a prompt owned by `owner` and resolve with the action the user chose.
///
/// See [`super::open`] for the calling rules: not from inside a draw, and keep
/// the returned task alive.
pub fn open<T: Clone + 'static>(
    owner: &Window,
    cx: &mut App,
    options: DialogOptions,
    actions: Vec<DialogAction<T>>,
) -> Task<Option<T>> {
    let DialogOptions {
        title,
        message,
        modal,
        owned,
        buttons,
        size,
    } = options;

    let owner_native = if owned { window::owner_of(owner) } else { None };
    let centre = owner.bounds().center();

    super::open(owner, cx, owner_native, modal, move |host| DialogView {
        focus: None,
        title,
        message,
        actions,
        buttons,
        host,
        centre,
        size,
        _visibility: None,
    })
}

/// A one-button message box, for reporting something the user has to see.
///
/// Deferred, so it is safe to call from inside an update — a failed save does.
pub fn alert(owner: &Window, cx: &mut App, title: &str, message: impl Into<SharedString>) {
    let owner = owner.window_handle();
    let title = SharedString::from(title.to_owned());
    let message = message.into();

    cx.defer(move |cx| {
        let _ = owner.update(cx, |_, window, cx| {
            let actions = vec![DialogAction {
                id: "ok",
                label: SharedString::from("OK"),
                primary: true,
                value: (),
            }];
            let options = DialogOptions {
                title,
                message: Some(message),
                buttons: WindowButtons::close_only(),
                ..Default::default()
            };
            open(window, cx, options, actions).detach();
        });
    });
}

/// One prompt window.
struct DialogView<T> {
    focus: Option<FocusHandle>,
    title: SharedString,
    message: Option<SharedString>,
    actions: Vec<DialogAction<T>>,
    buttons: WindowButtons,
    host: Host<T>,
    /// Where the owner was when the prompt opened.
    centre: gpui::Point<Pixels>,
    size: Size<Pixels>,
    /// Dropped with the view, so the visibility observer stops with it.
    _visibility: Option<gpui::Subscription>,
}

impl<T: Clone + 'static> DialogView<T> {
    /// Send `value` back and close the window.
    fn answer(&mut self, value: T, window: &mut Window) {
        self.host.answer(Some(value), window);
    }

    /// Close without an answer, which the caller reads as a dismissal.
    fn dismiss(&mut self, window: &mut Window) {
        self.host.answer(None, window);
    }

    /// Activate the default button, if there is one.
    fn confirm(&mut self, window: &mut Window) {
        if let Some(value) = primary_value(&self.actions) {
            self.answer(value, window);
        }
    }

    /// The title row: the drag handle, the title, then the window controls.
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let background = theme.title_bar;
        let muted = theme.muted_foreground;
        let title = self.title.clone();
        rsx! {
            <div class="flex flex-row items-center h-8 flex-none" bg={background}>
                <div
                    id="dialog-drag"
                    class="flex flex-1 min-w-0 h-full items-center px-3"
                    // On Windows the platform hit-tests this area and moves the
                    // window itself; `start_window_move` is the same gesture
                    // everywhere else.
                    window_control_area={WindowControlArea::Drag}
                    on_mouse_down={(MouseButton::Left, |_, window, _| window.start_window_move())}
                >
                    <div class="text-xs truncate" text_color={muted}>{title}</div>
                </div>
                {...self.buttons.minimize.then(|| {
                    rsx! {
                        <Button("dialog-minimize")
                            icon={IconName::WindowMinimize}
                            ghost
                            small
                            tooltip="Minimize"
                            onClick={|_, window, _| window.minimize_window()}
                        />
                    }
                    .into_any_element()
                })}
                {...self.buttons.maximize.then(|| {
                    rsx! {
                        <Button("dialog-maximize")
                            icon={IconName::WindowMaximize}
                            ghost
                            small
                            tooltip="Maximize"
                            onClick={|_, window, _| window.zoom_window()}
                        />
                    }
                    .into_any_element()
                })}
                {...self.buttons.close.then(|| {
                    rsx! {
                        <Button("dialog-close")
                            icon={IconName::WindowClose}
                            ghost
                            small
                            tooltip="Close"
                            onClick={cx.listener(|this, _, window, _| this.dismiss(window))}
                        />
                    }
                    .into_any_element()
                })}
            </div>
        }
        .into_any_element()
    }

    /// The body and the footer buttons.
    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = self.message.clone().map(|message| {
            rsx! { <div class="flex-1 min-h-0 px-4 py-3 text-sm">{message}</div> }
                .into_any_element()
        });

        let buttons = self
            .actions
            .iter()
            .map(|action| {
                let value = action.value.clone();
                let label = action.label.clone();
                let primary = action.primary;
                rsx! {
                    <Button
                        base={Button::new(action.id)}
                        label={label}
                        when={(primary, |this| this.primary())}
                        on_click={cx.listener(move |this, _, window, _| {
                            this.answer(value.clone(), window)
                        })}
                    />
                }
                .into_any_element()
            })
            .collect::<Vec<_>>();

        rsx! {
            <div class="flex flex-col flex-1 min-h-0">
                {...message.into_iter()}
                <div class="flex flex-row justify-end items-center gap-2 px-4 pb-3 flex-none">
                    {...buttons.into_iter()}
                </div>
            </div>
        }
        .into_any_element()
    }
}

impl<T: Clone + 'static> Popup for DialogView<T> {
    type Answer = T;

    fn options(&self) -> WindowOptions {
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered_at(
                self.centre,
                self.size,
            ))),
            window_min_size: None,
            // Windows takes its ownership from the Win32 call the host makes,
            // which can name the actual owner; everywhere else the platform's own
            // dialog kind does the right thing.
            kind: if cfg!(target_os = "windows") {
                WindowKind::Normal
            } else {
                WindowKind::Dialog
            },
            // A window that offers maximize has to be resizable for the platform
            // to honour it.
            is_resizable: self.buttons.maximize,
            is_minimizable: self.buttons.minimize,
            titlebar: Some(TitlebarOptions {
                title: Some(self.title.clone()),
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
        // A modal owner cannot be clicked, so a prompt minimized behind it would
        // lock the app: restoring the prompt is the only way back, and its
        // taskbar button may not be there. It therefore comes back with its
        // owner instead.
        let modal = self.host.modal();
        self._visibility = Some(
            window.observe_window_visibility(move |visibility, window, _| {
                if modal && visibility == WindowVisibility::Visible {
                    window.activate_window();
                }
            }),
        );
        window.focus(&focus, cx);
    }
}

/// The value Enter activates: the first action marked primary.
///
/// A prompt with none is answered only by a click, which is why the close button
/// and Escape exist — they are what a keyboard-only user gets instead.
fn primary_value<T: Clone>(actions: &[DialogAction<T>]) -> Option<T> {
    actions
        .iter()
        .find(|action| action.primary)
        .map(|action| action.value.clone())
}

impl<T: Clone + 'static> Render for DialogView<T> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let background = theme.background;
        let foreground = theme.foreground;
        let border = theme.border;
        let font_family = theme.font_family.clone();
        let font_size = theme.font_size;

        let header = self.header(cx);
        let body = self.body(cx);

        let mut root = rsx! {
            <div
                id="dialog-window"
                class="flex flex-col size-full border-1"
                bg={background}
                text_color={foreground}
                font_family={font_family}
                text_size={font_size}
                border_color={border}
                key_context={CONTEXT}
                on_action={cx.listener(|this, _: &DismissDialog, window, _| this.dismiss(window))}
                on_action={cx.listener(|this, _: &ConfirmDialog, window, _| this.confirm(window))}
            >
                {header}
                {body}
            </div>
        };
        if let Some(focus) = self.focus.clone() {
            root = root.track_focus(&focus);
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions() -> Vec<DialogAction<&'static str>> {
        vec![
            DialogAction {
                id: "cancel",
                label: SharedString::from("Cancel"),
                primary: false,
                value: "cancel",
            },
            DialogAction {
                id: "save",
                label: SharedString::from("Save"),
                primary: true,
                value: "save",
            },
        ]
    }

    #[test]
    fn enter_activates_the_primary_action_wherever_it_sits() {
        assert_eq!(primary_value(&actions()), Some("save"));
    }

    #[test]
    fn a_prompt_without_a_primary_action_has_no_default() {
        let plain = vec![DialogAction {
            id: "ok",
            label: SharedString::from("OK"),
            primary: false,
            value: (),
        }];
        assert_eq!(primary_value(&plain), None);
    }

    #[test]
    fn the_default_prompt_offers_all_three_controls() {
        let buttons = WindowButtons::default();
        assert!(buttons.minimize && buttons.maximize && buttons.close);
    }

    #[test]
    fn a_prompt_with_no_controls_can_only_be_answered_by_its_buttons() {
        let buttons = WindowButtons::none();
        assert!(!buttons.close && !buttons.minimize && !buttons.maximize);
    }
}
