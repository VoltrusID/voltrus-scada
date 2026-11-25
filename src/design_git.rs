//! Optional git sync of the project (docs/version-history.md).
//!
//! On publish (or on demand) the server exports the project as
//! `voltrus-project/1` YAML, with each screen's published rows, into a working
//! copy and commits and pushes it to a configured remote (GitHub, Gitea, a
//! bare repository on a share). It shells out to the `git` command line: the
//! single binary carries no git library, and a site without git still gets
//! the export written to the working folder (reported as not pushed).
//!
//! Credentials (an HTTPS token or an SSH deploy key) are stored encrypted at
//! rest like the copilot's API key, never put on a command line, and handed
//! to git through its environment. Pull stays manual: a file from git is
//! imported through the ordinary project import plan.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::security::secrets::{decrypt_secret, encrypt_secret};
use crate::storage::Storage;

/// Setting with the [`GitConfig`].
pub const CONFIG_KEY: &str = "design_git";
/// Setting with the encrypted token or deploy key.
pub const SECRET_KEY: &str = "design_git_secret";
/// Setting with the outcome of the last sync.
pub const STATUS_KEY: &str = "design_git_status";
const PURPOSE: &str = "design_git_secret";
/// The longest one git command may run.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// How the server authenticates to the remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GitAuth {
    /// A local path, `file://`, or a remote that needs no credentials.
    #[default]
    None,
    /// HTTPS with a personal access or deploy token (sent as basic auth).
    Token,
    /// SSH with a deploy key (private key in OpenSSH format).
    SshKey,
}

/// Git sync settings (setting `design_git`). The secret is kept apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GitConfig {
    pub enabled: bool,
    /// Remote URL (`https://…`, `ssh://…`, `git@host:org/repo.git`) or the
    /// path of a bare repository.
    pub remote: String,
    pub branch: String,
    /// File inside the repository the project is written to.
    pub path: String,
    pub auth: GitAuth,
    /// User name for token auth (`x-access-token` for GitHub, your user for Gitea).
    pub username: String,
    /// Push after every publish (otherwise only *Sync now*).
    pub push_on_publish: bool,
    /// Commits are authored `<user>@<domain>`.
    pub author_email_domain: String,
    /// Working copy; empty: `design-git/` beside the database.
    pub work_dir: String,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            remote: String::new(),
            branch: "main".into(),
            path: "project.yaml".into(),
            auth: GitAuth::None,
            username: String::new(),
            push_on_publish: true,
            author_email_domain: "voltrus.local".into(),
            work_dir: String::new(),
        }
    }
}

impl GitConfig {
    /// Refuse values git would read as options or that escape the working copy.
    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.remote.trim().is_empty() {
            bail!("remote is required when git sync is enabled");
        }
        if self.remote.starts_with('-') || self.branch.starts_with('-') {
            bail!("remote and branch must not start with '-'");
        }
        if self.remote.contains("::") {
            bail!("remote helpers (`transport::address`) are not allowed");
        }
        if self.branch.trim().is_empty()
            || self
                .branch
                .chars()
                .any(|c| c.is_whitespace() || "~^:?*[\\".contains(c))
            || self.branch.contains("..")
        {
            bail!("branch '{}' is not a valid branch name", self.branch);
        }
        let p = Path::new(&self.path);
        if self.path.trim().is_empty()
            || p.is_absolute()
            || p.components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || self.path.starts_with(".git")
        {
            bail!("path must be a relative file path inside the repository");
        }
        Ok(())
    }
}

/// Why a sync runs, for the commit.
#[derive(Debug, Clone)]
pub struct SyncReason {
    /// Commit message.
    pub message: String,
    /// The person the commit is authored by.
    pub author: String,
}

/// What a sync did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncOutcome {
    /// HEAD after the sync, `None` without git.
    pub commit: Option<String>,
    /// A new commit was made (false: the project had not changed).
    pub committed: bool,
    pub pushed: bool,
    /// Where the file was written.
    pub file: String,
    pub message: String,
}

pub async fn load_config(storage: &Storage) -> Result<GitConfig> {
    let rows = storage.get_settings(&[CONFIG_KEY]).await?;
    Ok(rows
        .first()
        .and_then(|r| serde_json::from_str(&r.value).ok())
        .unwrap_or_default())
}

