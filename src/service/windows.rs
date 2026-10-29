//! Windows: the server as the Service Control Manager service `VoltrusSCADA`
//! (LocalSystem, delayed auto-start, restarted on failure).

use super::{Station, Status};
use crate::edition::Extension;
use anyhow::{Context, Result};
use std::ffi::{OsStr, OsString};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

pub const SERVICE_NAME: &str = "VoltrusSCADA";
const DISPLAY_NAME: &str = "Voltrus SCADA Server";
const DESCRIPTION: &str = "Voltrus SCADA local station server: device polling, alarms, \
                           history and writes for the console on this computer.";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;
/// How long `start`, `stop` and `uninstall` wait for the SCM.
const WAIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(250);
/// Pending-state wait hint; the checkpoint is raised every PROGRESS while the
/// server starts up or drains connections (up to 3 s).
const PENDING_WAIT_HINT: Duration = Duration::from_secs(10);
const PROGRESS: Duration = Duration::from_secs(2);

const ERROR_SERVICE_ALREADY_RUNNING: i32 = 1056;
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
const ERROR_SERVICE_NOT_ACTIVE: i32 = 1062;
const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;
const ERROR_SERVICE_MARKED_FOR_DELETE: i32 = 1072;

/// What the SCM's service thread needs from the `service run` invocation.
struct ServiceContext {
    ext: Arc<dyn Extension>,
    config: String,
    runtime: tokio::runtime::Handle,
}

static CONTEXT: OnceLock<ServiceContext> = OnceLock::new();

windows_service::define_windows_service!(ffi_service_main, service_main);

/// Hand the process to the service dispatcher. When `voltrus service run` was
/// started from a console instead of by the SCM, run in the foreground.
pub(super) async fn run(ext: Arc<dyn Extension>, station: Station) -> Result<()> {
    let config = station
        .config
        .to_str()
        .context("Config path is not valid UTF-8")?
        .to_string();
    let context = ServiceContext {
        ext: ext.clone(),
        config: config.clone(),
        runtime: tokio::runtime::Handle::current(),
    };
    anyhow::ensure!(
        CONTEXT.set(context).is_ok(),
        "The service is already running in this process"
    );
    // The dispatcher blocks until the service stops; the SCM calls
    // `service_main` on a thread of its own, which drives the server on this
    // runtime.
    let dispatched =
        tokio::task::spawn_blocking(|| service_dispatcher::start(SERVICE_NAME, ffi_service_main))
            .await
            .context("Service dispatcher thread failed")?;
    match dispatched {
        Ok(()) => Ok(()),
        Err(e) if os_error(&e) == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) => {
            tracing::info!("Not started by the Service Control Manager; running in the foreground");
            crate::app::serve(ext, &config, crate::app::shutdown_signal(), None).await
        }
        Err(e) => Err(e).context("Cannot start the service dispatcher"),
    }
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        tracing::error!("Service failed: {e:#}");
    }
}

enum Event {
    /// The server's listener is bound.
    Ready,
    StopRequested,
    Finished(Result<()>),
}

