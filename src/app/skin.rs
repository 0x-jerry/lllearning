//! The shell's dock appearance.
//!
//! `gpui_component::dock::DockSkin` cannot be reused as a base: its shared state
//! is crate-private, and a second `impl DockAreaRenderer for DockSkin` is barred
//! by the orphan rule. So this is a fresh renderer that draws the mockup's
//! chrome — flat frames, hairline dividers, an underline tab bar with a close
//! button per tab — using the public building blocks `DockSkin` itself uses.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use gpui::base::{
    HandleEdge, ResizeHandleContext,
    dock::{
        DockArea, DockAreaRenderer, DockContext, DockEvent, DockPlacement, NodeId, PanelId,
        PanelState, PanelView, TabGroupContext, TabGroupRenderer,
    },
};
use gpui::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    dock::PanelHandle,
    resizable::resize_handle_appearance,
    tab::{Tab, TabBar},
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Element, Empty, InteractiveElement as _,
    IntoElement, ParentElement as _, ScrollHandle, SharedString, Stateful, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_rsx::rsx;

/// The shell's dock appearance.
pub struct ShellSkin {
    shared: Rc<SkinShared>,
}

/// What every part of the skin reads.
struct SkinShared {
    /// The area, for the dock open/closed state a tab bar cannot reach on its
    /// own.
    area: gpui::WeakEntity<DockArea>,
    /// The dock whose handle is being dragged, so its mouse listeners know to
    /// act. Only one can be dragged at a time.
    resizing: Cell<Option<DockPlacement>>,
    /// Whether that dock was open when the drag began, so the release can tell
    /// whether the drag opened or closed it.
    resize_started_open: Cell<bool>,
    /// Asked before a tab closes. The shell answers `true` to close at once and
    /// `false` to keep the tab open because it opened its own prompt — which is
    /// how the dirty-close flow gets a say without the skin knowing what a
    /// document is.
    close_request: RefCell<Option<CloseRequest>>,
}

/// A callback that decides whether a tab may close.
type CloseRequest = Rc<dyn Fn(PanelId, &mut Window, &mut App) -> bool>;

impl SkinShared {
    fn area(&self) -> &gpui::WeakEntity<DockArea> {
        &self.area
    }

    /// Install the shell's close gate.
    fn set_close_request(&self, request: CloseRequest) {
        *self.close_request.borrow_mut() = Some(request);
    }
}

impl ShellSkin {
    /// Build a `DockArea` wearing this skin, plus the handle its settings are
    /// changed through.
    ///
    /// The skin needs the area's own weak handle, which exists only while the
    /// area is being constructed — the same dance `DockSkin::dock_area` does.
    pub fn dock_area(
        id: impl Into<SharedString>,
        version: Option<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::Entity<DockArea>, Rc<Self>) {
        let mut skin = None;
        let area = cx.new(|cx| {
            let this = Self::new(cx);
            skin = Some(this.clone());
            DockArea::new(id, version, window, cx).with_renderer(this)
        });
        (
            area,
            skin.expect("ShellSkin::new ran inside the constructor"),
        )
    }

    fn new(cx: &mut Context<DockArea>) -> Rc<Self> {
        Rc::new(Self {
            shared: Rc::new(SkinShared {
                area: cx.weak_entity(),
                resizing: Cell::new(None),
                resize_started_open: Cell::new(false),
                close_request: RefCell::new(None),
            }),
        })
    }

    /// Install the gate the shell uses to intercept a tab close.
    ///
    /// The skin cannot prompt by itself: a prompt needs to know whether the
    /// panel holds unsaved work, which is the shell's business.
    pub fn set_close_request(&self, request: CloseRequest) {
        self.shared.set_close_request(request);
    }
}

impl DockAreaRenderer for ShellSkin {
    fn frame(&self, _window: &mut Window, cx: &mut App) -> Stateful<Div> {
        div().id("shell-dock-area").bg(cx.theme().tokens.background)
    }

    fn center_frame(&self, _window: &mut Window, cx: &mut App) -> Stateful<Div> {
        div()
            .id("shell-dock-center")
            .bg(cx.theme().tokens.background)
    }

    fn split_frame(
        &self,
        node: NodeId,
        _axis: gpui::Axis,
        _window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        div()
            .id(("shell-split", node.as_u64()))
            .bg(cx.theme().tokens.background)
    }

