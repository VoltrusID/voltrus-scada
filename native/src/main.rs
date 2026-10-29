//! voltrus-native: the GPUI operator console for the Voltrus SCADA server.

// A release build is a window, not a console program: no console window
// opens beside it on Windows (redirected output still reaches its files).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod console;

use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point,
    px, size,
};
use voltrus_scada_native::station::{self, Start};
use voltrus_scada_native::zone::LocalZone;
use vt_ui::theme::{self, ThemeCatalog};

use console::{Console, Launch, LaunchOverlay, Page, StressPlan, TagDialog};

fn usage() -> String {
    format!(
        "\
usage: voltrus-native [--sign-in] [--page {pages}] [--tag <key>]
                      [--range 5m|30m|1h|6h|24h] [--appearance dark|light]
                      [--hud] [--overlay palette|help|new-source|add-tags|<action>|tag-write|tag-policy|tag-note|tag-compare|tag-range|tag-expand]
                      [--alarms history|analytics|audit|detail|shelve|confirm]
                      [--screen N|ID] [--edit] [--section ID]
                      [--window WxH] [--keys <keystrokes>]
                      [--stress [--stress-tags N] [--stress-rate N] [--stress-switch-ms N]
                                [--stress-seconds N] [--stress-report FILE]]
                      [--stress-profile]

  --sign-in          sign in at once with VOLTRUS_NATIVE_URL (default http://localhost:3000),
                     VOLTRUS_NATIVE_USER (default admin) and VOLTRUS_NATIVE_PASSWORD
  --page             the page to open after signing in
  --tag              a tag key to open on the Tags page
  --range            the Trends range to start with (default 1h)
  --appearance       dark or light instead of the server's preference or the system
  --hud              show the performance HUD (also ⇧⌘P)
  --overlay          open the command palette, the shortcut help, the Data Manager's
                     new-source form, the dashboard's tag picker (add-tags), or run
                     a command by its action name
                     (e.g. ShellOpenSettings) once signed in; tag-… opens a
                     dialog of the --tag tag's detail (tag-expand widens the detail)
  --alarms           open the Alarms page with this view or dialog once signed in
  --screen           the dashboard's and the mimic's screen: its id, or 1 for the overview,
                     2 for the first
  --edit             open the dashboard in edit mode (operator roles); with
                     --page mimic, the mimic's editor
  --section          the Data Manager's section to show (with --page data), e.g. flows
  --window           the window's size in points, e.g. 1280x800 (screenshots; the
                     window's minimum, 1024x640, gives way to a smaller one)
  --keys             keystrokes to type once signed in and the --overlay is open,
                     separated by commas or spaces (e.g. down,down,down)
  --stress           feed a synthetic plant (no server): 10000 tags, 20000 updates/s
  --stress-tags      how many synthetic tags
  --stress-rate      synthetic updates per second
  --stress-switch-ms switch to the next page this often (default 2000, 0 stays put)
  --stress-seconds   end the run after this long, print the report and quit
  --stress-report    write the report to this file too
  --stress-profile   the stress test as an animation showcase: the Overview stays put, its
                     tiles flash in ripples and a trend moves on every frame (120 fps on
                     a ProMotion display)",
        pages = console::pages::ids()
    )
}

/// What the command line asks for.
#[derive(Debug, PartialEq)]
struct Options {
    sign_in: bool,
    page: Option<Page>,
    tag: Option<String>,
    range: Option<&'static str>,
    dark: Option<bool>,
    hud: bool,
    overlay: Option<LaunchOverlay>,
    stress: Option<StressPlan>,
    screen: Option<String>,
    edit: bool,
    section: Option<String>,
    window: Option<(f32, f32)>,
    keys: Option<String>,
}

fn number<T: std::str::FromStr>(flag: &str, text: Option<&String>) -> Result<T, String> {
    text.and_then(|text| text.parse().ok())
        .ok_or_else(|| format!("{flag} takes a number"))
}

/// `1280x800`: a window's width and height in points, each at least 320.
fn window_size(text: &str) -> Option<(f32, f32)> {
    let (width, height) = text.split_once('x')?;
    let (width, height): (f32, f32) = (width.parse().ok()?, height.parse().ok()?);
    (width >= 320.0 && height >= 320.0 && width.is_finite() && height.is_finite())
        .then_some((width, height))
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        sign_in: false,
        page: None,
        tag: None,
        range: None,
        dark: None,
        hud: false,
        overlay: None,
        stress: None,
        screen: None,
        edit: false,
        section: None,
        window: None,
        keys: None,
    };
    let mut stress = StressPlan::default();
    let mut stressed = false;
    let mut stress_flags = false;
    let mut switch_set = false;
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
            "--hud" => options.hud = true,
            "--overlay" => {
                options.overlay = Some(match args.next().map(String::as_str) {
                    Some("palette") => LaunchOverlay::Palette,
                    Some("help") => LaunchOverlay::Help,
                    Some("new-source") => LaunchOverlay::NewSource,
                    Some("add-tags") => LaunchOverlay::AddTags,
                    other => {
                        let name = other.unwrap_or("");
                        if let Some(dialog) = TagDialog::parse(name) {
                            LaunchOverlay::Tags(dialog)
                        } else {
                            match console::keys::COMMANDS
                                .iter()
                                .map(|command| (command.action)().name())
                                .find(|action| action.rsplit("::").next() == Some(name))
                            {
                                Some(action) => LaunchOverlay::Action(action),
                                None => {
                                    return Err(format!(
                                        "--overlay takes palette, help, a tag-… dialog or a command's action name, not “{name}”"
                                    ));
                                }
                            }
                        }
                    }
                });
            }
            "--alarms" => {
                let name = args.next().map_or("", String::as_str);
                let view = console::alarms::Launch::parse(name).ok_or_else(|| {
                    format!(
                        "--alarms takes {}, not “{name}”",
                        console::alarms::Launch::NAMES
                    )
                })?;
                options.overlay = Some(LaunchOverlay::Alarms(view));
            }
            "--screen" => match args.next() {
                Some(screen) if !screen.is_empty() => options.screen = Some(screen.clone()),
                _ => return Err("--screen takes a screen id or position".into()),
            },
            "--edit" => options.edit = true,
            "--section" => match args.next() {
                Some(id) if console::data_manager_section(id) => {
                    options.section = Some(id.clone());
                }
                other => {
                    return Err(format!(
                        "--section takes a Data Manager section id, not “{}”",
                        other.map_or("", String::as_str)
                    ));
                }
            },
            "--window" => {
                let text = args.next().map_or("", String::as_str);
                options.window = Some(
                    window_size(text)
                        .ok_or_else(|| format!("--window takes WIDTHxHEIGHT, not “{text}”"))?,
                );
            }
            "--keys" => match args.next() {
                Some(keys) if !keys.trim().is_empty() => options.keys = Some(keys.clone()),
                _ => return Err("--keys takes keystrokes, e.g. \"down down\"".into()),
            },
            "--stress" => stressed = true,
            "--stress-tags" => {
                stress.options.tags = number(arg, args.next())?;
                stress_flags = true;
            }
            "--stress-rate" => {
                stress.options.rate = number(arg, args.next())?;
                stress_flags = true;
            }
            "--stress-switch-ms" => {
                let ms: u64 = number(arg, args.next())?;
                stress.switch_every = (ms > 0).then(|| Duration::from_millis(ms));
                stress_flags = true;
                switch_set = true;
            }
            "--stress-seconds" => {
                let seconds: u64 = number(arg, args.next())?;
                stress.stop_after = Some(Duration::from_secs(seconds));
                stress_flags = true;
            }
            "--stress-report" => match args.next() {
                Some(path) if !path.is_empty() => {
                    stress.report = Some(PathBuf::from(path));
                    stress_flags = true;
                }
                _ => return Err("--stress-report takes a file".into()),
            },
            "--stress-profile" => {
                stressed = true;
                stress.options.profile = true;
            }
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument “{other}”")),
        }
    }
    if stress_flags && !stressed {
        return Err("the --stress-… options need --stress".into());
    }
    if stressed && options.sign_in {
        return Err("--stress runs without a server; leave out --sign-in".into());
    }
    if stress.options.profile && !switch_set {
        stress.switch_every = None;
    }
    options.stress = stressed.then_some(stress);
    Ok(options)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse(&args) {
        Ok(options) => options,
        Err(error) => {
            if !error.is_empty() {
                eprintln!("voltrus-native: {error}");
            }
            eprintln!("{}", usage());
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
    let station_dir = station::dir();
    let local_station = station::start(
        sign_in.is_some() || options.stress.is_some(),
        station_dir.as_deref(),
    ) == Start::Station;
    let launch = Launch {
        sign_in,
        page: options.page,
        tag: options.tag,
        range: options.range,
        hud: options.hud,
        overlay: options.overlay,
        stress: options.stress,
        screen: options.screen,
        edit: options.edit,
        section: options.section,
        zone: LocalZone::system(),
        appearance: options.dark,
        keys: options.keys,
        station_dir,
        bundled_server: station::bundled_server(),
        local_station,
    };
    let appearance = options.dark;
    let wanted = options
        .window
        .map(|(width, height)| size(px(width), px(height)));

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

            // The window's minimum: room for every page (the navbar steps
            // down to fit far narrower). `--window` may ask for less.
            let minimum = size(px(1024.0), px(640.0));
            // `--window` resizes the window once open (below): a window
            // opened at a size that needs no fitting to the screen first
            // draws at the wrong scale on the CI runner's display.
            let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Voltrus SCADA".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(14.0), px(13.0))),
                }),
                window_min_size: Some(wanted.map_or(minimum, |wanted| {
                    size(
                        wanted.width.min(minimum.width),
                        wanted.height.min(minimum.height),
                    )
                })),
                app_id: Some("id.voltrus.scada.console".into()),
                ..WindowOptions::default()
            };
            let opened = cx.open_window(options, move |window, cx| {
                let console =
                    cx.new(|cx| Console::new(runtime, events, catalog, launch, window, cx));
                let weak = console.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    // The kiosk lock keeps the window open.
                    let locked = weak
                        .update(cx, |console, cx| {
                            let locked = console.kiosk_locked();
                            if locked {
                                console.show_toast(
                                    "The kiosk lock keeps this window open",
                                    true,
                                    cx,
                                );
                            }
                            locked
                        })
                        .unwrap_or(false);
                    if locked {
                        return false;
                    }
                    let _ = weak.update(cx, |console, _| console.shutdown());
                    cx.quit();
                    true
                });
                console
            });
            match opened {
                Err(error) => {
                    eprintln!("voltrus-native: could not open the window: {error}");
                    cx.quit();
                    return;
                }
                // AppKit fits a new window to the screen (a taller one than
                // the screen keeps the screen's height).
                Ok(handle) => {
                    if let Some(wanted) = wanted {
                        let _ = handle.update(cx, |_, window, _| {
                            if window.viewport_size() != wanted {
                                window.resize(wanted);
                            }
                        });
                    }
                }
            }
            cx.activate(true);
        });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use voltrus_scada_native::stress::StressOptions;

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
                "light",
                "--hud",
                "--overlay",
                "help"
            ])),
            Ok(Options {
                sign_in: true,
                page: Some(Page::Alarms),
                tag: None,
                range: None,
                dark: Some(false),
                hud: true,
                overlay: Some(LaunchOverlay::Help),
                stress: None,
                screen: None,
                edit: false,
                section: None,
                window: None,
                keys: None,
            })
        );
        assert_eq!(
            parse(&args(&["--window", "1280x800", "--keys", "down down"]))
                .map(|options| (options.window, options.keys)),
            Ok((Some((1280.0, 800.0)), Some("down down".to_owned())))
        );
        assert!(parse(&args(&["--window", "1280"])).is_err());
        assert!(parse(&args(&["--window", "10x10"])).is_err());
        assert!(parse(&args(&["--keys"])).is_err());
        assert_eq!(
            parse(&args(&["--tag", "ac_current_l1"])).map(|options| options.tag),
            Ok(Some("ac_current_l1".to_owned()))
        );
        assert_eq!(
            parse(&args(&["--screen", "2", "--edit", "--overlay", "add-tags"])).map(|options| (
                options.screen,
                options.edit,
                options.overlay
            )),
            Ok((Some("2".to_owned()), true, Some(LaunchOverlay::AddTags)))
        );
        assert!(parse(&args(&["--screen"])).is_err());
        assert_eq!(
            parse(&args(&["--page", "performance"])).map(|options| options.page),
            Ok(Some(Page::Performance))
        );
        assert!(parse(&args(&["--tag"])).is_err());
        assert!(parse(&args(&["--page", "nowhere"])).is_err());
        assert!(parse(&args(&["--appearance"])).is_err());
        assert!(parse(&args(&["--overlay", "menu"])).is_err());
        assert_eq!(
            parse(&args(&["--page", "data", "--overlay", "new-source"]))
                .map(|options| (options.page, options.overlay)),
            Ok((Some(Page::DataManager), Some(LaunchOverlay::NewSource)))
        );
        assert_eq!(
            parse(&args(&["--page", "data", "--section", "flows"]))
                .map(|options| (options.page, options.section)),
            Ok((Some(Page::DataManager), Some("flows".to_owned())))
        );
        assert!(parse(&args(&["--section", "nowhere"])).is_err());
        assert!(parse(&args(&["--section"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
        assert_eq!(
            parse(&args(&["--overlay", "tag-write"])).map(|options| options.overlay),
            Ok(Some(LaunchOverlay::Tags(TagDialog::Write)))
        );
        assert_eq!(
            parse(&args(&["--page", "tagtree"])).map(|options| options.page),
            Ok(Some(Page::TagTree))
        );
    }

    #[test]
    fn stress_options_parse_and_need_stress() {
        let plan = parse(&args(&[
            "--stress",
            "--stress-tags",
            "500",
            "--stress-rate",
            "1000",
            "--stress-switch-ms",
            "0",
            "--stress-seconds",
            "20",
            "--stress-report",
            "/tmp/report.txt",
        ]))
        .unwrap()
        .stress
        .unwrap();
        assert_eq!(
            plan,
            StressPlan {
                options: StressOptions {
                    tags: 500,
                    rate: 1000,
                    ..StressOptions::default()
                },
                switch_every: None,
                stop_after: Some(Duration::from_secs(20)),
                report: Some(PathBuf::from("/tmp/report.txt")),
            }
        );
        assert_eq!(
            parse(&args(&["--stress"])).unwrap().stress,
            Some(StressPlan::default())
        );
        assert!(
            parse(&args(&["--stress-rate", "5"])).is_err(),
            "needs --stress"
        );
        assert!(parse(&args(&["--stress", "--stress-rate", "fast"])).is_err());
        assert!(parse(&args(&["--stress", "--sign-in"])).is_err());
    }

    #[test]
    fn the_stress_profile_stays_on_one_page_unless_told() {
        let plan = parse(&args(&["--stress-profile", "--stress-seconds", "10"]))
            .unwrap()
            .stress
            .unwrap();
        assert!(plan.options.profile);
        assert_eq!(plan.switch_every, None);
        assert_eq!(plan.stop_after, Some(Duration::from_secs(10)));
        let switching = parse(&args(&["--stress-switch-ms", "500", "--stress-profile"]))
            .unwrap()
            .stress
            .unwrap();
        assert_eq!(switching.switch_every, Some(Duration::from_millis(500)));
        assert!(
            !parse(&args(&["--stress"]))
                .unwrap()
                .stress
                .unwrap()
                .options
                .profile
        );
    }

    #[test]
    fn an_overlay_names_a_command_by_its_action() {
        assert_eq!(
            parse(&args(&["--overlay", "ShellOpenSettings"])).map(|options| options.overlay),
            Ok(Some(LaunchOverlay::Action(
                "voltrus_console::ShellOpenSettings"
            )))
        );
        assert!(parse(&args(&["--overlay", "voltrus_console::Nowhere"])).is_err());
    }

    #[test]
    fn alarms_views_parse() {
        assert_eq!(
            parse(&args(&["--sign-in", "--alarms", "shelve"])).map(|options| options.overlay),
            Ok(Some(LaunchOverlay::Alarms(console::alarms::Launch::Shelve)))
        );
        assert!(parse(&args(&["--alarms", "everything"])).is_err());
        assert!(parse(&args(&["--alarms"])).is_err());
    }
}
