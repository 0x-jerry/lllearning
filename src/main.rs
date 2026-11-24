use gpui_kit::component::{
    ActiveTheme as _, IconName,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::*;
use gpui_webview::WebView;

const HOME: &str = "https://gpui-kit.com/docs/webview";

struct WebViewDemo {
    focus_handle: FocusHandle,
    webview: Entity<WebView>,
    address: Entity<InputState>,
}

impl WebViewDemo {
    fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let webview = cx.new(|cx| {
            let builder = wry::WebViewBuilder::new();
            #[cfg(any(debug_assertions, feature = "inspector"))]
            let builder = builder.with_devtools(true);

            WebView::build(builder, window, cx).expect("Failed to create WebView")
        });
        webview.update(cx, |view, _| view.load_url(HOME));

        let address = cx.new(|cx| InputState::new(window, cx).default_value(HOME));

        cx.new(|cx| {
            cx.subscribe(
                &address,
                |this: &mut Self, input, event: &InputEvent, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        let url = input.read(cx).value();
                        this.webview.update(cx, |view, _| view.load_url(&url));
                    }
                },
            )
            .detach();

            Self {
                focus_handle: cx.focus_handle(),
                webview,
                address,
            }
        })
    }

    fn go_back(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.webview.update(cx, |view, _| view.back());
    }

    fn go_forward(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.webview.update(cx, |view, _| view.forward());
    }

    fn reload(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.webview.read(cx).raw().reload();
    }
}

impl Focusable for WebViewDemo {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WebViewDemo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .p_2()
            .gap_3()
            .size_full()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("back")
                            .ghost()
                            .icon(IconName::ChevronLeft)
                            .on_click(cx.listener(Self::go_back)),
                    )
                    .child(
                        Button::new("forward")
                            .ghost()
                            .icon(IconName::ChevronRight)
                            .on_click(cx.listener(Self::go_forward)),
                    )
                    .child(
                        Button::new("reload")
                            .ghost()
                            .icon(IconName::Redo)
                            .on_click(cx.listener(Self::reload)),
                    )
                    .child(Input::new(&self.address)),
            )
            .child(
                div()
                    .flex_1()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(self.webview.clone()),
            )
    }
}

fn main() {
    // Without this, the WebView2 child view does not render on Windows.
    #[cfg(target_os = "windows")]
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true");
    }

    // WebKitGTK only embeds into X11 windows, so Wayland sessions go through XWayland.
    #[cfg(target_os = "linux")]
    let app = gpui_kit::platform::linux(WindowingModes::X11);
    #[cfg(not(target_os = "linux"))]
    let app = gpui_kit::application();

    app.with_assets(gpui_kit::assets::Assets).run(|cx| {
        gpui_kit::init(cx);

        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            WebViewDemo::new(window, cx)
        })
        .expect("Failed to open window");
    });
}
