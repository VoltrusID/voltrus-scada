//! Login screen: server URL + credentials → `UiCmd::Login`.
//! Persists URL + username (never the password) to config.toml.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use crate::runtime::{RuntimeHandle, UiCmd};

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Config {
    url: Option<String>,
    username: Option<String>,
}

fn config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("voltrus-native").join("config.toml"))
}

fn load_config() -> Config {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(url: &str, username: &str) {
    if let Some(p) = config_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let cfg = Config {
            url: Some(url.to_string()),
            username: Some(username.to_string()),
        };
        if let Ok(s) = toml::to_string_pretty(&cfg) {
            let _ = std::fs::write(p, s);
        }
    }
}

pub struct LoginView {
    url: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    error: Option<String>,
    busy: bool,
    handle: RuntimeHandle,
}

impl LoginView {
    pub fn new(handle: RuntimeHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cfg = load_config();
        let url = cx.new(|cx| {
            InputState::new(window, cx).default_value(
                cfg.url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:3000".to_string()),
            )
        });
        let username = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(cfg.username.clone().unwrap_or_else(|| "admin".to_string()))
        });
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));

        // Enter on the password field submits the form.
        cx.subscribe_in(&password, window, |this, _state, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit(window, cx);
            }
        })
        .detach();

        Self {
            url,
            username,
            password,
            error: None,
            busy: false,
            handle,
        }
    }

    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let url = format!("{}", self.url.read(cx).value());
        let username = format!("{}", self.username.read(cx).value());
        let password = format!("{}", self.password.read(cx).value());
        if url.trim().is_empty() || username.trim().is_empty() || password.is_empty() {
            self.error = Some("Server URL, username and password are required.".into());
            cx.notify();
            return;
        }
        save_config(url.trim(), username.trim());
        self.busy = true;
        self.error = None;
        cx.notify();
        self.handle.send(UiCmd::Login {
            url: url.trim().to_string(),
            username: username.trim().to_string(),
            password,
        });
    }

    pub fn login_failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.busy = false;
        self.error = Some(message);
        cx.notify();
    }

    fn render_field(
        &self,
        label: &str,
        state: &Entity<InputState>,
        masked: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(label.to_string()),
            )
            .child(Input::new(state).when(masked, |this| this.mask_toggle()))
    }
}

impl Render for LoginView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy;
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .w(px(380.))
                    .gap_4()
                    .p_6()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_lg()
                    .bg(cx.theme().secondary)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(cx.theme().foreground)
                                            .child("Voltrus"),
                                    )
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(cx.theme().green)
                                            .child("SCADA"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Native operator console — GPU accelerated (GPUI)"),
                            ),
                    )
                    .child(self.render_field("Server URL", &self.url, false, cx))
                    .child(self.render_field("Username", &self.username, false, cx))
                    .child(self.render_field("Password", &self.password, true, cx))
                    .when_some(self.error.clone(), |this, err| {
                        this.child(div().text_color(cx.theme().danger).child(err))
                    })
                    .child(
                        Button::new("login")
                            .primary()
                            .label(if busy { "Signing in…" } else { "Sign In" })
                            .on_click(cx.listener(|this, _ev, window, cx| {
                                this.submit(window, cx);
                            })),
                    )
                    .when(busy, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Connecting…"),
                        )
                    }),
            )
    }
}