pub async fn save_config(storage: &Storage, c: &GitConfig) -> Result<()> {
    c.validate()?;
    storage
        .upsert_setting(CONFIG_KEY, &serde_json::to_string(c)?)
        .await
}

pub async fn save_secret(storage: &Storage, auth_secret: &str, secret: Option<&str>) -> Result<()> {
    match secret {
        Some(s) if !s.is_empty() => {
            let stored = encrypt_secret(auth_secret, PURPOSE, s)?;
            storage.upsert_setting(SECRET_KEY, &stored).await
        }
        _ => storage.upsert_setting(SECRET_KEY, "").await,
    }
}

async fn load_secret(storage: &Storage, auth_secret: &str) -> Result<Option<String>> {
    let rows = storage.get_settings(&[SECRET_KEY]).await?;
    match rows.first().map(|r| r.value.clone()).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(stored) => decrypt_secret(auth_secret, PURPOSE, &stored)
            .map(Some)
            .map_err(|_| anyhow!("the stored git credential cannot be read (the server's auth secret changed); enter it again")),
    }
}

pub async fn has_secret(storage: &Storage) -> bool {
    storage
        .get_settings(&[SECRET_KEY])
        .await
        .ok()
        .and_then(|r| r.first().map(|r| !r.value.is_empty()))
        .unwrap_or(false)
}

pub async fn last_status(storage: &Storage) -> Value {
    storage
        .get_settings(&[STATUS_KEY])
        .await
        .ok()
        .and_then(|r| r.first().and_then(|r| serde_json::from_str(&r.value).ok()))
        .unwrap_or(Value::Null)
}

fn work_dir(storage: &Storage, c: &GitConfig) -> PathBuf {
    if !c.work_dir.trim().is_empty() {
        return PathBuf::from(&c.work_dir);
    }
    let db = Path::new(storage.db_path());
    match db.parent() {
        Some(dir) if storage.db_path() != ":memory:" => dir.join("design-git"),
        _ => std::env::temp_dir().join(format!("voltrus-design-git-{}", std::process::id())),
    }
}

/// Serialises syncs: one working copy, one git at a time.
static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Export the project (published rows) and commit and push it.
pub async fn sync(
    storage: &Storage,
    auth_secret: &str,
    reason: &SyncReason,
) -> Result<SyncOutcome> {
    let _guard = SYNC_LOCK.lock().await;
    let result = sync_locked(storage, auth_secret, reason).await;
    let status = match &result {
        Ok(o) => {
            json!({"ok": true, "at_ms": crate::timeutil::now_millis(), "outcome": o, "reason": reason.message})
        }
        Err(e) => {
            json!({"ok": false, "at_ms": crate::timeutil::now_millis(), "error": format!("{e:#}"), "reason": reason.message})
        }
    };
    let _ = storage
        .upsert_setting(STATUS_KEY, &status.to_string())
        .await;
    result
}

