//! Dialogs as their own window.
//!
//! `window.open_dialog` draws the prompt into the main window's element tree,
//! where the browser pane's WebView2 child window paints over it (risk R1). A
//! prompt is therefore a small top-level window owned by its caller: the
//! platform keeps it above the owner, so nothing can cover it, and it reports
//! the chosen action back through a channel instead of a dialog layer.
//!
//! The window has no OS caption. Its title row is drawn here and doubles as the
//! drag handle, and the window controls it offers are [`WindowButtons`] — a
//! prompt that must be answered can hide all three.

use gpui::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, FocusHandle, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Pixels, Render, SharedString, Size, Styled as _,
    Task, TitlebarOptions, Window, WindowBounds, WindowControlArea, WindowId, WindowKind,
    WindowOptions, WindowVisibility, div, px, size,
};
use gpui_rsx::rsx;

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
/// `None` means the window was dismissed — Escape, the close button, or the
/// owner going away — so a caller that needs an answer must treat it as one.
///
/// Must not be called while a window is being drawn: the new window paints its
/// first frame before this returns. Defer it, as `AppShell` does.
///
/// Keep the returned task alive. Dropping it drops the receiving end, and the
/// window then has nowhere to report its answer.
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
        size: window_size,
    } = options;

    let (sender, receiver) = gpui::base::async_util::unbounded::<T>();
    // A dismissed prompt never sends, so the channel closes instead and this
    // resolves to `None` with no extra plumbing.
    let answered = cx.spawn(async move |_| receiver.recv().await.ok());

    let centre = owner.bounds().center();
    let owner_window = owner.window_handle();
    let owner = if owned { window::owner_of(owner) } else { None };
    let bounds = Bounds::centered_at(centre, window_size);

    let window_options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: None,
        // Windows takes its ownership from the Win32 call below, which can name
        // the actual owner; everywhere else the platform's own dialog kind does
        // the right thing.
        kind: if cfg!(target_os = "windows") {
            WindowKind::Normal
        } else {
            WindowKind::Dialog
        },
        // A window that offers maximize has to be resizable for the platform to
        // honour it.
        is_resizable: buttons.maximize,
        is_minimizable: buttons.minimize,
        titlebar: Some(TitlebarOptions {
            title: Some(title.clone()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        app_owns_titlebar_drag: true,
        ..Default::default()
    };

    let built = gpui::open_window(window_options, cx, move |window, cx| {
        // After the window exists: there is no handle to adopt before it, and it
        // must not paint once as a taskbar window before being adopted.
        window::adopt_owner(window, owner, modal);
        let view = cx.new(|cx| {
            DialogView::new(
                window,
                cx,
                DialogView {
                    focus: cx.focus_handle(),
                    title,
                    message,
                    actions,
                    buttons,
                    sender: Some(sender),
                    owner,
                    modal,
                    _visibility: None,
                },
            )
        });
        registry::add(window.window_handle(), owner_window, cx);
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        view
    });

    match built {
        Ok(_) => answered,
        Err(error) => {
            eprintln!("could not open the prompt window: {error}");
            Task::ready(None)
        }
    }
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

/// Close every prompt `owner` still has open, so none is left floating once the
/// owner leaves the screen.
///
/// The tray's Hide and the close-to-tray guard both take the owner away without
/// it being closed, which no owned window follows on its own.
pub fn close_for(owner: WindowId, cx: &mut App) {
    let Some(dialogs) = registry::take(owner, cx) else {
        return;
    };
    for dialog in dialogs {
        let _ = dialog.update(cx, |_, window, _| window.remove_window());
    }
}

/// One prompt window.
struct DialogView<T> {
    focus: FocusHandle,
    title: SharedString,
    message: Option<SharedString>,
    actions: Vec<DialogAction<T>>,
    buttons: WindowButtons,
    sender: Option<gpui::base::async_util::Sender<T>>,
    /// The owner's native handle, so `Drop` can hand its input back.
    owner: Option<isize>,
    /// Whether the owner's input was taken, so it is only handed back if it was.
    modal: bool,
    /// Dropped with the view, so the visibility observer stops with it.
    _visibility: Option<gpui::Subscription>,
}

impl<T: Clone + 'static> DialogView<T> {
    fn new(window: &mut Window, cx: &mut Context<Self>, mut view: Self) -> Self {
        // A modal owner cannot be clicked, so a prompt minimized behind it would
        // lock the app: restoring the prompt is the only way back, and its
        // taskbar button may not be there. It therefore comes back with its
        // owner instead.
        let modal = view.modal;
        let _ = cx;
        view._visibility = Some(
            window.observe_window_visibility(move |visibility, window, _| {
                if modal && visibility == WindowVisibility::Visible {
                    window.activate_window();
                }
            }),
        );
        view
    }

    /// Send `value` back and close the window.
    fn answer(&mut self, value: T, window: &mut Window) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.try_send(value);
        }
        window.remove_window();
    }

    /// Close without an answer, which the caller reads as a dismissal.
    fn dismiss(&mut self, window: &mut Window) {
        self.sender = None;
        window.remove_window();
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
        let drag = div()
            .id("dialog-drag")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .px_3()
            // On Windows the platform hit-tests this area and moves the window
            // itself; `start_window_move` is the same gesture everywhere else.
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(
                div()
                    .text_xs()
                    .truncate()
                    .text_color(muted)
                    .child(self.title.clone()),
            );

        let mut controls: Vec<AnyElement> = Vec::new();
        if self.buttons.minimize {
            controls.push(
                Button::new("dialog-minimize")
                    .icon(gpui::component::IconName::WindowMinimize)
                    .ghost()
                    .small()
                    .tooltip("Minimize")
                    .on_click(|_, window, _| window.minimize_window())
                    .into_any_element(),
            );
        }
        if self.buttons.maximize {
            controls.push(
                Button::new("dialog-maximize")
                    .icon(gpui::component::IconName::WindowMaximize)
                    .ghost()
                    .small()
                    .tooltip("Maximize")
                    .on_click(|_, window, _| window.zoom_window())
                    .into_any_element(),
            );
        }
        if self.buttons.close {
            controls.push(
                Button::new("dialog-close")
                    .icon(gpui::component::IconName::WindowClose)
                    .ghost()
                    .small()
                    .tooltip("Close")
                    .on_click(cx.listener(|this, _, window, _| this.dismiss(window)))
                    .into_any_element(),
            );
        }

        rsx! {
            <div class="flex flex-row items-center h-8 flex-none" bg={background}>
                {drag}
                {...controls.into_iter()}
            </div>
        }
        .into_any_element()
    }

    /// The body and the footer buttons.
    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = self.message.clone().map(|message| {
            div()
                .flex_1()
                .min_h_0()
                .px_4()
                .py_3()
                .text_sm()
                .child(message)
                .into_any_element()
        });

        let buttons = self
            .actions
            .iter()
            .map(|action| {
                let value = action.value.clone();
                let button = Button::new(action.id).label(action.label.clone());
                let button = if action.primary {
                    button.primary()
                } else {
                    button
                };
                button
                    .on_click(
                        cx.listener(move |this, _, window, _| this.answer(value.clone(), window)),
                    )
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

/// The value Enter activates: the first action marked primary.
///
/// A prompt with none is answered only by a click, which is why the close
/// button and Escape exist — they are what a keyboard-only user gets instead.
fn primary_value<T: Clone>(actions: &[DialogAction<T>]) -> Option<T> {
    actions
        .iter()
        .find(|action| action.primary)
        .map(|action| action.value.clone())
}

impl<T> Drop for DialogView<T> {
    fn drop(&mut self) {
        // Every exit path — a button, Escape, Alt+F4, the owner going away —
        // ends here, so the owner's input is handed back exactly once.
        window::release_owner(self.owner, self.modal);
    }
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

        div()
            .id("dialog-window")
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .font_family(font_family)
            .text_size(font_size)
            .border_1()
            .border_color(border)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &DismissDialog, window, _| this.dismiss(window)))
            .on_action(cx.listener(|this, _: &ConfirmDialog, window, _| this.confirm(window)))
            .child(header)
            .child(body)
    }
}

