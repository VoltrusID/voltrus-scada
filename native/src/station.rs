//! The local station: the server installed on this computer as a system
//! service, and the credential the console signs in to it with.
//!
//! - [`dir`]: the station directory, resolved exactly as the server's
//!   `voltrus station` and `voltrus service` commands resolve it.
//! - [`load`]: `station.toml`, the console's credential for the station.
//! - [`start`]: whether the console opens in local station mode.
//! - [`bundled_server`] and [`run`]: the `voltrus` server beside the
//!   console, and its setup commands ([`Step`]).

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Deserialize;

/// The console's credential in the station directory.
pub const FILE: &str = "station.toml";
/// Overrides the station directory (tests, CI, a second station).
pub const DIR_ENV: &str = "VOLTRUS_STATION_DIR";
/// The server's binary beside the console's.
pub const SERVER_EXE: &str = if cfg!(windows) {
    "voltrus.exe"
} else {
    "voltrus"
};

/// The platform rules of [`dir`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Os {
    Windows,
    Mac,
    Other,
}

const OS: Os = if cfg!(windows) {
    Os::Windows
} else if cfg!(target_os = "macos") {
    Os::Mac
} else {
    Os::Other
};

/// The station directory: `VOLTRUS_STATION_DIR` if set, else
/// `%ProgramData%\Voltrus SCADA` on Windows, `~/Library/Application
/// Support/Voltrus SCADA` on macOS and the XDG data directory elsewhere.
/// `None` without a home to put it in.
pub fn dir() -> Option<PathBuf> {
    dir_for(OS, |key| std::env::var_os(key))
}

/// [`dir`] for `os`, reading the environment through `env`. Deliberately a
/// copy of the server's resolution (`voltrus station`), not shared code:
/// the console builds without the server crate, so both must change
/// together.
fn dir_for(os: Os, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |key: &str| env(key).filter(|value| !value.is_empty());
    if let Some(dir) = var(DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    match os {
        Os::Windows => Some(
            PathBuf::from(var("PROGRAMDATA").unwrap_or_else(|| "C:\\ProgramData".into()))
                .join("Voltrus SCADA"),
        ),
        Os::Mac => Some(
            PathBuf::from(var("HOME")?)
                .join("Library")
                .join("Application Support")
                .join("Voltrus SCADA"),
        ),
        Os::Other => Some(
            var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| Some(PathBuf::from(var("HOME")?).join(".local").join("share")))?
                .join("voltrus-scada"),
        ),
    }
}

/// `station.toml`: where the station's server answers and the account the
/// console signs in with (written by `voltrus station init`).
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Credential {
    pub url: String,
    pub username: String,
    pub password: String,
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("url", &self.url)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// `station.toml`'s text as a credential, its address without a trailing
/// slash.
pub fn parse(text: &str) -> Result<Credential, String> {
    let mut credential: Credential =
        toml::from_str(text).map_err(|error| error.message().to_owned())?;
    credential.url = credential.url.trim().trim_end_matches('/').to_owned();
    if !(credential.url.starts_with("http://") || credential.url.starts_with("https://")) {
        return Err("its url does not start with http:// or https://".into());
    }
    if credential.username.trim().is_empty() || credential.password.is_empty() {
        return Err("it has no username or password".into());
    }
    Ok(credential)
}

/// The station's credential in `dir`.
pub fn load(dir: &Path) -> Result<Credential, String> {
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    parse(&text).map_err(|error| format!("{} is not usable: {error}", path.display()))
}

/// Whether `dir` holds a station's credential.
pub fn exists(dir: &Path) -> bool {
    dir.join(FILE).is_file()
}

/// How the console starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// As the command line says (`--sign-in`, `--stress`).
    AsTold,
    /// Local station mode: signs in to the station by itself.
    Station,
    /// The sign-in card, with the last server and username.
    Card,
}

