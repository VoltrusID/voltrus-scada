//! The edition and the sign-in methods: the navbar's tier badge, the
//! sign-in card's edition line and its list of sign-in methods
//! (`/api/license/status`, `/api/v1/auth/providers`, both public).

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, SharedString, div};
use voltrus_scada_native::api::shell::{SignInMethod, Tier};
use vt_ui::Icon;
use vt_ui::components::{Button, Tooltip, pill};
use vt_ui::theme::*;

use super::host;
use crate::console::Console;
use crate::console::shell::Shell;

fn tier_color(tier: Tier, theme: &Theme) -> gpui::Hsla {
    match tier {
        Tier::Community => theme.fg_muted,
        Tier::Integrator => theme.info,
        Tier::Enterprise => theme.accent,
    }
}

/// The navbar's tier badge's text, once the server said (for the navbar's
/// fit).
pub fn badge_label(console: &mut Console) -> Option<&'static str> {
    let license = console.state::<Shell>().license.as_ref()?;
    Some(license.tier().label())
}

/// The navbar's tier badge, once the server said.
pub fn badge(console: &mut Console, cx: &mut Context<Console>) -> Option<AnyElement> {
    let theme = theme(cx).clone();
    let license = console.state::<Shell>().license.clone()?;
    let tier = license.tier();
    let tip = SharedString::from(match license.note() {
        Some(note) => format!("{} edition · {note}", tier.label()),
        None => format!("{} edition", tier.label()),
    });
    Some(
        div()
            .id("edition")
            .flex_none()
            .tooltip(move |_, cx| Tooltip::view(tip.clone(), cx))
            .child(pill(tier.label(), tier_color(tier, &theme)))
            .into_any_element(),
    )
}

/// The sign-in card's edition line, under the product name.
pub fn sign_in_edition(console: &mut Console, cx: &mut Context<Console>) -> Option<AnyElement> {
    let theme = theme(cx).clone();
    let license = console.state::<Shell>().license.clone()?;
    let tier = license.tier();
    Some(
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .text_size(FONT_SMALL)
            .text_color(theme.fg_muted)
            .child(pill(tier.label(), tier_color(tier, &theme)))
            .child(license.note().unwrap_or_else(|| "edition".to_owned()))
            .into_any_element(),
    )
}

/// The sign-in card's methods: the password, and any single sign-on the
/// server lists, each with why this console can or cannot use it.
pub fn sign_in_methods(console: &mut Console, cx: &mut Context<Console>) -> Option<AnyElement> {
    let theme = theme(cx).clone();
    let url = console
        .inputs
        .url
        .read(cx)
        .value()
        .trim()
        .trim_end_matches('/')
        .to_owned();
    let shell = console.state::<Shell>();
    if let Some(error) = shell.providers_error.clone() {
        return Some(
            div()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .truncate()
                .child(format!("{} did not answer: {error}", host(&url)))
                .into_any_element(),
        );
    }
    let providers = shell.providers.clone()?;
    let password = providers
        .providers
        .iter()
        .any(|provider| provider.method() == SignInMethod::Password);
    let others: Vec<_> = providers.others().cloned().collect();
    let chip = |label: SharedString, usable: bool| {
        div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .px(SPACE_3)
            .py(SPACE_1)
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(theme.border)
            .text_color(if usable { theme.fg } else { theme.fg_dim })
            .child(vt_ui::icon(
                if usable { Icon::Check } else { Icon::Lock },
                ICON_SM,
                if usable { theme.success } else { theme.fg_dim },
            ))
            .child(label)
    };
    let mut block =
        div()
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .text_size(FONT_SMALL)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(SPACE_3)
                    .child(
                        div()
                            .text_color(theme.fg_muted)
                            .font_weight(FontWeight::MEDIUM)
                            .child("Sign-in methods"),
                    )
                    .when(password, |this| this.child(chip("Password".into(), true)))
                    .children(others.iter().map(|provider| {
                        chip(SharedString::from(provider.name().to_owned()), false)
                    }))
                    .when(others.is_empty(), |this| {
                        this.child(div().text_color(theme.fg_dim).child("no single sign-on"))
                    }),
            );
    for provider in &others {
        if let SignInMethod::Unavailable(why) = provider.method() {
            block = block.child(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child(format!("{}: {why}.", provider.name())),
            );
        }
    }
    if !others.is_empty() && (url.starts_with("http://") || url.starts_with("https://")) {
        block = block.child(
            Button::new("open-web-sign-in", "Open the web sign-in")
                .compact()
                .ghost()
                .icon(Icon::Server)
                .tooltip(url.clone())
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
        );
    }
    Some(block.into_any_element())
}
