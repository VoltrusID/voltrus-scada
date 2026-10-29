//! Local station: one PC runs the server as an OS background service bound to
//! 127.0.0.1, and the native console on the same PC signs in to it with a
//! credential the server wrote next to its data.
//!
//! The station directory ([`station_dir`]) holds:
//! - `config.yaml`: the server config (loopback host, port, absolute `db_path`,
//!   a persisted `auth_secret` so sessions survive restarts),
//! - `data.db`: the database,
//! - `logs/voltrus.log`: the service log,
//! - `station.toml`: the console credential (`url`, `username`, `password`).
//!
//! `voltrus station init` creates or repairs all of it; the native console
//! resolves the same directory to find `station.toml`.

use crate::config;
use crate::storage::Storage;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Overrides the station directory (tests, CI, portable installs).
pub const DIR_ENV: &str = "VOLTRUS_STATION_DIR";
pub const CONFIG_FILE: &str = "config.yaml";
pub const DB_FILE: &str = "data.db";
pub const CREDENTIAL_FILE: &str = "station.toml";
pub const LOG_DIR: &str = "logs";
pub const LOG_FILE: &str = "voltrus.log";
/// The account the console signs in with.
pub const STATION_USER: &str = "station";
pub const DEFAULT_PORT: u16 = 3000;
/// How far above the requested port `station init` looks for a free one.
const PORT_SEARCH_RANGE: u16 = 100;
const AUTH_SECRET_LEN: usize = 48;
const PASSWORD_LEN: usize = 32;

/// The station directory: `VOLTRUS_STATION_DIR` if set, else the per-OS
/// application data directory.
pub fn station_dir() -> PathBuf {
    resolve_station_dir(|key| std::env::var_os(key))
}

fn resolve_station_dir(env: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    let var = |key: &str| env(key).filter(|value| !value.is_empty());
    if let Some(dir) = var(DIR_ENV) {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    {
        PathBuf::from(var("PROGRAMDATA").unwrap_or_else(|| OsString::from(r"C:\ProgramData")))
            .join("Voltrus SCADA")
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from(var("HOME").unwrap_or_default())
            .join("Library")
            .join("Application Support")
            .join("Voltrus SCADA")
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        match var("XDG_DATA_HOME") {
            Some(data_home) => PathBuf::from(data_home),
            None => PathBuf::from(var("HOME").unwrap_or_default())
                .join(".local")
                .join("share"),
        }
        .join("voltrus-scada")
    }
}

/// `station.toml`: what the console signs in with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    pub url: String,
    pub username: String,
    pub password: String,
}

/// The part of `config.yaml` that `station init` writes. Read back through
/// [`config::load_config`], so it stays a valid [`crate::models::AppConfig`].
#[derive(Serialize)]
struct StationConfig<'a> {
    server_host: &'a str,
    server_port: u16,
    db_path: &'a str,
    auth_secret: &'a str,
    cookie_secure: bool,
}

/// Outcome of [`init`].
#[derive(Debug)]
pub struct Initialized {
    pub dir: PathBuf,
    pub url: String,
    pub username: String,
    /// `config.yaml` was written by this run (it did not exist).
    pub config_created: bool,
    /// The `station` user got a new password (and `station.toml` with it).
    pub password_changed: bool,
    /// The seed created the `admin` user with a random password (not shown).
    pub admin_created: bool,
    pub config_path: PathBuf,
}