/// How the console starts: `told` when the command line signs in or runs a
/// stress test, `dir` the station directory.
pub fn start(told: bool, dir: Option<&Path>) -> Start {
    if told {
        Start::AsTold
    } else if dir.is_some_and(exists) {
        Start::Station
    } else {
        Start::Card
    }
}

/// How long to wait before asking the station again after `failures` in a
/// row: 1 s, 2 s, 4 s, then every 5 s.
pub fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs((1u64 << failures.saturating_sub(1).min(3)).min(5))
}

/// The server installed beside the console (`voltrus.exe` in the same
/// folder on Windows, `Contents/MacOS/voltrus` in the macOS app).
pub fn bundled_server() -> Option<PathBuf> {
    beside(&std::env::current_exe().ok()?)
}

fn beside(exe: &Path) -> Option<PathBuf> {
    let server = exe.parent()?.join(SERVER_EXE);
    server.is_file().then_some(server)
}

/// Setting up this computer as a station, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Init,
    Install,
    Start,
}

impl Step {
    pub const ALL: [Self; 3] = [Self::Init, Self::Install, Self::Start];

    /// The server's arguments for this step.
    pub fn args(self, dir: &Path) -> Vec<OsString> {
        let command: [&str; 2] = match self {
            Self::Init => ["station", "init"],
            Self::Install => ["service", "install"],
            Self::Start => ["service", "start"],
        };
        command
            .into_iter()
            .map(OsString::from)
            .chain([OsString::from("--dir"), dir.as_os_str().to_owned()])
            .collect()
    }

    /// What the step is doing, for the sign-in card.
    pub fn label(self) -> &'static str {
        match self {
            Self::Init => "Creating the station…",
            Self::Install => "Installing the server as a service…",
            Self::Start => "Starting the service…",
        }
    }
}

/// The most of a failed command's output shown.
const OUTPUT_LIMIT: usize = 600;

