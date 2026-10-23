//! voltrus-native — GPUI operator console for the voltrus SCADA server.

use gpui::*;
use gpui_component::{ActiveTheme, Root, Theme, ThemeMode};

use voltrus_scada_native::runtime::{RtEvent, RuntimeHandle};
use voltrus_scada_native::views::console::Console;
use voltrus_scada_native::views::login::LoginView;

actions!(voltrus, [Quit]);

struct VoltrusApp {
    login: Entity<LoginView>,
    console: Entity<Console>,
    authed: bool,
    #[allow(dead_code)] // keeps the runtime thread alive for the app lifetime
    handle: RuntimeHandle,
}

impl VoltrusApp {
    fn new(
        handle: RuntimeHandle,
        events: tokio::sync::mpsc::UnboundedReceiver<RtEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let login = cx.new(|cx| LoginView::new(handle.clone(), window, cx));
        let console = cx.new(|cx| Console::new(handle.clone(), window, cx));

        // Pump runtime events into the view tree.
        cx.spawn(async move |this, cx| {
            let mut rx = events;
            while let Some(ev) = rx.recv().await {
                let Ok(()) = this.update(cx, |app, cx| app.on_event(ev, cx)) else {
                    break;
                };
            }
        })
        .detach();

        Self {
            login,
            console,
            authed: false,
            handle,
        }
    }

    fn on_event(&mut self, ev: RtEvent, cx: &mut Context<Self>) {
        match ev {
            RtEvent::LoginResult(Ok(login)) => {
                self.authed = true;
                let user = login.username.clone();
                self.console.update(cx, |c, cx| c.set_user(&user, cx));
            }
            RtEvent::LoginResult(Err(e)) => {
                self.login.update(cx, |v, cx| v.login_failed(e, cx));
            }
            RtEvent::Conn(state) => self.console.update(cx, |c, cx| c.on_conn(state, cx)),
            RtEvent::Sensors(sensors) => {
                self.console.update(cx, |c, cx| c.on_sensors(sensors, cx));
            }
            RtEvent::Tags(updates) => {
                self.console.update(cx, |c, cx| c.on_tags(updates, cx));
            }
            RtEvent::Alarm(ev) => self.console.update(cx, |c, cx| c.on_alarm(ev, cx)),
            RtEvent::Alarms(alarms) => {
                self.console.update(cx, |c, cx| c.on_alarms(alarms, cx));
            }
            RtEvent::AckResult(_) => {}
        }
        cx.notify();
    }
}

impl Render for VoltrusApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if self.authed {
            self.console.clone().into_any_element()
        } else {
            self.login.clone().into_any_element()
        };
        div().size_full().bg(cx.theme().background).child(content)
    }
}

fn main() {
    let (handle, events) = voltrus_scada_native::runtime::start();

    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);

            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            let handle = handle.clone();
            cx.spawn(async move |cx| {
                let opts = WindowOptions {
                    window_bounds: Some(
                        cx.update(|cx| WindowBounds::centered(size(px(1240.), px(780.)), cx))
                            .expect("app context"),
                    ),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Voltrus SCADA — Native Console".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                cx.open_window(opts, |window, cx| {
                    window.activate_window();
                    Theme::change(ThemeMode::Dark, Some(window), cx);
                    // Brand overrides must come after Theme::change (it resets colors).
                    voltrus_scada_native::theme::apply(cx);
                    let app = cx.new(|cx| VoltrusApp::new(handle.clone(), events, window, cx));
                    // gpui-component requires its Root as the window's root view.
                    cx.new(|cx| Root::new(app, window, cx))
                })
                .expect("failed to open window");
            })
            .detach();
        });
}