/// Report StartPending until the server listens, then Running; StopPending
/// while it drains; Stopped (with a service-specific exit code when the server
/// failed or panicked, so the recovery actions fire) when it is done.
fn run_service() -> Result<()> {
    let context = CONTEXT.get().context("Service context missing")?;
    let (events_tx, events) = mpsc::channel();

    let control_tx = events_tx.clone();
    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = control_tx.send(Event::StopRequested);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })
        .context("Cannot register the service control handler")?;
    let mut state = ServiceState::StartPending;
    let mut checkpoint = 1;
    set_status(&status_handle, state, ServiceExitCode::Win32(0), checkpoint)?;

    // The server runs on a thread of its own so this one stays free to report
    // progress to the SCM.
    let stop = Arc::new(tokio::sync::Notify::new());
    let server_stop = stop.clone();
    let runtime = context.runtime.clone();
    let ext = context.ext.clone();
    let config = context.config.clone();
    std::thread::Builder::new()
        .name("voltrus-server".to_string())
        .spawn(move || {
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let ready_events = events_tx.clone();
            runtime.spawn(async move {
                if ready_rx.await.is_ok() {
                    let _ = ready_events.send(Event::Ready);
                }
            });
            let shutdown = async move { server_stop.notified().await };
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                runtime.block_on(crate::app::serve(ext, &config, shutdown, Some(ready_tx)))
            }))
            .unwrap_or_else(|panic| {
                Err(anyhow::anyhow!(
                    "The server panicked: {}",
                    panic_message(panic.as_ref())
                ))
            });
            let _ = events_tx.send(Event::Finished(result));
        })
        .context("Cannot start the server thread")?;

    let result = loop {
        match events.recv_timeout(PROGRESS) {
            Ok(Event::Ready) => {
                tracing::info!("Service running");
                state = ServiceState::Running;
                checkpoint = 0;
                set_status(&status_handle, state, ServiceExitCode::Win32(0), checkpoint)?;
            }
            Ok(Event::StopRequested) => {
                tracing::info!("Stop requested by the Service Control Manager");
                state = ServiceState::StopPending;
                checkpoint = 1;
                set_status(&status_handle, state, ServiceExitCode::Win32(0), checkpoint)?;
                stop.notify_one();
            }
            Ok(Event::Finished(result)) => break result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if state != ServiceState::Running {
                    checkpoint += 1;
                    set_status(&status_handle, state, ServiceExitCode::Win32(0), checkpoint)?;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err(anyhow::anyhow!("The server thread ended without a result"));
            }
        }
    };
    let exit_code = match &result {
        Ok(()) => ServiceExitCode::Win32(0),
        Err(e) => {
            tracing::error!("Server stopped with an error: {e:#}");
            ServiceExitCode::ServiceSpecific(1)
        }
    };
    set_status(&status_handle, ServiceState::Stopped, exit_code, 0)
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

fn set_status(
    handle: &ServiceStatusHandle,
    state: ServiceState,
    exit_code: ServiceExitCode,
    checkpoint: u32,
) -> Result<()> {
    let (controls_accepted, wait_hint) = match state {
        ServiceState::Running => (
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            Duration::default(),
        ),
        ServiceState::StartPending | ServiceState::StopPending => {
            (ServiceControlAccept::empty(), PENDING_WAIT_HINT)
        }
        _ => (ServiceControlAccept::empty(), Duration::default()),
    };
    handle
        .set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: state,
            controls_accepted,
            exit_code,
            checkpoint,
            wait_hint,
            process_id: None,
        })
        .with_context(|| format!("Cannot report service state {state:?}"))
}

fn os_error(e: &windows_service::Error) -> Option<i32> {
    match e {
        windows_service::Error::Winapi(io) => io.raw_os_error(),
        _ => None,
    }
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access)
        .context("Cannot open the Service Control Manager (run as administrator)")
}

/// The installed service, or `None` when it is not installed.
fn open(access: ServiceAccess) -> Result<Option<Service>> {
    match manager(ServiceManagerAccess::CONNECT)?.open_service(SERVICE_NAME, access) {
        Ok(service) => Ok(Some(service)),
        Err(e) if os_error(&e) == Some(ERROR_SERVICE_DOES_NOT_EXIST) => Ok(None),
        Err(e) => Err(e).context("Cannot open the Voltrus SCADA service"),
    }
}

fn state(service: &Service) -> Result<ServiceState> {
    Ok(service
        .query_status()
        .context("Cannot query the service status")?
        .current_state)
}

