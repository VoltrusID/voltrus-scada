//! macOS: the server as the per-user LaunchAgent `id.voltrus.scada.server`,
//! managed with `launchctl` in the `gui/<uid>` domain.

use super::{Station, Status};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

pub const LABEL: &str = "id.voltrus.scada.server";
/// How long `stop` and `uninstall` wait for launchd.
const WAIT: Duration = Duration::from_secs(20);
/// How long `start` waits for the server to answer its health check.
const READY_WAIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(250);

/// The LaunchAgent property list. `KeepAlive` restarts the server whenever it
/// does not exit cleanly (a crash or a kill), while a clean exit after
/// `voltrus service stop` keeps it stopped until the next start or login.
pub(super) fn render_plist(exe: &Path, dir: &Path) -> String {
    let logs = dir.join(crate::station::LOG_DIR);
    let string = |path: &Path| format!("<string>{}</string>", xml_escape(&path.to_string_lossy()));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		{exe}
		<string>service</string>
		<string>run</string>
		<string>--dir</string>
		{dir}
	</array>
	<key>WorkingDirectory</key>
	{dir}
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ProcessType</key>
	<string>Standard</string>
	<key>StandardOutPath</key>
	{stdout}
	<key>StandardErrorPath</key>
	{stderr}
</dict>
</plist>
"#,
        exe = string(exe),
        dir = string(dir),
        stdout = string(&logs.join("service.out.log")),
        stderr = string(&logs.join("service.err.log")),
    )
}

fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c => escaped.push(c),
        }
    }
    escaped
}

fn plist_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

fn domain() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("Cannot run `id -u`")?;
    anyhow::ensure!(output.status.success(), "`id -u` failed");
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(format!("gui/{uid}"))
}

fn launchctl(args: &[&str]) -> Result<Output> {
    Command::new("launchctl")
        .args(args)
        .output()
        .context("Cannot run launchctl")
}

/// Run launchctl and fail with its message unless it succeeds.
fn launchctl_ok(args: &[&str]) -> Result<()> {
    let output = launchctl(args)?;
    anyhow::ensure!(
        output.status.success(),
        "launchctl {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

/// The agent as launchd sees it: `None` when it is not loaded.
fn loaded_state(domain: &str) -> Result<Option<bool>> {
    let output = launchctl(&["print", &format!("{domain}/{LABEL}")])?;
    if !output.status.success() {
        return Ok(None);
    }
    let running = String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim() == "state = running");
    Ok(Some(running))
}

fn wait_until(what: &str, mut done: impl FnMut() -> Result<bool>) -> Result<()> {
    let deadline = Instant::now() + WAIT;
    while !done()? {
        anyhow::ensure!(
            Instant::now() < deadline,
            "Timed out waiting for the service to {what}"
        );
        std::thread::sleep(POLL);
    }
    Ok(())
}

fn bootout(domain: &str) -> Result<()> {
    if loaded_state(domain)?.is_some() {
        launchctl_ok(&["bootout", &format!("{domain}/{LABEL}")])?;
        wait_until("unload", || Ok(loaded_state(domain)?.is_none()))?;
    }
    Ok(())
}

pub(super) fn install(station: &Station) -> Result<()> {
    let exe = std::env::current_exe().context("Cannot locate the voltrus executable")?;
    let plist = plist_path()?;
    let domain = domain()?;
    bootout(&domain)?;
    if let Some(dir) = plist.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("Cannot create {}", dir.display()))?;
    }
    std::fs::write(&plist, render_plist(&exe, &station.dir))
        .with_context(|| format!("Cannot write {}", plist.display()))?;
    launchctl_ok(&["bootstrap", &domain, &plist.to_string_lossy()])
}

pub(super) fn uninstall() -> Result<()> {
    bootout(&domain()?)?;
    let plist = plist_path()?;
    match std::fs::remove_file(&plist) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("Cannot remove {}", plist.display()))
        }
        _ => Ok(()),
    }
}