/// `voltrus station init`: create the station directory, its config and
/// database, the `station` user and `station.toml`. Idempotent: an existing
/// config (and its `auth_secret`) is kept, and the stored password is kept as
/// long as it still signs in.
///
/// The directory is locked down before anything is written into it: private
/// to the owner on unix; on Windows owned by Administrators, full control for
/// SYSTEM and Administrators only, with Users allowed to list the folder and
/// read `station.toml`. On Windows that needs an elevated prompt.
pub async fn init(dir: &Path, port: Option<u16>) -> Result<Initialized> {
    let dir = std::path::absolute(dir)
        .with_context(|| format!("Cannot resolve station directory {}", dir.display()))?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Cannot create station directory {}", dir.display()))?;
    secure_dir(&dir)?;
    std::fs::create_dir_all(dir.join(LOG_DIR))
        .with_context(|| format!("Cannot create {}", dir.join(LOG_DIR).display()))?;

    let config_path = dir.join(CONFIG_FILE);
    let config_created = !config_path.exists();
    if config_created {
        let port = free_port(port.unwrap_or(DEFAULT_PORT))?;
        write_atomic(&config_path, render_config(&dir, port)?.as_bytes())?;
    }
    let app_config = config::load_config(path_str(&config_path)?)?;
    if !config_created && port.is_some_and(|p| p != app_config.server_port) {
        println!(
            "Keeping the existing {} (port {})",
            config_path.display(),
            app_config.server_port
        );
    }

    // `service run` starts the server in the station directory, so a relative
    // `db_path` in a hand-edited config is relative to it.
    let db_path = dir.join(&app_config.db_path);
    let storage =
        Storage::new_with_key(path_str(&db_path)?, app_config.db_encryption_key.as_deref())?;
    // Setup output ends up in installer logs: the station signs in as
    // `station`, so the generated `admin` password is not shown.
    let admin_created = storage.seed_database_quietly(&app_config.devices).await?;

    let credential_path = dir.join(CREDENTIAL_FILE);
    let saved = read_credential(&credential_path);
    let (password, password_changed) = ensure_station_user(&storage, saved.as_ref()).await?;

    let credential = Credential {
        url: format!("http://127.0.0.1:{}", app_config.server_port),
        username: STATION_USER.to_string(),
        password,
    };
    if saved.as_ref() != Some(&credential) {
        write_credential(&credential_path, &credential)?;
    }
    allow_users_to_read(&credential_path)?;

    Ok(Initialized {
        dir,
        url: credential.url,
        username: credential.username,
        config_created,
        password_changed,
        admin_created,
        config_path,
    })
}

/// Unix: the station directory is private to the user the service runs as.
#[cfg(unix)]
fn secure_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("Cannot restrict {}", dir.display()))
}

/// Windows: make BUILTIN\Administrators the owner of the directory and all in
/// it, give it a protected ACL (SYSTEM and Administrators full control,
/// inherited; Users may only list the folder itself) and let the contents
/// inherit that. Fails before anything is written when not elevated, so a
/// standard user cannot claim the folder first.
///
/// The directory may already hold files and folders: the installer creates
/// `logs\` and writes `logs\setup.log` before it runs `station init`, and an
/// earlier install leaves its data. So ownership is taken recursively and,
/// after the folder ACL is set, every existing entry is reset (recursively) to
/// inherit it, not only what init writes afterwards.
#[cfg(windows)]
fn secure_dir(dir: &Path) -> Result<()> {
    const ADMINISTRATORS: &str = "*S-1-5-32-544";
    const SYSTEM: &str = "*S-1-5-18";
    const USERS: &str = "*S-1-5-32-545";
    let dir_arg = dir.as_os_str();

    if let Err(e) = icacls(&[dir_arg, "/setowner".as_ref(), ADMINISTRATORS.as_ref()]) {
        return Err(e.context(format!(
            "Cannot take ownership of {}. Run `voltrus station init` as administrator \
             (from an elevated prompt).",
            dir.display()
        )));
    }
    icacls(&[
        dir_arg,
        "/setowner".as_ref(),
        ADMINISTRATORS.as_ref(),
        "/T".as_ref(),
        "/C".as_ref(),
    ])?;
    // Drop every explicit entry, then replace the inherited ones with ours.
    icacls(&[dir_arg, "/reset".as_ref()])?;
    icacls(&[
        dir_arg,
        "/inheritance:r".as_ref(),
        "/grant:r".as_ref(),
        format!("{SYSTEM}:(OI)(CI)F").as_ref(),
        format!("{ADMINISTRATORS}:(OI)(CI)F").as_ref(),
        format!("{USERS}:(RX)").as_ref(),
    ])?;
    for entry in std::fs::read_dir(dir).with_context(|| format!("Cannot list {}", dir.display()))? {
        let path = entry?.path();
        icacls(&[
            path.as_os_str(),
            "/reset".as_ref(),
            "/T".as_ref(),
            "/C".as_ref(),
        ])?;
    }
    Ok(())
}

/// Windows: the console runs as a standard user and reads `station.toml`.
/// Rewriting the file replaces it, so the grant is applied after every write.
#[cfg(windows)]
fn allow_users_to_read(path: &Path) -> Result<()> {
    icacls(&[
        path.as_os_str(),
        "/grant".as_ref(),
        "*S-1-5-32-545:(R)".as_ref(),
    ])
}

