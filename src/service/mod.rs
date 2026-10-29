//! `voltrus service install|uninstall|start|stop|status|run`: run the local
//! station server ([`crate::station`]) as an OS background service, so closing
//! the console never stops polling, alarms, history or writes.
//!
//! - Windows: the Service Control Manager service `VoltrusSCADA` (LocalSystem,
//!   delayed auto-start, restart on failure).
//! - macOS: the per-user LaunchAgent `id.voltrus.scada.server`.
//! - Other systems: install the systemd unit in `deploy/voltrus.service`.
//!
//! `service run` is what the service manager executes: it starts the server
//! from the station directory, logging to `logs/voltrus.log`.

use crate::edition::Extension;
use crate::station;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(any(target_os = "macos", all(test, unix)))]
mod macos;
#[cfg(windows)]
mod windows;

/// Whether the service is installed and running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    Stopped,
    Running,
}

impl Status {
    /// Exit code of `voltrus service status`.
    pub fn exit_code(self) -> i32 {
        match self {
            Status::Running => 0,
            Status::Stopped => 3,
            Status::NotInstalled => 4,
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Status::Running => "installed, running",
            Status::Stopped => "installed, stopped",
            Status::NotInstalled => "not installed",
        }
    }
}

/// The paths a service run needs, all under the station directory.
pub(crate) struct Station {
    pub dir: PathBuf,
    pub config: PathBuf,
    pub log_file: PathBuf,
}

impl Station {
    fn new(dir: &Path) -> Result<Self> {
        let dir = std::path::absolute(dir)
            .with_context(|| format!("Cannot resolve station directory {}", dir.display()))?;
        Ok(Self {
            config: dir.join(station::CONFIG_FILE),
            log_file: dir.join(station::LOG_DIR).join(station::LOG_FILE),
            dir,
        })
    }

    /// Fail unless `voltrus station init` has set this directory up.
    fn ensure_initialized(&self) -> Result<()> {
        anyhow::ensure!(
            self.config.is_file(),
            "{} not found. Run `voltrus station init --dir \"{}\"` first.",
            self.config.display(),
            self.dir.display()
        );
        std::fs::create_dir_all(self.dir.join(station::LOG_DIR)).with_context(|| {
            format!(
                "Cannot create {}",
                self.dir.join(station::LOG_DIR).display()
            )
        })
    }
}

/// `voltrus service run [--dir D]`: start the server from the station
/// directory with its log file. On Windows it hands control to the service
/// dispatcher when the SCM started it; elsewhere (and when started from a
/// console) it runs in the foreground until SIGTERM, Ctrl-C or a console close.
pub(crate) async fn run(ext: Arc<dyn Extension>, dir: &Path) -> Result<()> {
    #[cfg(not(windows))]
    let shutdown = crate::app::shutdown_signal();
    let station = Station::new(dir)?;
    station.ensure_initialized()?;
    std::env::set_current_dir(&station.dir)
        .with_context(|| format!("Cannot enter {}", station.dir.display()))?;
    crate::app::init_tracing(Some(&station.log_file))?;
    tracing::info!(dir = %station.dir.display(), "Voltrus station service starting");

    #[cfg(windows)]
    {
        windows::run(ext, station).await
    }
    #[cfg(not(windows))]
    {
        let config = station
            .config
            .to_str()
            .context("Config path is not valid UTF-8")?;
        crate::app::serve(ext, config, shutdown, None).await
    }
}

/// `voltrus service install|uninstall|start|stop|status [--dir D]`.
pub(crate) fn handle_cli(args: &[String]) -> bool {
    let usage = || -> ! {
        eprintln!("Usage: voltrus service install|uninstall|start|stop|status|run [--dir DIR]");
        std::process::exit(1);
    };
    let Some(action) = args.get(2) else { usage() };
    let result = station::dir_arg(&args[3..])
        .and_then(|dir| Station::new(&dir))
        .and_then(|station| match action.as_str() {
            "install" => {
                station.ensure_initialized()?;
                install(&station)?;
                println!(
                    "Installed the Voltrus SCADA service for {}",
                    station.dir.display()
                );
                Ok(0)
            }
            "uninstall" => {
                uninstall()?;
                println!("Removed the Voltrus SCADA service");
                Ok(0)
            }
            "start" => {
                start(&station)?;
                println!("Voltrus SCADA service is running");
                Ok(0)
            }
            "stop" => {
                stop()?;
                println!("Voltrus SCADA service is stopped");
                Ok(0)
            }
            "status" => {
                let status = status()?;
                println!("Voltrus SCADA service: {}", status.describe());
                Ok(status.exit_code())
            }
            _ => usage(),
        });
    match result {
        Ok(0) => true,
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("Error: {e:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(windows)]
use windows::{install, start, status, stop, uninstall};

#[cfg(target_os = "macos")]
use macos::{install, start, status, stop, uninstall};

#[cfg(not(any(windows, target_os = "macos")))]
use unsupported::{install, start, status, stop, uninstall};

#[cfg(not(any(windows, target_os = "macos")))]
mod unsupported {
    use super::{Station, Status};
    use anyhow::Result;

    fn unsupported<T>() -> Result<T> {
        anyhow::bail!(
            "`voltrus service` manages Windows services and macOS launch agents. \
             On this system install the systemd unit in deploy/voltrus.service and \
             point it at `voltrus service run --dir <station dir>`."
        )
    }

    pub(super) fn install(_station: &Station) -> Result<()> {
        unsupported()
    }

    pub(super) fn uninstall() -> Result<()> {
        unsupported()
    }

    pub(super) fn start(_station: &Station) -> Result<()> {
        unsupported()
    }

    pub(super) fn stop() -> Result<()> {
        unsupported()
    }

    pub(super) fn status() -> Result<Status> {
        unsupported()
    }
}