    fn render_split_handle(
        &self,
        handle: &ResizeHandleContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        // The design system's own divider: a hairline at rest that answers the
        // pointer with a pill. Reusing it keeps the shell's dividers identical
        // to every other resizable in the app.
        (resize_handle_appearance())(handle, window, cx)
    }

    fn closed_dock_extent(&self, _placement: DockPlacement) -> gpui::Pixels {
        // Side docks disappear completely when closed: the activity strip is
        // what reopens them, so a residual strip would be dead space.
        px(0.)
    }

    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let placement = dock.placement();
        let physical = placement.physical(gpui::base::LayoutDirection::global(cx));
        let open = dock.is_open();
        let shared = self.shared.clone();

        // A dock's own resize handle. It lives inside the dock because the
        // dock's box clips its children, so it hugs the edge that faces the
        // centre rather than straddling it.
        let edge = match physical {
            DockPlacement::Left => HandleEdge::Trailing,
            _ => HandleEdge::Leading,
        };
        let handle_id = match placement {
            DockPlacement::Left => "shell-dock-resize-left",
            DockPlacement::Right => "shell-dock-resize-right",
            DockPlacement::Bottom => "shell-dock-resize-bottom",
            DockPlacement::Center => "shell-dock-resize-center",
        };

        let shared_for_drag = shared.clone();
        let handle = gpui::component::resizable::resize_handle::<ResizeDrag, Empty>(
            handle_id,
            placement.axis(),
        )
        .inside(edge)
        .on_drag(ResizeDrag, move |_, _, _, cx| {
            cx.stop_propagation();
            shared_for_drag.resizing.set(Some(placement));
            shared_for_drag.resize_started_open.set(open);
            cx.new(|_| Empty)
        });

        let left = physical == DockPlacement::Left;
        let right = physical == DockPlacement::Right;

        rsx! {
            <div class="flex size-full relative"
                bg={cx.theme().tokens.sidebar}
                border_color={cx.theme().border}
                when={(left, |el| el.border_r_1())}
                when={(right, |el| el.border_l_1())}
            >
                {content}
                {handle}
                {ResizeTracker { dock: dock.clone(), shared }}
            </div>
        }
        .into_any_element()
    }

    fn build_placeholder(
        &self,
        state: &PanelState,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn PanelView>> {
        // An unknown panel keeps its persisted state so a layout written by a
        // build with more panes survives a round trip here.
        let state = state.clone();
        Some(gpui::component::dock::panel_handle(
            cx.new(|cx| UnknownPane::new(state, cx)),
        ))
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(ShellTabs::new(self.shared.clone()))
    }
}

/// The payload the dock's resize handle drags. It draws nothing; the handle is
/// the affordance.
#[derive(Clone)]
struct ResizeDrag;

/// Turns the window's mouse stream into dock resizing.
///
/// A resize follows the pointer anywhere in the window, so it cannot be a
/// listener on the handle itself. This element paints nothing and exists for its
/// `paint` hook, the only place a window-level listener can be registered.
struct ResizeTracker {
    dock: DockContext,
    shared: Rc<SkinShared>,
}

impl Element for ResizeTracker {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        (window.request_layout(gpui::Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        let placement = self.dock.placement();

        window.on_mouse_event({
            let dock = self.dock.clone();
            let shared = self.shared.clone();
            move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                if !phase.bubble() || shared.resizing.get() != Some(placement) {
                    return;
                }
                // The dock opens, closes and sizes itself from the pointer, so
                // a drag that started on a closed dock reopens it too.
                dock.resize_to(event.position, window, cx);
            }
        });

        window.on_mouse_event({
            let dock = self.dock.clone();
            let shared = self.shared.clone();
            move |_: &gpui::MouseUpEvent, phase, window, cx| {
                if !phase.bubble() || shared.resizing.get() != Some(placement) {
                    return;
                }
                shared.resizing.set(None);
                dock.end_resize(window, cx);
                // The size lives on the dock, not in the layout tree, so
                // nothing else tells a subscriber to persist it.
                let _ = shared.area().update(cx, |_, cx| {
                    cx.emit(DockEvent::LayoutChanged);
                });
            }
        });
    }
}

/// The shell's tab appearance: an underline bar with a close button per tab.
struct ShellTabs {
    shared: Rc<SkinShared>,
    scroll: ScrollHandle,
}