/// Unix: the console runs as the same user as the service.
#[cfg(unix)]
fn allow_users_to_read(_path: &Path) -> Result<()> {
    Ok(())
}

/// Run `icacls` (well-known SIDs keep it independent of the Windows language).
#[cfg(windows)]
fn icacls(args: &[&std::ffi::OsStr]) -> Result<()> {
    let output = std::process::Command::new("icacls")
        .args(args)
        .output()
        .context("Cannot run icacls")?;
    anyhow::ensure!(
        output.status.success(),
        "icacls {} failed ({}): {}",
        args.iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
        output.status,
        String::from_utf8_lossy(if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        })
        .trim()
    );
    Ok(())
}

/// Read `station.toml`; `None` when it is missing or unreadable.
pub fn read_credential(path: &Path) -> Option<Credential> {
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

fn write_credential(path: &Path, credential: &Credential) -> Result<()> {
    let body = toml::to_string(credential).context("Cannot encode station.toml")?;
    let text = format!(
        "# Local station sign-in for the Voltrus SCADA console.\n\
         # Written by `voltrus station init`; run it again to repair.\n{body}"
    );
    write_atomic(path, text.as_bytes())
}

/// Make sure the `station` user exists as an enabled, unlocked admin that does
/// not have to change its password, and return the password the console should
/// use. The saved password is kept while it still verifies; otherwise a new
/// random one is set (and the user's old sessions are revoked).
async fn ensure_station_user(
    storage: &Storage,
    saved: Option<&Credential>,
) -> Result<(String, bool)> {
    let user = storage.get_user_by_username(STATION_USER).await?;
    let kept = match (&user, saved) {
        (Some(user), Some(saved))
            if saved.username == STATION_USER
                && crate::api::auth::verify_password(&saved.password, &user.password_hash) =>
        {
            Some(saved.password.clone())
        }
        _ => None,
    };
    let password_changed = kept.is_none();
    let password = kept.unwrap_or_else(|| random_alphanumeric(PASSWORD_LEN));

    match user {
        None => {
            let hash = crate::api::auth::hash_password(&password);
            storage
                .create_user(STATION_USER, &hash, "", "admin")
                .await?;
        }
        Some(user) => {
            if password_changed {
                let hash = crate::api::auth::hash_password(&password);
                storage.update_user_password(user.id, &hash).await?;
                storage.revoke_all_user_refresh_tokens(STATION_USER).await?;
            }
            if user.role != "admin" {
                storage.update_user_role(user.id, "admin").await?;
            }
            if user.disabled {
                storage.set_user_disabled(user.id, false).await?;
            }
        }
    }
    storage
        .set_must_change_password(STATION_USER, false)
        .await?;
    storage.reset_failed_attempts(STATION_USER).await?;
    Ok((password, password_changed))
}

fn render_config(dir: &Path, port: u16) -> Result<String> {
    let db_path = dir.join(DB_FILE);
    let auth_secret = random_alphanumeric(AUTH_SECRET_LEN);
    let yaml = serde_yaml_neo::to_string(&StationConfig {
        server_host: "127.0.0.1",
        server_port: port,
        db_path: path_str(&db_path)?,
        auth_secret: &auth_secret,
        cookie_secure: false,
    })
    .context("Cannot encode config.yaml")?;
    Ok(format!(
        "# Local station server config, written by `voltrus station init`.\n\
         # The server listens on this computer only; keep auth_secret private.\n{yaml}"
    ))
}

/// The first port from `start` up to `start + 100` that nothing on this
/// computer listens on.
fn free_port(start: u16) -> Result<u16> {
    let end = start.saturating_add(PORT_SEARCH_RANGE);
    (start..=end)
        .find(|&port| port_is_free(port))
        .with_context(|| format!("No free TCP port on 127.0.0.1 between {start} and {end}"))
}

/// Nothing accepts connections on 127.0.0.1:`port` and the port can be bound
/// there. The connect check catches a listener on all interfaces that a bind
/// with `SO_REUSEADDR` would not notice. Only loopback is touched, so no
/// firewall prompt appears.
fn port_is_free(port: u16) -> bool {
    let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&loopback, Duration::from_millis(200)).is_err()
        && TcpListener::bind(loopback).is_ok()
}