/// Run `step` with the station's `server` for `dir`, blocking until it
/// ends; why it failed is its last output.
pub fn run(server: &Path, step: Step, dir: &Path) -> Result<(), String> {
    let args = step.args(dir);
    let shown = args
        .iter()
        .take(2)
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let mut command = Command::new(server);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes up for the server's
        // command line.
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .output()
        .map_err(|error| format!("Could not run {}: {error}", server.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let said = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    let mut start = said.len().saturating_sub(OUTPUT_LIMIT);
    while !said.is_char_boundary(start) {
        start += 1;
    }
    let mut message = format!("voltrus {shown} failed ({})", output.status);
    if !said.is_empty() {
        message.push_str(": ");
        message.push_str(&said[start..]);
    }
    // Installing a Windows service needs an administrator.
    if cfg!(windows) && step == Step::Install && said.to_ascii_lowercase().contains("denied") {
        message.push_str(
            " Run Voltrus SCADA as administrator once (right-click, Run as administrator) to install the service.",
        );
    }
    Err(message)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), OsString::from(value)))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn the_station_dir_follows_the_servers_rules() {
        // The override wins everywhere; an empty one is unset.
        for os in [Os::Windows, Os::Mac, Os::Other] {
            assert_eq!(
                dir_for(os, env(&[(DIR_ENV, "/srv/station"), ("HOME", "/home/op")])),
                Some(PathBuf::from("/srv/station"))
            );
        }
        assert_eq!(
            dir_for(
                Os::Windows,
                env(&[(DIR_ENV, ""), ("PROGRAMDATA", "D:\\Data")])
            ),
            Some(PathBuf::from("D:\\Data").join("Voltrus SCADA"))
        );
        assert_eq!(
            dir_for(Os::Windows, env(&[])),
            Some(PathBuf::from("C:\\ProgramData").join("Voltrus SCADA"))
        );
        assert_eq!(
            dir_for(Os::Mac, env(&[("HOME", "/Users/op")])),
            Some(PathBuf::from(
                "/Users/op/Library/Application Support/Voltrus SCADA"
            ))
        );
        assert_eq!(dir_for(Os::Mac, env(&[])), None);
        assert_eq!(
            dir_for(
                Os::Other,
                env(&[("XDG_DATA_HOME", "/data"), ("HOME", "/home/op")])
            ),
            Some(PathBuf::from("/data/voltrus-scada"))
        );
        assert_eq!(
            dir_for(Os::Other, env(&[("HOME", "/home/op")])),
            Some(PathBuf::from("/home/op/.local/share/voltrus-scada"))
        );
        assert_eq!(dir_for(Os::Other, env(&[])), None);
    }

    #[test]
    fn station_toml_parses_and_keeps_the_password_out_of_debug() {
        let credential = parse(
            "url = \"http://127.0.0.1:3000/\"\nusername = \"station\"\npassword = \"s3cretS3cret\"\n",
        )
        .unwrap();
        assert_eq!(
            credential,
            Credential {
                url: "http://127.0.0.1:3000".into(),
                username: "station".into(),
                password: "s3cretS3cret".into(),
            }
        );
        assert!(!format!("{credential:?}").contains("s3cret"));
        assert!(parse("url = \"127.0.0.1:3000\"\nusername = \"s\"\npassword = \"p\"\n").is_err());
        assert!(parse("url = \"http://x\"\nusername = \"\"\npassword = \"p\"\n").is_err());
        assert!(parse("url = \"http://x\"\nusername = \"s\"\n").is_err());
        assert!(parse("not toml").is_err());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "voltrus-native-station-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_launch_opens_the_station_only_when_not_told_otherwise() {
        let dir = scratch("launch");
        assert_eq!(start(false, Some(&dir)), Start::Card, "no station.toml");
        assert_eq!(start(false, None), Start::Card);
        std::fs::write(
            dir.join(FILE),
            "url = \"http://127.0.0.1:3000\"\nusername = \"station\"\npassword = \"p\"\n",
        )
        .unwrap();
        assert_eq!(start(false, Some(&dir)), Start::Station);
        assert_eq!(start(true, Some(&dir)), Start::AsTold, "--sign-in wins");
        assert_eq!(load(&dir).unwrap().username, "station");
        assert!(load(&dir.join("nowhere")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn retries_back_off_to_five_seconds() {
        let secs: Vec<u64> = (1..=6).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 5, 5, 5]);
        assert_eq!(retry_delay(0).as_secs(), 1);
        assert_eq!(retry_delay(u32::MAX).as_secs(), 5);
    }

    #[test]
    fn the_setup_steps_name_the_station_dir() {
        let dir = Path::new("/srv/station");
        let args = |step: Step| -> Vec<String> {
            step.args(dir)
                .into_iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(
            args(Step::Init),
            ["station", "init", "--dir", "/srv/station"]
        );
        assert_eq!(
            args(Step::Install),
            ["service", "install", "--dir", "/srv/station"]
        );
        assert_eq!(
            args(Step::Start),
            ["service", "start", "--dir", "/srv/station"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_step_says_what_the_server_said() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("run");
        let server = dir.join(SERVER_EXE);
        std::fs::write(
            &server,
            "#!/bin/sh\nif [ \"$1\" = station ]; then echo made; exit 0; fi\necho \"no service manager\" >&2\nexit 2\n",
        )
        .unwrap();
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(beside(&dir.join("voltrus-native")), Some(server.clone()));
        assert_eq!(beside(&dir.join("nested").join("voltrus-native")), None);
        assert_eq!(run(&server, Step::Init, &dir), Ok(()));
        let error = run(&server, Step::Install, &dir).unwrap_err();
        assert!(
            error.starts_with("voltrus service install failed"),
            "{error}"
        );
        assert!(error.ends_with("no service manager"), "{error}");
        assert!(
            run(&dir.join("missing"), Step::Start, &dir)
                .unwrap_err()
                .starts_with("Could not run")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