impl ShellTabs {
    fn new(shared: Rc<SkinShared>) -> Self {
        Self {
            shared,
            scroll: ScrollHandle::new(),
        }
    }
}

impl TabGroupRenderer for ShellTabs {
    fn frame(&self, _group: &TabGroupContext, _window: &mut Window, cx: &mut App) -> Stateful<Div> {
        div().id("shell-tab-group").bg(cx.theme().tokens.background)
    }

    fn content_frame(
        &self,
        group: &TabGroupContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        div()
            .id("shell-tab-content")
            .bg(cx.theme().tokens.background)
            .when(group.panels().len() > 1, |this| this.pt_1())
    }

    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        // A lone panel that carries its own chrome draws no tab bar: the
        // explorer and the browser both have their own toolbars. A document is
        // the exception — its tab is where its close button and dirty marker
        // live, so it asks for a title bar even when it is alone.
        if group.panels().len() == 1 {
            let panel = &group.panels()[0];
            let wants_title = PanelHandle::of(panel).is_none_or(|handle| handle.title_bar(cx));
            if !wants_title {
                return Empty.into_any_element();
            }
        }

        let active = group.active_ix();
        let mut tabs: Vec<Tab> = Vec::new();
        for (ix, panel) in group.panels().iter().enumerate() {
            let Some(handle) = PanelHandle::of(panel) else {
                // A panel handed to base without a handle has no presentation
                // to read; its name is the best available label.
                tabs.push(
                    Tab::new()
                        .label(SharedString::from(panel.panel_name(cx)))
                        .selected(ix == active),
                );
                continue;
            };

            let panel_id = handle.panel_id(cx);
            let title = handle
                .tab_name(cx)
                .unwrap_or_else(|| SharedString::from(panel.panel_name(cx)));
            // The dock's own gate answers "may this tab be *dragged out*", which
            // is false for a lone panel — and a document opened on its own is
            // exactly that. So the close button is offered whenever the panel
            // itself says it may close, and the shell gets the final say
            // through the close gate.
            let closable = handle.closable(cx) && !group.is_collapsed();
            let _ = group.is_panel_closable(panel_id, cx);
            let group_for_select = group.clone();
            let group_for_close = group.clone();
            let theme = cx.theme();
            let foreground = theme.secondary_foreground;
            let hover = *theme.tokens.secondary_hover;
            let active_bg = *theme.tokens.secondary_active;

            let close_button = Button::new(SharedString::from(format!("shell-close-tab-{ix}")))
                .icon(IconName::Close)
                .xsmall()
                .custom(
                    ButtonCustomVariant::new(cx)
                        .foreground(foreground)
                        .hover(hover)
                        .active(active_bg),
                )
                .tab_stop(false)
                .on_click({
                    let close_request = self.shared.close_request.borrow().clone();
                    move |_, window, cx| {
                        // Closing must not also select the tab.
                        cx.stop_propagation();
                        // The shell gets first refusal: it may need to prompt
                        // for unsaved work, in which case the tab stays open
                        // until the prompt is answered.
                        let allowed = match &close_request {
                            Some(request) => request(panel_id, window, cx),
                            None => true,
                        };
                        if allowed {
                            group_for_close.close(panel_id, window, cx);
                        }
                    }
                });

            tabs.push(
                Tab::new()
                    .label(title)
                    .selected(ix == active)
                    .when(closable, |this| this.suffix(close_button))
                    .on_click(move |_, window, cx| {
                        group_for_select.select_tab(ix, window, cx);
                    }),
            );
        }

        let trailing = self.render_trailing(cx);
        let _ = &self.shared;

        TabBar::new("shell-tabs")
            .underline()
            .small()
            .menu(true)
            .track_scroll(&self.scroll)
            .selected_index(active)
            .children(tabs)
            .last_empty_space(
                div()
                    .id("shell-tabs-empty")
                    .h_full()
                    .flex_grow_1()
                    .min_w(px(8.)),
            )
            .when_some(trailing, |this, trailing| this.suffix(trailing))
            .into_any_element()
    }

    fn render_empty(
        &self,
        _group: &TabGroupContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        // The centre with no tabs is the app's start screen, not a blank area.
        // Side docks that lose their last pane fall back to this too, which is
        // the right answer there as well: the strip is how you get one back.
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;

        Some(
            rsx! {
                <div class="flex flex-col size-full items-center justify-center gap-2 p-6">
                    {Icon::new(IconName::FolderOpen).large().text_color(muted)}
                    <div class="text-sm" text_color={foreground}>
                        {"No file open"}
                    </div>
                    <div class="text-xs" text_color={muted}>
                        {"Pick a file in the explorer to start."}
                    </div>
                </div>
            }
            .into_any_element(),
        )
    }

    fn render_active_panel(
        &self,
        panel: gpui::AnyView,
        _group: &TabGroupContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        panel.into_any_element()
    }
}

