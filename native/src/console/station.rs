//! Local station mode: the console on the computer that runs the server as
//! a service signs in to it by itself, with the credential the server's
//! `voltrus station init` left in the station directory
//! (`voltrus_scada_native::station`). Until the server answers, the window
//! shows the station's state instead of the sign-in card
//! (`views::station`) and keeps asking; a session that ends is started
//! again. Signing out leaves the mode for the sign-in card, whose "Use this
//! station" comes back to it.
//!
//! On a computer with the server beside the console but no station yet,
//! the sign-in card offers to set one up: the server's `station init`,
//! `service install` and `service start`, run off the UI thread. A setup
//! that failed keeps its error and its button on the card. A station that
//! does not answer is asked about its service (`service status`): one not
//! installed (a setup that stopped after `station init`) offers to finish
//! the setup, a stopped one to start it.

use std::path::PathBuf;

use gpui::{Context, Task, Window};
use serde::de::IgnoredAny;
use voltrus_scada_native::runtime::{Call, CallKey, Command, Reply};
use voltrus_scada_native::station::{self, Credential, Service, Step};

use super::{Console, Launch};

/// The scope of the health checks' keys (`Command::Public`).
pub const SCOPE: &str = "station";
const HEALTH: &str = "station.health";

/// The station as the console knows it.
#[derive(Default)]
pub struct Station {
    /// The station directory, if this computer has one.
    pub dir: Option<PathBuf>,
    /// The server beside the console, which can set a station up.
    pub server: Option<PathBuf>,
    /// The directory holds a credential (`station.toml`), as last looked.
    pub available: bool,
    /// Local station mode, while in it.
    pub local: Option<Local>,
    /// Setting up this computer as a station, while it runs or once it
    /// failed.
    pub setup: Option<Setup>,
}

impl Station {
    pub fn new(launch: &Launch) -> Self {
        let mut station = Self {
            dir: launch.station_dir.clone(),
            server: launch.bundled_server.clone(),
            ..Self::default()
        };
        station.look();
        station
    }

    /// Look for the credential again.
    fn look(&mut self) {
        self.available = self.dir.as_deref().is_some_and(station::exists);
    }

    /// Whether the sign-in card offers to set this computer up: no station
    /// yet, or a setup that is running or failed (after `station init`
    /// made `station.toml`, the rest may still be missing).
    pub fn offers_setup(&self) -> bool {
        self.server.is_some()
            && self.dir.is_some()
            && self.local.is_none()
            && (!self.available || self.setup.is_some())
    }

    /// Whether the sign-in card offers to go back to the station.
    pub fn offers_return(&self) -> bool {
        self.available && self.local.is_none() && self.setup.is_none()
    }
}

/// Local station mode.
pub struct Local {
    pub dir: PathBuf,
    pub status: Status,
    /// Failed attempts in a row, for the back-off.
    failures: u32,
    /// Counts health checks: an answer to an older one is dropped.
    seq: i64,
    /// The credential the last health check read, for its sign-in.
    credential: Option<Credential>,
    /// The next attempt, while waiting for it.
    retry: Option<Task<()>>,
    /// The station's service as the bundled server last reported it while
    /// the station did not answer (`None` once it answers, or unknown).
    pub service: Option<Result<Service, String>>,
    /// Asking the bundled server about the service.
    querying: bool,
    /// Finishing the setup or starting the service, while it runs or once
    /// it failed.
    pub fix: Option<Setup>,
}