async fn sync_locked(
    storage: &Storage,
    auth_secret: &str,
    reason: &SyncReason,
) -> Result<SyncOutcome> {
    let c = load_config(storage).await?;
    c.validate()?;
    if !c.enabled {
        bail!("git sync is not enabled");
    }
    let doc = crate::project_yaml::export::export_project_yaml_with(storage, true).await?;
    let yaml = crate::project_yaml::to_yaml(&doc);
    let dir = work_dir(storage, &c);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let file = dir.join(&c.path);
    let file_text = file.display().to_string();

    if !git_available().await {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file, &yaml)?;
        return Ok(SyncOutcome {
            commit: None,
            committed: false,
            pushed: false,
            file: file_text,
            message:
                "git is not installed on this server: the project was exported to the folder only"
                    .into(),
        });
    }

    let secret = match c.auth {
        GitAuth::None => None,
        _ => Some(load_secret(storage, auth_secret).await?.ok_or_else(|| {
            anyhow!(
                "git sync uses {:?} auth but no credential is stored",
                c.auth
            )
        })?),
    };
    let git = Git::new(&dir, &c, secret.as_deref())?;

    if !dir.join(".git").exists() {
        git.run(&["init", "-q"]).await?;
    }
    let has_origin = git.run(&["remote", "get-url", "origin"]).await.is_ok();
    if has_origin {
        git.run(&["remote", "set-url", "origin", &c.remote]).await?;
    } else {
        git.run(&["remote", "add", "origin", &c.remote]).await?;
    }
    // Start from the remote branch when it exists: the working copy is ours,
    // the remote is the truth.
    let remote_ref = format!("refs/remotes/origin/{}", c.branch);
    let fetched = git
        .run(&[
            "fetch",
            "-q",
            "origin",
            &format!("+refs/heads/{}:{remote_ref}", c.branch),
        ])
        .await
        .is_ok();
    if fetched {
        git.run(&["checkout", "-q", "-B", &c.branch, &remote_ref])
            .await?;
        git.run(&["reset", "-q", "--hard", &remote_ref]).await?;
    } else {
        git.run(&["checkout", "-q", "-B", &c.branch]).await?;
    }

    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // The export's `generated_at` alone is no change worth a commit.
    let same = std::fs::read_to_string(&file)
        .is_ok_and(|old| without_timestamp(&old) == without_timestamp(&yaml));
    if !same {
        std::fs::write(&file, &yaml)?;
    }
    git.run(&["add", "--", &c.path]).await?;
    let unchanged = git.run(&["diff", "--cached", "--quiet"]).await.is_ok()
        && git
            .run(&["rev-parse", "-q", "--verify", "HEAD"])
            .await
            .is_ok();
    let mut committed = false;
    if !unchanged {
        let email = format!(
            "{}@{}",
            reason
                .author
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() || "._-".contains(ch) {
                    ch
                } else {
                    '_'
                })
                .collect::<String>(),
            c.author_email_domain
        );
        git.run_env(
            &["commit", "-q", "--no-verify", "-m", &reason.message],
            &[
                ("GIT_AUTHOR_NAME", reason.author.as_str()),
                ("GIT_AUTHOR_EMAIL", email.as_str()),
                ("GIT_COMMITTER_NAME", "Voltrus SCADA"),
                ("GIT_COMMITTER_EMAIL", email.as_str()),
            ],
        )
        .await?;
        committed = true;
    }
    let mut pushed = false;
    if committed || !fetched {
        git.run(&[
            "push",
            "-q",
            "origin",
            &format!("HEAD:refs/heads/{}", c.branch),
        ])
        .await
        .context("git push")?;
        pushed = true;
    }
    let commit = git
        .run(&["rev-parse", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_string());
    Ok(SyncOutcome {
        commit,
        committed,
        pushed,
        file: file_text,
        message: if committed {
            "committed and pushed".into()
        } else {
            "the project has not changed since the last commit".into()
        },
    })
}