impl ShellTabs {
    /// The trailing controls on the tab bar: the dock's collapse buttons.
    ///
    /// A tab bar cannot read the dock's open state, so this goes through the
    /// area handle the skin keeps.
    fn render_trailing(&self, cx: &mut App) -> Option<AnyElement> {
        let area = self.shared.area().upgrade()?;
        let area = area.read(cx);

        let mut buttons: Vec<AnyElement> = Vec::new();
        for placement in [DockPlacement::Left, DockPlacement::Right] {
            if !area.has_dock(placement) || !area.is_dock_collapsible(placement) {
                continue;
            }
            let open = area.is_dock_open(placement);
            let icon = match (placement, open) {
                (DockPlacement::Left, true) => IconName::PanelLeftClose,
                (DockPlacement::Left, false) => IconName::PanelLeftOpen,
                (_, true) => IconName::PanelRightClose,
                (_, false) => IconName::PanelRightOpen,
            };
            let weak = self.shared.area().clone();
            buttons.push(
                Button::new(SharedString::from(format!(
                    "shell-dock-toggle-{}",
                    placement as usize
                )))
                .icon(icon)
                .xsmall()
                .ghost()
                .tab_stop(false)
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |area, cx| {
                        area.toggle_dock(placement, window, cx);
                    });
                })
                .into_any_element(),
            );
        }

        if buttons.is_empty() {
            return None;
        }
        Some(
            rsx! { <div class="flex items-center gap-1 px-1">{...buttons.into_iter()}</div> }
                .into_any_element(),
        )
    }
}

/// The stand-in for a panel this build does not know.
struct UnknownPane {
    state: PanelState,
    focus_handle: gpui::FocusHandle,
}

impl UnknownPane {
    fn new(state: PanelState, cx: &mut Context<Self>) -> Self {
        Self {
            state,
            focus_handle: cx.focus_handle(),
        }
    }
}

impl gpui::EventEmitter<gpui::base::dock::PanelEvent> for UnknownPane {}

impl gpui::Focusable for UnknownPane {
    fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui::Render for UnknownPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = SharedString::from(self.state.panel_name.clone());
        let danger = cx.theme().danger;

        rsx! {
            <div class="flex flex-col size-full items-center justify-center gap-2">
                {Icon::new(IconName::TriangleAlert).large().text_color(danger)}
                <div class="text-sm">
                    {SharedString::from(format!(
                        "{} is not available in this build.",
                        name
                    ))}
                </div>
            </div>
        }
    }
}

impl gpui::base::dock::Panel for UnknownPane {
    fn panel_name(&self) -> &'static str {
        // The persisted name must come back unchanged, or a later save would
        // rewrite it and the panel would be lost for good. The dock's trait
        // wants `'static`, so the name is interned once per panel; there are at
        // most a handful of these in a layout.
        interned_name(&self.state.panel_name)
    }

    fn dump(&self, _cx: &App) -> PanelState {
        self.state.clone()
    }
}

/// Intern a panel name so the dock's `&'static str` requirement is met without
/// leaking a new string on every call.
fn interned_name(name: &str) -> &'static str {
    use std::{
        collections::HashSet,
        sync::{Mutex, OnceLock},
    };
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let names = NAMES.get_or_init(|| Mutex::new(HashSet::new()));
    let mut names = names.lock().expect("the name set is not poisoned");
    if let Some(existing) = names.get(name) {
        return existing;
    }
    // Leaked on purpose: a panel name lives as long as the process, and the set
    // bounds the leak to the distinct names this build ever sees.
    let interned: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(interned);
    interned
}

impl gpui::component::dock::Panel for UnknownPane {
    fn tab_name(&self, _cx: &App) -> Option<SharedString> {
        Some(SharedString::from(self.state.panel_name.clone()))
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from(self.state.panel_name.clone())
    }
}