fn random_alphanumeric(len: usize) -> String {
    use rand::Rng;
    rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// Write through a temporary file in the same directory and rename it over
/// `path`, so a reader never sees a half-written file. The temporary file is
/// private to the owner on unix, and the rename keeps that.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("Cannot write in {}", dir.display()))?;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .with_context(|| format!("Cannot write {}", path.display()))?;
    Ok(())
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("Path is not valid UTF-8: {}", path.display()))
}

/// The value of `--dir D` in `args`, or the default station directory.
pub(crate) fn dir_arg(args: &[String]) -> Result<PathBuf> {
    match flag_value(args, "--dir")? {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(station_dir()),
    }
}

/// The value after `flag` in `args`, if the flag is present.
pub(crate) fn flag_value<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>> {
    match args.iter().position(|arg| arg == flag) {
        Some(i) => args
            .get(i + 1)
            .map(|value| Some(value.as_str()))
            .with_context(|| format!("{flag} needs a value")),
        None => Ok(None),
    }
}

/// `voltrus station init [--dir D] [--port N]`.
pub(crate) async fn handle_cli(args: &[String]) -> bool {
    let usage = || {
        eprintln!("Usage: voltrus station init [--dir DIR] [--port PORT]");
        std::process::exit(1);
    };
    if args.get(2).map(String::as_str) != Some("init") {
        usage();
    }
    let options = &args[3..];
    let parsed = dir_arg(options).and_then(|dir| {
        let port = flag_value(options, "--port")?
            .map(|p| {
                p.parse::<u16>()
                    .with_context(|| format!("Invalid port: {p}"))
            })
            .transpose()?;
        Ok((dir, port))
    });
    let result = match parsed {
        Ok((dir, port)) => init(&dir, port).await,
        Err(e) => Err(e),
    };
    match result {
        Ok(done) => {
            println!("Station directory: {}", done.dir.display());
            println!("Server URL: {}", done.url);
            println!("Console user: {}", done.username);
            if done.admin_created {
                println!(
                    "Created user admin with a random password. Set one with \
                     `voltrus --reset-admin-password \"{}\"` or under Users.",
                    done.config_path.display()
                );
            }
            true
        }
        Err(e) => {
            eprintln!("Error: {e:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |key| vars.get(key).cloned()
    }

    #[test]
    fn station_dir_env_override_wins() {
        let env = env_of(&[
            (DIR_ENV, "/tmp/station"),
            ("PROGRAMDATA", "D:\\Data"),
            ("HOME", "/home/op"),
            ("XDG_DATA_HOME", "/data"),
        ]);
        assert_eq!(resolve_station_dir(env), PathBuf::from("/tmp/station"));
    }

    #[test]
    fn station_dir_empty_override_is_ignored() {
        let env = env_of(&[
            (DIR_ENV, ""),
            ("PROGRAMDATA", "D:\\Data"),
            ("HOME", "/home/op"),
        ]);
        assert_ne!(resolve_station_dir(env), PathBuf::from(""));
    }

    #[cfg(windows)]
    #[test]
    fn station_dir_windows_default() {
        assert_eq!(
            resolve_station_dir(env_of(&[("PROGRAMDATA", r"D:\Data")])),
            PathBuf::from(r"D:\Data\Voltrus SCADA")
        );
        assert_eq!(
            resolve_station_dir(env_of(&[])),
            PathBuf::from(r"C:\ProgramData\Voltrus SCADA")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn station_dir_macos_default() {
        assert_eq!(
            resolve_station_dir(env_of(&[("HOME", "/Users/op")])),
            PathBuf::from("/Users/op/Library/Application Support/Voltrus SCADA")
        );
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn station_dir_xdg_default() {
        assert_eq!(
            resolve_station_dir(env_of(&[("HOME", "/home/op"), ("XDG_DATA_HOME", "/data")])),
            PathBuf::from("/data/voltrus-scada")
        );
        assert_eq!(
            resolve_station_dir(env_of(&[("HOME", "/home/op")])),
            PathBuf::from("/home/op/.local/share/voltrus-scada")
        );
    }

    #[test]
    fn busy_port_falls_back_to_the_next_free_one() {
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        assert!(!port_is_free(port));
        let picked = free_port(port).unwrap();
        assert!(picked > port && picked <= port.saturating_add(PORT_SEARCH_RANGE));
    }

    #[test]
    fn flag_values_are_parsed() {
        let args: Vec<String> = ["--dir", "/x", "--port", "4000"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(dir_arg(&args).unwrap(), PathBuf::from("/x"));
        assert_eq!(flag_value(&args, "--port").unwrap(), Some("4000"));
        assert!(flag_value(&["--port".to_string()], "--port").is_err());
    }

    async fn station_user_can_sign_in(dir: &Path, credential: &Credential) -> bool {
        let app_config = config::load_config(dir.join(CONFIG_FILE).to_str().unwrap()).unwrap();
        let storage = Storage::new(&app_config.db_path).unwrap();
        let user = storage
            .get_user_by_username(&credential.username)
            .await
            .unwrap()
            .expect("station user exists");
        user.role == "admin"
            && !user.disabled
            && !user.must_change_password
            && crate::api::auth::verify_password(&credential.password, &user.password_hash)
    }

    #[tokio::test]
    async fn init_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("station");
        let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);

        let first = init(&dir, Some(port)).await.unwrap();
        assert!(first.config_created);
        assert!(first.password_changed);
        assert!(first.admin_created);
        assert_eq!(first.username, STATION_USER);

        let config_text = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        let app_config = config::load_config(dir.join(CONFIG_FILE).to_str().unwrap()).unwrap();
        assert_eq!(app_config.server_host, "127.0.0.1");
        // The port was free a moment ago; another process may have taken it
        // since, in which case init moved up to the next free one.
        assert!((port..=port.saturating_add(PORT_SEARCH_RANGE)).contains(&app_config.server_port));
        assert_eq!(
            PathBuf::from(&app_config.db_path),
            std::path::absolute(dir.join(DB_FILE)).unwrap()
        );
        assert_eq!(app_config.auth_secret.as_deref().map(str::len), Some(48));
        assert_eq!(app_config.cookie_secure, crate::models::CookieSecure::Never);
        assert!(dir.join(LOG_DIR).is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join(CONFIG_FILE)), 0o600);
            assert_eq!(mode(&dir.join(CREDENTIAL_FILE)), 0o600);
        }

        let credential = read_credential(&dir.join(CREDENTIAL_FILE)).unwrap();
        assert_eq!(
            credential.url,
            format!("http://127.0.0.1:{}", app_config.server_port)
        );
        assert_eq!(credential.username, STATION_USER);
        assert_eq!(credential.password.len(), 32);
        assert!(station_user_can_sign_in(&dir, &credential).await);

        // Second run: config and password are kept.
        let second = init(&dir, Some(port)).await.unwrap();
        assert!(!second.config_created);
        assert!(!second.password_changed);
        assert!(!second.admin_created);
        assert_eq!(
            std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap(),
            config_text
        );
        assert_eq!(
            read_credential(&dir.join(CREDENTIAL_FILE)),
            Some(credential.clone())
        );

        // A stored password that no longer works is replaced.
        let stale = Credential {
            password: "not-the-password".to_string(),
            ..credential.clone()
        };
        write_credential(&dir.join(CREDENTIAL_FILE), &stale).unwrap();
        let third = init(&dir, None).await.unwrap();
        assert!(third.password_changed);
        let repaired = read_credential(&dir.join(CREDENTIAL_FILE)).unwrap();
        assert_ne!(repaired.password, credential.password);
        assert!(station_user_can_sign_in(&dir, &repaired).await);

        // A missing station.toml gets a fresh password too.
        std::fs::remove_file(dir.join(CREDENTIAL_FILE)).unwrap();
        let fourth = init(&dir, None).await.unwrap();
        assert!(fourth.password_changed);
        let rewritten = read_credential(&dir.join(CREDENTIAL_FILE)).unwrap();
        assert!(station_user_can_sign_in(&dir, &rewritten).await);
    }

    #[tokio::test]
    async fn init_skips_a_busy_port() {
        let tmp = tempfile::tempdir().unwrap();
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let done = init(tmp.path(), Some(port)).await.unwrap();
        let app_config =
            config::load_config(tmp.path().join(CONFIG_FILE).to_str().unwrap()).unwrap();
        assert_ne!(app_config.server_port, port);
        assert_eq!(
            done.url,
            format!("http://127.0.0.1:{}", app_config.server_port)
        );
    }
}