/// Start the agent and return once the server answers `/api/v1/health`.
pub(super) fn start(station: &Station) -> Result<()> {
    station.ensure_initialized()?;
    let port = crate::config::load_config(
        station
            .config
            .to_str()
            .context("Config path is not valid UTF-8")?,
    )?
    .server_port;
    let domain = domain()?;
    match status()? {
        Status::NotInstalled => {
            anyhow::bail!("The service is not installed. Run `voltrus service install` first.")
        }
        Status::Running => {}
        Status::Stopped => match loaded_state(&domain)? {
            // RunAtLoad starts it as it loads.
            None => launchctl_ok(&["bootstrap", &domain, &plist_path()?.to_string_lossy()])?,
            Some(_) => launchctl_ok(&["kickstart", "-k", &format!("{domain}/{LABEL}")])?,
        },
    }
    let deadline = Instant::now() + READY_WAIT;
    while !healthy(port) {
        anyhow::ensure!(
            Instant::now() < deadline,
            "The service did not answer on 127.0.0.1:{port} within {}s; see {}",
            READY_WAIT.as_secs(),
            station.log_file.display()
        );
        std::thread::sleep(POLL);
    }
    Ok(())
}

/// `GET /api/v1/health` on 127.0.0.1:`port` answers 200.
fn healthy(port: u16) -> bool {
    use std::io::{Read, Write};
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)) else {
        return false;
    };
    let timeout = Some(Duration::from_secs(2));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return false;
    }
    let request = "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut status_line = [0u8; 12];
    stream.read_exact(&mut status_line).is_ok() && status_line == *b"HTTP/1.1 200"
}

pub(super) fn stop() -> Result<()> {
    let domain = domain()?;
    if loaded_state(&domain)? != Some(true) {
        return Ok(());
    }
    launchctl_ok(&["kill", "TERM", &format!("{domain}/{LABEL}")])?;
    wait_until("stop", || Ok(loaded_state(&domain)? != Some(true)))
}

pub(super) fn status() -> Result<Status> {
    let installed = plist_path()?.is_file();
    Ok(match loaded_state(&domain()?)? {
        Some(true) => Status::Running,
        Some(false) => Status::Stopped,
        None if installed => Status::Stopped,
        None => Status::NotInstalled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_runs_the_service_from_the_station_dir() {
        let plist = render_plist(
            Path::new("/Applications/Voltrus SCADA.app/Contents/MacOS/voltrus"),
            Path::new("/Users/op/Library/Application Support/Voltrus <&> SCADA"),
        );
        let dir = "/Users/op/Library/Application Support/Voltrus &lt;&amp;&gt; SCADA";
        assert!(plist.contains(&format!("<key>Label</key>\n\t<string>{LABEL}</string>")));
        assert!(plist.contains(&format!(
            "<array>\n\t\t<string>/Applications/Voltrus SCADA.app/Contents/MacOS/voltrus</string>\n\
             \t\t<string>service</string>\n\t\t<string>run</string>\n\
             \t\t<string>--dir</string>\n\t\t<string>{dir}</string>\n\t</array>"
        )));
        assert!(plist.contains(&format!(
            "<key>WorkingDirectory</key>\n\t<string>{dir}</string>"
        )));
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains(&format!("<string>{dir}/logs/service.out.log</string>")));
        assert!(plist.contains(&format!("<string>{dir}/logs/service.err.log</string>")));
        assert!(!plist.contains("<&>"));
    }

    fn serve_once(response: &'static str) -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 256];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(response.as_bytes());
        });
        port
    }

    #[test]
    fn health_probe_needs_a_200() {
        assert!(healthy(serve_once(
            "HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n"
        )));
        assert!(!healthy(serve_once(
            "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n"
        )));
        let closed = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        assert!(!healthy(port));
    }
}