impl Local {
    /// Whether the operator may retry: an attempt waits for its timer, or
    /// the server refused the credential (no attempt until asked).
    pub fn idle(&self) -> bool {
        self.retry.is_some() || matches!(self.status, Status::Refused(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Asking the server whether it is up.
    Connecting,
    /// The server answered; signing in with the station's credential.
    SigningIn,
    /// The last attempt failed: why, before the next one.
    Unreachable(String),
    /// The server refused the station's credential: why. Not tried again
    /// until the operator retries, as each refusal counts towards locking
    /// the account.
    Refused(String),
    /// Signed in.
    Live,
}

/// Setting up this computer as a station.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    Running(Step),
    Failed(String),
}

/// Enter local station mode and start asking the server.
pub fn open(console: &mut Console, cx: &mut Context<Console>) {
    let Some(dir) = console.station.dir.clone() else {
        return;
    };
    console.error = None;
    console.station.local = Some(Local {
        dir,
        status: Status::Connecting,
        failures: 0,
        seq: 0,
        credential: None,
        retry: None,
        service: None,
        querying: false,
        fix: None,
    });
    check(console, cx);
}

/// Leave local station mode for the sign-in card.
pub fn leave(console: &mut Console, cx: &mut Context<Console>) {
    console.station.local = None;
    console.station.look();
    cx.notify();
}

/// Retry now (the station screen's Retry), the back-off starting over;
/// after a refusal, once: `station.toml` is read again (`voltrus station
/// init` may have changed the password).
pub fn retry(console: &mut Console, cx: &mut Context<Console>) {
    if let Some(local) = &mut console.station.local {
        local.failures = 0;
        if matches!(local.status, Status::Refused(_)) {
            local.status = Status::Connecting;
        }
        check(console, cx);
    }
}

/// Ask the station's server whether it is up (`GET /api/v1/health`); its
/// answer comes back as a public reply ([`on_public_reply`]).
fn check(console: &mut Console, cx: &mut Context<Console>) {
    let Some(local) = &mut console.station.local else {
        return;
    };
    local.retry = None;
    match station::load(&local.dir) {
        Err(error) => {
            local.credential = None;
            wait(console, error, cx);
        }
        Ok(credential) => {
            // The status stays as it was (why the last attempt failed)
            // until this one is answered.
            local.seq += 1;
            let base = credential.url.clone();
            let key = CallKey::new(HEALTH).with(local.seq);
            local.credential = Some(credential);
            console.send(Command::Public {
                base,
                key,
                call: Call::new(|api| async move {
                    // The root cause (connection refused) says more than
                    // reqwest's "error sending request".
                    api.get_public::<IgnoredAny>("/api/v1/health")
                        .await
                        .map_err(|error| anyhow::anyhow!("{}", error.root_cause()))
                }),
            });
            cx.notify();
        }
    }
}

/// The last attempt failed: say why, and try again after the back-off.
fn wait(console: &mut Console, reason: String, cx: &mut Context<Console>) {
    let Some(local) = &mut console.station.local else {
        return;
    };
    local.failures = local.failures.saturating_add(1);
    // One line per attempt, for whoever started the console (CI greps it).
    eprintln!("voltrus-native: station: {reason}");
    local.status = Status::Unreachable(reason);
    let delay = station::retry_delay(local.failures);
    local.retry = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(delay).await;
        let _ = this.update(cx, check);
    }));
    cx.notify();
}

/// The answer to a health check: up, so sign in; else wait and ask again.
pub fn on_public_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(local) = &mut console.station.local else {
        return;
    };
    if key.name != HEALTH || key.arg != local.seq || local.retry.is_some() {
        return;
    }
    if let Some(error) = reply.error() {
        let reason = format!("The server is not answering: {error}");
        wait(console, reason, cx);
        query_service(console, cx);
        return;
    }
    // Up: whatever the service was, it serves.
    local.service = None;
    let Some(credential) = local.credential.clone() else {
        return;
    };
    if local.status == Status::Live {
        // Up and signed in already (a check after the session ended
        // raced a sign-in): nothing to do.
        return;
    }
    local.status = Status::SigningIn;
    console.request_sign_in(
        credential.url,
        credential.username,
        credential.password,
        window,
        cx,
    );
}

/// Signed in: in local mode, the station is live.
pub fn signed_in(console: &mut Console, server: &str, username: &str) {
    if let Some(local) = &mut console.station.local {
        local.status = Status::Live;
        local.failures = 0;
        local.retry = None;
        // For whoever started the console (CI greps it).
        eprintln!("voltrus-native: station: signed in to {server} as {username}");
    }
}

/// A sign-in failed. In local mode the card is not shown: a server that
/// did not answer is tried again after the back-off, one that `refused`
/// the credential is not (until Retry). `true` when handled so.
pub fn sign_in_failed(
    console: &mut Console,
    error: &str,
    refused: bool,
    cx: &mut Context<Console>,
) -> bool {
    let Some(local) = &mut console.station.local else {
        return false;
    };
    console.busy = false;
    if refused {
        let reason = format!("The server refused the station's sign-in: {error}");
        // For whoever started the console (CI greps it).
        eprintln!("voltrus-native: station: {reason}");
        local.retry = None;
        local.status = Status::Refused(reason);
        cx.notify();
    } else {
        wait(
            console,
            format!("Signing in to the station failed: {error}"),
            cx,
        );
    }
    true
}