/// Poll until the service leaves the pending states; return where it ended.
fn settle(service: &Service) -> Result<ServiceState> {
    let deadline = Instant::now() + WAIT;
    loop {
        let current = state(service)?;
        if !matches!(
            current,
            ServiceState::StartPending | ServiceState::StopPending | ServiceState::ContinuePending
        ) {
            return Ok(current);
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "Timed out waiting for the service (still {current:?})"
        );
        std::thread::sleep(POLL);
    }
}

/// Install the service, or point an installed one at this executable and
/// station directory.
pub(super) fn install(station: &Station) -> Result<()> {
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe().context("Cannot locate the voltrus executable")?,
        launch_arguments: vec![
            OsString::from("service"),
            OsString::from("run"),
            OsString::from("--dir"),
            station.dir.clone().into_os_string(),
        ],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };
    let manager = manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::CHANGE_CONFIG;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => {
            service
                .change_config(&info)
                .context("Cannot update the service")?;
            service
        }
        Err(e) if os_error(&e) == Some(ERROR_SERVICE_DOES_NOT_EXIST) => manager
            .create_service(&info, access)
            .context("Cannot create the service")?,
        Err(e) => return Err(e).context("Cannot open the Voltrus SCADA service"),
    };
    service
        .set_description(DESCRIPTION)
        .context("Cannot set the service description")?;
    service
        .set_delayed_auto_start(true)
        .context("Cannot set delayed auto-start")?;
    let restart = |secs| ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(secs),
    };
    service
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![restart(5), restart(10), restart(30)]),
        })
        .context("Cannot set the service recovery actions")?;
    service
        .set_failure_actions_on_non_crash_failures(true)
        .context("Cannot set the service recovery actions")?;
    Ok(())
}

pub(super) fn uninstall() -> Result<()> {
    let Some(service) =
        open(ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)?
    else {
        return Ok(());
    };
    stop_service(&service)?;
    match service.delete() {
        Err(e) if os_error(&e) != Some(ERROR_SERVICE_MARKED_FOR_DELETE) => {
            Err(e).context("Cannot delete the service")
        }
        _ => Ok(()),
    }
}

/// Start the service and return once it reports Running.
pub(super) fn start(station: &Station) -> Result<()> {
    let service = open(ServiceAccess::QUERY_STATUS | ServiceAccess::START)?
        .context("The service is not installed. Run `voltrus service install` first.")?;
    if settle(&service)? == ServiceState::Running {
        return Ok(());
    }
    if let Err(e) = service.start::<&OsStr>(&[])
        && os_error(&e) != Some(ERROR_SERVICE_ALREADY_RUNNING)
    {
        return Err(e).context("Cannot start the service");
    }
    std::thread::sleep(POLL);
    let ended = settle(&service)?;
    anyhow::ensure!(
        ended == ServiceState::Running,
        "The service did not start (now {ended:?}); see {}",
        station.log_file.display()
    );
    Ok(())
}

/// Stop the service and return once it reports Stopped. Stopping a stopped
/// or uninstalled service succeeds.
pub(super) fn stop() -> Result<()> {
    match open(ServiceAccess::QUERY_STATUS | ServiceAccess::STOP)? {
        Some(service) => stop_service(&service),
        None => Ok(()),
    }
}

fn stop_service(service: &Service) -> Result<()> {
    if settle(service)? == ServiceState::Stopped {
        return Ok(());
    }
    if let Err(e) = service.stop()
        && os_error(&e) != Some(ERROR_SERVICE_NOT_ACTIVE)
    {
        return Err(e).context("Cannot stop the service");
    }
    std::thread::sleep(POLL);
    let ended = settle(service)?;
    anyhow::ensure!(
        ended == ServiceState::Stopped,
        "The service did not stop (now {ended:?})"
    );
    Ok(())
}

pub(super) fn status() -> Result<Status> {
    Ok(match open(ServiceAccess::QUERY_STATUS)? {
        None => Status::NotInstalled,
        Some(service) if state(&service)? == ServiceState::Running => Status::Running,
        Some(_) => Status::Stopped,
    })
}
