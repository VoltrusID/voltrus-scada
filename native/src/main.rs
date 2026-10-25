//! voltrus-native: the GPUI operator console for the Voltrus SCADA server.

mod console;

use std::process::ExitCode;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point,
    px, size,
};
use time::UtcOffset;
use vt_ui::theme::{self, ThemeCatalog};

use console::{Console, Launch, Page};

const USAGE: &str = "\
usage: voltrus-native [--sign-in] [--page overview|tags|alarms|trends] [--tag <key>]
                      [--range 5m|30m|1h|6h|24h] [--appearance dark|light]

  --sign-in      sign in at once with VOLTRUS_NATIVE_URL (default http://localhost:3000),
                 VOLTRUS_NATIVE_USER (default admin) and VOLTRUS_NATIVE_PASSWORD
  --page         the page to open after signing in
  --tag          a tag key to open on the Tags page
  --range        the Trends range to start with (default 1h)
  --appearance   dark or light instead of following the system";

/// What the command line asks for.
#[derive(Debug, PartialEq)]
struct Options {
    sign_in: bool,
    page: Option<Page>,
    tag: Option<String>,
    range: Option<&'static str>,
    dark: Option<bool>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        sign_in: false,
        page: None,
        tag: None,
        range: None,
        dark: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sign-in" => options.sign_in = true,
            "--page" => {
                let name = args.next().map_or("", String::as_str);
                options.page =
                    Some(Page::parse(name).ok_or_else(|| format!("unknown page “{name}”"))?);
            }
            "--tag" => match args.next() {
                Some(key) if !key.is_empty() => options.tag = Some(key.clone()),
                _ => return Err("--tag takes a tag key".into()),
            },
            "--range" => {
                let text = args.next().map_or("", String::as_str);
                options.range = Some(
                    console::parse_range(text).ok_or_else(|| format!("unknown range “{text}”"))?,
                );
            }
            "--appearance" => {
                options.dark = Some(match args.next().map(String::as_str) {
                    Some("dark") => true,
                    Some("light") => false,
                    other => {
                        return Err(format!(
                            "--appearance takes dark or light, not “{}”",
                            other.unwrap_or("")
                        ));
                    }
                });
            }
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument “{other}”")),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    // `time` can only read the local offset while the process has one
    // thread, so ask before GPUI or the network runtime start any.
    let utc_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse(&args) {
        Ok(options) => options,
        Err(error) => {
            if !error.is_empty() {
                eprintln!("voltrus-native: {error}");
            }
            eprintln!("{USAGE}");
            return if error.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };
    let sign_in = if options.sign_in {
        let Ok(password) = std::env::var("VOLTRUS_NATIVE_PASSWORD") else {
            eprintln!("voltrus-native: --sign-in needs VOLTRUS_NATIVE_PASSWORD");
            return ExitCode::FAILURE;
        };
        Some((
            std::env::var("VOLTRUS_NATIVE_URL").unwrap_or_else(|_| "http://localhost:3000".into()),
            std::env::var("VOLTRUS_NATIVE_USER").unwrap_or_else(|_| "admin".into()),
            password,
        ))
    } else {
        None
    };
    let launch = Launch {
        sign_in,
        page: options.page,
        tag: options.tag,
        range: options.range,
        utc_offset,
    };
    let appearance = options.dark;

    Application::new()
        .with_assets(vt_ui::Assets)
        .run(move |cx: &mut App| {
            let catalog = Rc::new(ThemeCatalog::bundled());
            let dark = appearance.unwrap_or_else(|| theme::is_dark(cx.window_appearance()));
            let initial = catalog.resolve(
                if dark {
                    theme::DEFAULT_DARK
                } else {
                    theme::DEFAULT_LIGHT
                },
                dark,
            );
            vt_ui::init(initial, cx);
            cx.bind_keys(console::key_bindings());
            cx.set_menus(console::menus());
            cx.on_action(|_: &console::Quit, cx| cx.quit());

            let (runtime, events) = match voltrus_scada_native::runtime::start() {
                Ok(started) => started,
                Err(error) => {
                    eprintln!("voltrus-native: could not start the network runtime: {error}");
                    cx.quit();
                    return;
                }
            };

            let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Voltrus SCADA".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(14.0), px(13.0))),
                }),
                window_min_size: Some(size(px(1024.0), px(640.0))),
                app_id: Some("id.voltrus.scada.console".into()),
                ..WindowOptions::default()
            };
            let opened = cx.open_window(options, move |window, cx| {
                let console =
                    cx.new(|cx| Console::new(runtime, events, catalog, launch, window, cx));
                let weak = console.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    let _ = weak.update(cx, |console, _| console.shutdown());
                    cx.quit();
                    true
                });
                console
            });
            if let Err(error) = opened {
                eprintln!("voltrus-native: could not open the window: {error}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn launch_options_parse() {
        assert_eq!(
            parse(&args(&[
                "--sign-in",
                "--page",
                "alarms",
                "--appearance",
                "light"
            ])),
            Ok(Options {
                sign_in: true,
                page: Some(Page::Alarms),
                tag: None,
                range: None,
                dark: Some(false),
            })
        );
        assert_eq!(
            parse(&args(&["--tag", "ac_current_l1"])).map(|options| options.tag),
            Ok(Some("ac_current_l1".to_owned()))
        );
        assert!(parse(&args(&["--tag"])).is_err());
        assert!(parse(&args(&["--page", "nowhere"])).is_err());
        assert!(parse(&args(&["--appearance"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
    }
}