/// The session ended (the server refused it): in local mode, sign in again.
pub fn session_ended(console: &mut Console, cx: &mut Context<Console>) {
    if let Some(local) = &mut console.station.local {
        local.status = Status::Connecting;
        local.failures = 0;
        check(console, cx);
    }
}

/// Ask the bundled server whether the station's service is installed and
/// running, off the UI thread: the station screen then offers to finish
/// the setup or start the service.
fn query_service(console: &mut Console, cx: &mut Context<Console>) {
    let (Some(server), Some(dir)) = (console.station.server.clone(), console.station.dir.clone())
    else {
        return;
    };
    let Some(local) = &mut console.station.local else {
        return;
    };
    if local.querying || matches!(local.fix, Some(Setup::Running(_))) {
        return;
    }
    local.querying = true;
    cx.spawn(async move |this, cx| {
        let service = cx
            .background_executor()
            .spawn(async move { station::service_status(&server, &dir) })
            .await;
        let _ = this.update(cx, |console, cx| {
            if let Some(local) = &mut console.station.local {
                local.querying = false;
                local.service = Some(service);
                cx.notify();
            }
        });
    })
    .detach();
}

/// Where a run of setup steps reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// The sign-in card's "Set up this computer as a station".
    Card,
    /// The station screen's "Finish setting up" or "Start the server".
    Screen,
}

/// The progress of a run for `target`; `None` once there is nowhere to
/// show it (local station mode ended).
fn progress(console: &mut Console, target: Target) -> Option<&mut Option<Setup>> {
    match target {
        Target::Card => Some(&mut console.station.setup),
        Target::Screen => console.station.local.as_mut().map(|local| &mut local.fix),
    }
}

/// "Set up this computer as a station": the bundled server's setup steps
/// one after the other, off the UI thread, then local station mode.
pub fn set_up(console: &mut Console, cx: &mut Context<Console>) {
    console.error = None;
    run_steps(console, Target::Card, Step::SETUP, cx);
}

/// The station screen's "Finish setting up this station"
/// ([`Step::FINISH`]) or "Start the station server" ([`Step::START`]),
/// then a health check at once.
pub fn repair(console: &mut Console, steps: &'static [Step], cx: &mut Context<Console>) {
    run_steps(console, Target::Screen, steps, cx);
}

fn run_steps(
    console: &mut Console,
    target: Target,
    steps: &'static [Step],
    cx: &mut Context<Console>,
) {
    let (Some(server), Some(dir), Some(first)) = (
        console.station.server.clone(),
        console.station.dir.clone(),
        steps.first().copied(),
    ) else {
        return;
    };
    let Some(slot) = progress(console, target) else {
        return;
    };
    if matches!(slot, Some(Setup::Running(_))) {
        return;
    }
    *slot = Some(Setup::Running(first));
    cx.spawn(async move |this, cx| {
        for &step in steps {
            let shown = this.update(cx, |console, cx| {
                let Some(slot) = progress(console, target) else {
                    return false;
                };
                *slot = Some(Setup::Running(step));
                cx.notify();
                true
            });
            if !matches!(shown, Ok(true)) {
                return;
            }
            let (server, dir) = (server.clone(), dir.clone());
            let result = cx
                .background_executor()
                .spawn(async move { station::run(&server, step, &dir) })
                .await;
            if let Err(error) = result {
                let _ = this.update(cx, |console, cx| {
                    if let Some(slot) = progress(console, target) {
                        *slot = Some(Setup::Failed(error));
                    }
                    console.station.look();
                    cx.notify();
                });
                return;
            }
        }
        let _ = this.update(cx, |console, cx| {
            console.station.look();
            match target {
                Target::Card => {
                    console.station.setup = None;
                    open(console, cx);
                }
                Target::Screen => {
                    if let Some(local) = &mut console.station.local {
                        local.fix = None;
                        local.service = None;
                        local.failures = 0;
                        check(console, cx);
                    }
                }
            }
        });
    })
    .detach();
    cx.notify();
}