fn without_timestamp(yaml: &str) -> String {
    yaml.lines()
        .filter(|l| !l.starts_with("generated_at:"))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn git_available() -> bool {
    tokio::process::Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// One working copy and the environment its commands run with.
struct Git {
    dir: PathBuf,
    env: Vec<(String, String)>,
    /// Deploy key file, deleted on drop.
    key_file: Option<PathBuf>,
}

impl Drop for Git {
    fn drop(&mut self) {
        if let Some(f) = &self.key_file {
            let _ = std::fs::remove_file(f);
        }
    }
}

impl Git {
    fn new(dir: &Path, c: &GitConfig, secret: Option<&str>) -> Result<Self> {
        let mut env: Vec<(String, String)> = vec![
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            // Only ordinary transports: no `ext::` or other helpers.
            (
                "GIT_ALLOW_PROTOCOL".into(),
                "file:git:http:https:ssh".into(),
            ),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_COUNT".into(), "2".into()),
            ("GIT_CONFIG_KEY_0".into(), "credential.helper".into()),
            ("GIT_CONFIG_VALUE_0".into(), String::new()),
            ("GIT_CONFIG_KEY_1".into(), "core.hooksPath".into()),
            ("GIT_CONFIG_VALUE_1".into(), "/dev/null".into()),
        ];
        let mut key_file = None;
        match (c.auth, secret) {
            (GitAuth::Token, Some(token)) => {
                let user = if c.username.is_empty() {
                    "x-access-token"
                } else {
                    &c.username
                };
                let basic =
                    base64::engine::general_purpose::STANDARD.encode(format!("{user}:{token}"));
                env.push(("GIT_CONFIG_COUNT".into(), "3".into()));
                env.push(("GIT_CONFIG_KEY_2".into(), "http.extraHeader".into()));
                env.push((
                    "GIT_CONFIG_VALUE_2".into(),
                    format!("Authorization: Basic {basic}"),
                ));
            }
            (GitAuth::SshKey, Some(key)) => {
                let path = dir.with_file_name(format!(
                    ".{}-key",
                    dir.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("design-git")
                ));
                write_private(&path, key)?;
                let known = dir.with_file_name(format!(
                    ".{}-known_hosts",
                    dir.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("design-git")
                ));
                env.push((
                    "GIT_SSH_COMMAND".into(),
                    format!(
                        "ssh -i '{}' -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile='{}'",
                        path.display(),
                        known.display()
                    ),
                ));
                key_file = Some(path);
            }
            _ => {}
        }
        // Later keys win: collapse duplicates of GIT_CONFIG_COUNT.
        let mut seen = std::collections::HashMap::new();
        for (k, v) in &env {
            seen.insert(k.clone(), v.clone());
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            env: seen.into_iter().collect(),
            key_file,
        })
    }

    async fn run(&self, args: &[&str]) -> Result<String> {
        self.run_env(args, &[]).await
    }

    async fn run_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Result<String> {
        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("-C").arg(&self.dir).args(args);
        cmd.envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cmd.envs(extra.iter().copied());
        cmd.stdin(std::process::Stdio::null());
        cmd.kill_on_drop(true);
        let out = tokio::time::timeout(COMMAND_TIMEOUT, cmd.output())
            .await
            .map_err(|_| anyhow!("git {} timed out", args.first().unwrap_or(&"")))?
            .with_context(|| format!("run git {}", args.first().unwrap_or(&"")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!(
                "git {} failed: {}",
                args.first().unwrap_or(&""),
                redact(err.trim())
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Strip credentials a URL in git's message might carry (`https://user:tok@`).
fn redact(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    for word in msg.split_inclusive(char::is_whitespace) {
        match (word.find("://"), word.find('@')) {
            (Some(s), Some(a)) if a > s => {
                out.push_str(&word[..s + 3]);
                out.push_str("***");
                out.push_str(&word[a..]);
            }
            _ => out.push_str(word),
        }
    }
    out
}

fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .with_context(|| format!("write {}", path.display()))?;
    f.write_all(text.as_bytes())?;
    if !text.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    Ok(())
}

/// After a publish: sync in the background when configured to.
pub fn after_publish(storage: Storage, auth_secret: String, reason: SyncReason) {
    tokio::spawn(async move {
        let Ok(c) = load_config(&storage).await else {
            return;
        };
        if !c.enabled || !c.push_on_publish {
            return;
        }
        if let Err(e) = sync(&storage, &auth_secret, &reason).await {
            tracing::warn!("git sync after publish failed: {e:#}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_validation() {
        let mut c = GitConfig {
            enabled: true,
            remote: "/srv/git/plant.git".into(),
            ..GitConfig::default()
        };
        assert!(c.validate().is_ok());
        c.remote = "--upload-pack=touch /tmp/x".into();
        assert!(c.validate().is_err());
        c.remote = "ext::sh -c id".into();
        assert!(c.validate().is_err());
        c.remote = "https://git.example/plant.git".into();
        c.path = "../escape.yaml".into();
        assert!(c.validate().is_err());
        c.path = ".git/config".into();
        assert!(c.validate().is_err());
        c.path = "sites/plant.yaml".into();
        assert!(c.validate().is_ok());
        c.branch = "-x".into();
        assert!(c.validate().is_err());
        c.branch = "a..b".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn redacts_credentials_in_urls() {
        assert_eq!(
            redact("fatal: unable to access 'https://bob:secret@git.example/x.git/': 403"),
            "fatal: unable to access 'https://***@git.example/x.git/': 403"
        );
        assert_eq!(redact("no url here"), "no url here");
    }
}