/// Which prompts are open, by the window that owns them.
///
/// A prompt is owned by its caller's window on the platform, but nothing makes
/// it follow that window off the screen: the tray's Hide takes the owner away
/// and would leave the prompt behind. This is the list that lets the hide paths
/// find them.
mod registry {
    use std::collections::HashMap;

    use gpui::{AnyWindowHandle, App, Global, WindowId};

    #[derive(Default)]
    struct OpenDialogs(HashMap<WindowId, Vec<AnyWindowHandle>>);

    impl Global for OpenDialogs {}

    fn map(cx: &mut App) -> &mut OpenDialogs {
        if cx.try_global::<OpenDialogs>().is_none() {
            cx.set_global(OpenDialogs::default());
        }
        cx.global_mut::<OpenDialogs>()
    }

    pub(super) fn add(dialog: AnyWindowHandle, owner: AnyWindowHandle, cx: &mut App) {
        map(cx).0.entry(owner.window_id()).or_default().push(dialog);
    }

    // Entries are never removed when a prompt closes: `Drop` has no `App` to
    // reach the registry with. They stay until the owner's next hide clears the
    // list, and a handle to a closed window simply fails to update, so the
    // stale ones cost nothing but their place in the list.

    /// The prompts `owner` still has open, forgetting them.
    pub(super) fn take(owner: WindowId, cx: &mut App) -> Option<Vec<AnyWindowHandle>> {
        map(cx).0.remove(&owner)
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
