//! Optional git sync of the project (docs/version-history.md).
//!
//! On publish (or on demand) the server exports the project as
//! `voltrus-project/1` YAML, with each screen's published rows, and commits
//! and pushes it to a configured remote (GitHub, Gitea, a bare repository in
//! the data folder). The export is the **whole project**: devices with their
//! addresses, tags, flows and settings, not only screens; the remote host
//! gets all of it. It shells out to the `git` command line: the single binary
//! carries no git library, and a site without git still gets the export
//! written to the working folder (reported as not pushed).
//!
//! The working copy is a bare repository: the commit is built from a blob
//! and an index file (`hash-object`, `update-index`, `write-tree`,
//! `commit-tree`), never checked out, so nothing in the remote repository (a
//! symlink named like the project file, a symlinked folder) can make the
//! server read or write outside it. A symlink on the project file's path in
//! the remote tree is refused.
//!
//! Credentials (an HTTPS token or an SSH deploy key) are stored encrypted at
//! rest like the copilot's API key, never put on a command line or in the
//! remote URL (one with credentials in it is refused), and handed to git
//! through its environment. Token auth needs `https://`. A local remote and
//! the working copy must lie in the data folder. Pull stays manual: a file
//! from git is imported through the ordinary project import plan.

use std::path::{Component, Path, PathBuf};
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
/// Deploy key files are named `<this><random>` in the data folder.
const KEY_PREFIX: &str = ".design-git-key-";

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
    /// path of a bare repository in the data folder. No credentials in it.
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
    /// Working copy (a bare repository), relative to the data folder;
    /// empty: `design-git`.
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

/// What kind of place a remote is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Remote {
    Https,
    Http,
    Ssh,
    Git,
    Local(PathBuf),
}

impl Remote {
    /// `GIT_ALLOW_PROTOCOL` for it: only its own transport.
    fn protocol(&self) -> &'static str {
        match self {
            Self::Https => "https",
            Self::Http => "http",
            Self::Ssh => "ssh",
            Self::Git => "git",
            Self::Local(_) => "file",
        }
    }
}

/// Whether `s` reads as a Windows drive path (`C:\x`, `C:/x`).
fn drive_path(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// Classify a remote, refusing credentials in it.
fn classify(remote: &str) -> Result<Remote> {
    let lower = remote.to_ascii_lowercase();
    let authority =
        |rest: &str| -> String { rest.split(['/', '?', '#']).next().unwrap_or("").to_string() };
    for (scheme, kind) in [
        ("https://", Remote::Https),
        ("http://", Remote::Http),
        ("ssh://", Remote::Ssh),
        ("git://", Remote::Git),
    ] {
        if lower.starts_with(scheme) {
            let auth = authority(&remote[scheme.len()..]);
            if let Some((user, _)) = auth.rsplit_once('@')
                && (kind != Remote::Ssh || user.contains(':'))
            {
                bail!(
                    "put no credentials in the remote URL (they would be stored and shown in plain text): use auth token or ssh_key"
                );
            }
            return Ok(kind);
        }
    }
    if lower.starts_with("file://") {
        return Ok(Remote::Local(PathBuf::from(&remote["file://".len()..])));
    }
    if remote.contains("://") {
        bail!(
            "remote '{}' uses a transport that is not allowed (https, http, ssh, git, a local path)",
            redact(remote)
        );
    }
    // scp-like `user@host:path` / `host:path` (a colon before any slash).
    let scp = !remote.starts_with('/')
        && !remote.starts_with('.')
        && !drive_path(remote)
        && remote
            .find(':')
            .is_some_and(|c| remote.find('/').is_none_or(|s| c < s));
    if scp {
        return Ok(Remote::Ssh);
    }
    Ok(Remote::Local(PathBuf::from(remote)))
}

/// `path` is relative, inside, with no `..`.
fn plain_relative(path: &str) -> bool {
    let p = Path::new(path);
    !path.trim().is_empty()
        && !p.is_absolute()
        && !drive_path(path)
        && p.components().all(|c| matches!(c, Component::Normal(_)))
}

impl GitConfig {
    /// Refuse values git would read as options, credentials in the remote,
    /// a token over plain HTTP, and paths that escape the repository or the
    /// data folder.
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
        if self.remote.chars().any(char::is_control) {
            bail!("remote must not contain control characters");
        }
        if self.branch.trim().is_empty()
            || self
                .branch
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
            || self.branch.contains("..")
            || self.branch.contains("@{")
            || self.branch.ends_with('/')
            || self.branch.ends_with(".lock")
        {
            bail!("branch '{}' is not a valid branch name", self.branch);
        }
        if !plain_relative(&self.path)
            || self.path.starts_with(".git")
            || self.path.contains("/.git")
        {
            bail!("path must be a relative file path inside the repository");
        }
        if !self.work_dir.trim().is_empty() && !plain_relative(&self.work_dir) {
            bail!("work_dir must be a relative folder inside the data folder");
        }
        if !self.remote.trim().is_empty() {
            let kind = classify(&self.remote)?;
            match (self.auth, &kind) {
                (GitAuth::Token, Remote::Http) => bail!(
                    "token auth over http:// would send the token in clear text: use an https:// remote"
                ),
                (GitAuth::Token, k) if *k != Remote::Https => {
                    bail!("token auth needs an https:// remote")
                }
                (GitAuth::SshKey, k) if *k != Remote::Ssh => {
                    bail!("ssh_key auth needs an ssh remote (ssh://… or user@host:path)")
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// [`Self::validate`], and a local remote must be an existing folder in
    /// `data_dir`.
    pub fn validate_in(&self, data_dir: &Path) -> Result<()> {
        self.validate()?;
        if let Ok(Remote::Local(p)) = classify(&self.remote)
            && !self.remote.trim().is_empty()
        {
            let base = std::fs::canonicalize(data_dir)
                .with_context(|| format!("data folder {}", data_dir.display()))?;
            let target = std::fs::canonicalize(&p).map_err(|_| {
                anyhow!(
                    "the local remote {} does not exist (it must be a repository in the data folder {})",
                    p.display(),
                    base.display()
                )
            })?;
            if !target.starts_with(&base) {
                bail!(
                    "a local remote must be inside the data folder {} (not {})",
                    base.display(),
                    target.display()
                );
            }
        }
        Ok(())
    }

    /// The configuration as shown and audited (credentials redacted, in case
    /// an older setting carries some).
    pub fn redacted(&self) -> Self {
        Self {
            remote: redact(&self.remote),
            ..self.clone()
        }
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
    /// The branch's commit after the sync, `None` without git.
    pub commit: Option<String>,
    /// A new commit was made (false: the project had not changed).
    pub committed: bool,
    pub pushed: bool,
    /// The file in the repository (or, without git, where it was written).
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
    c.validate_in(&data_dir(storage))?;
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

/// The data folder: the database's folder (for an in-memory database, a
/// folder of this process in the temp folder).
pub fn data_dir(storage: &Storage) -> PathBuf {
    let db = Path::new(storage.db_path());
    match db.parent() {
        Some(dir) if storage.db_path() != ":memory:" => {
            if dir.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                dir.to_path_buf()
            }
        }
        _ => std::env::temp_dir().join(format!("voltrus-data-{}", std::process::id())),
    }
}

fn work_dir(data: &Path, c: &GitConfig) -> PathBuf {
    if c.work_dir.trim().is_empty() {
        data.join("design-git")
    } else {
        data.join(&c.work_dir)
    }
}

/// Delete deploy key files a crashed sync left in the data folder.
pub fn cleanup_stale_keys(data: &Path) {
    let Ok(entries) = std::fs::read_dir(data) else {
        return;
    };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().starts_with(KEY_PREFIX) {
            let _ = std::fs::remove_file(e.path());
        }
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
            json!({"ok": false, "at_ms": crate::timeutil::now_millis(), "error": redact(&format!("{e:#}")), "reason": reason.message})
        }
    };
    let _ = storage
        .upsert_setting(STATUS_KEY, &status.to_string())
        .await;
    result
}

/// Create `dir` (and its parents below `base`), refusing a symlink on the way.
fn create_inside(base: &Path, dir: &Path) -> Result<()> {
    let rel = dir.strip_prefix(base).unwrap_or(dir);
    let mut at = base.to_path_buf();
    std::fs::create_dir_all(&at).with_context(|| format!("create {}", at.display()))?;
    for part in rel.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => {
                bail!("{} is a symbolic link: refused", at.display())
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => bail!("{} is not a folder", at.display()),
            Err(_) => {
                std::fs::create_dir(&at).with_context(|| format!("create {}", at.display()))?
            }
        }
    }
    Ok(())
}

async fn sync_locked(
    storage: &Storage,
    auth_secret: &str,
    reason: &SyncReason,
) -> Result<SyncOutcome> {
    let c = load_config(storage).await?;
    let data = data_dir(storage);
    std::fs::create_dir_all(&data).with_context(|| format!("create {}", data.display()))?;
    c.validate_in(&data)?;
    if !c.enabled {
        bail!("git sync is not enabled");
    }
    cleanup_stale_keys(&data);
    let doc = crate::project_yaml::export::export_project_yaml_with(storage, true).await?;
    let yaml = crate::project_yaml::to_yaml(&doc);
    let dir = work_dir(&data, &c);
    create_inside(&data, &dir)?;

    if !git_available().await {
        // The export only, into the working folder (never through a link).
        let file = dir.join(&c.path);
        if let Some(parent) = file.parent() {
            create_inside(&data, parent)?;
        }
        if std::fs::symlink_metadata(&file).is_ok_and(|m| !m.is_file()) {
            bail!("{} is not a plain file: refused", file.display());
        }
        std::fs::write(&file, &yaml)?;
        return Ok(SyncOutcome {
            commit: None,
            committed: false,
            pushed: false,
            file: file.display().to_string(),
            message:
                "git is not installed on this server: the project was exported to the folder only"
                    .into(),
        });
    }

    let remote = classify(&c.remote)?;
    let secret = match c.auth {
        GitAuth::None => None,
        _ => Some(load_secret(storage, auth_secret).await?.ok_or_else(|| {
            anyhow!(
                "git sync uses {:?} auth but no credential is stored",
                c.auth
            )
        })?),
    };
    // The working copy is bare (an older non-bare one is used through its
    // `.git`): nothing is ever checked out.
    let git_dir = if dir.join(".git").is_dir() {
        dir.join(".git")
    } else {
        dir.clone()
    };
    let git = Git::new(&git_dir, &data, &c, &remote, secret.as_deref())?;
    if !git_dir.join("HEAD").exists() {
        git.init().await?;
    }
    let has_origin = git.run(&["remote", "get-url", "origin"]).await.is_ok();
    if has_origin {
        git.run(&["remote", "set-url", "origin", &c.remote]).await?;
    } else {
        git.run(&["remote", "add", "origin", &c.remote]).await?;
    }
    // Build on the remote branch when it exists: the remote is the truth.
    let remote_ref = format!("refs/remotes/origin/{}", c.branch);
    let fetched = git
        .run(&[
            "fetch",
            "-q",
            "--no-tags",
            "origin",
            &format!("+refs/heads/{}:{remote_ref}", c.branch),
        ])
        .await
        .is_ok();
    let parent = if fetched {
        Some(
            git.run(&["rev-parse", "--verify", &format!("{remote_ref}^{{commit}}")])
                .await?
                .trim()
                .to_string(),
        )
    } else {
        None
    };

    let mut same = false;
    if let Some(parent) = &parent {
        // Every component of the path must be a folder, the last a file.
        let parts: Vec<&str> = c.path.split('/').collect();
        for i in 0..parts.len() {
            let prefix = parts[..=i].join("/");
            let line = git.run(&["ls-tree", parent, "--", &prefix]).await?;
            let Some(entry) = line.lines().find(|l| l.ends_with(&format!("\t{prefix}"))) else {
                break;
            };
            let mode = entry.split_whitespace().next().unwrap_or("");
            let last = i + 1 == parts.len();
            match mode {
                "120000" => bail!(
                    "the remote repository has a symbolic link at '{prefix}': refused (make it a plain file or folder)"
                ),
                "040000" if !last => {}
                "100644" | "100755" if last => {
                    let old = git
                        .run(&["cat-file", "blob", &format!("{parent}:{}", c.path)])
                        .await?;
                    same = without_timestamp(&old) == without_timestamp(&yaml);
                }
                _ => bail!(
                    "'{prefix}' in the remote repository is not a {} (mode {mode})",
                    if last { "plain file" } else { "folder" }
                ),
            }
        }
    }

    let mut committed = false;
    let mut commit = parent.clone();
    if !same {
        let index = git_dir.join("voltrus-index");
        let _ = std::fs::remove_file(&index);
        let index_text = index.display().to_string();
        let ienv = [("GIT_INDEX_FILE", index_text.as_str())];
        match &parent {
            Some(p) => git.run_env(&["read-tree", p], &ienv, None).await?,
            None => git.run_env(&["read-tree", "--empty"], &ienv, None).await?,
        };
        let blob = git
            .run_env(
                &["hash-object", "-w", "--stdin"],
                &[],
                Some(yaml.as_bytes()),
            )
            .await?;
        git.run_env(
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{},{}", blob.trim(), c.path),
            ],
            &ienv,
            None,
        )
        .await?;
        let tree = git.run_env(&["write-tree"], &ienv, None).await?;
        let _ = std::fs::remove_file(&index);
        let tree = tree.trim().to_string();
        let parent_tree = match &parent {
            Some(p) => Some(
                git.run(&["rev-parse", &format!("{p}^{{tree}}")])
                    .await?
                    .trim()
                    .to_string(),
            ),
            None => None,
        };
        if parent_tree.as_deref() != Some(tree.as_str()) {
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
            let mut args = vec!["commit-tree", tree.as_str()];
            if let Some(p) = &parent {
                args.push("-p");
                args.push(p);
            }
            args.push("-m");
            args.push(&reason.message);
            let sha = git
                .run_env(
                    &args,
                    &[
                        ("GIT_AUTHOR_NAME", reason.author.as_str()),
                        ("GIT_AUTHOR_EMAIL", email.as_str()),
                        ("GIT_COMMITTER_NAME", "Voltrus SCADA"),
                        ("GIT_COMMITTER_EMAIL", email.as_str()),
                    ],
                    None,
                )
                .await?;
            commit = Some(sha.trim().to_string());
            committed = true;
        }
    }
    let mut pushed = false;
    if let Some(sha) = &commit {
        git.run(&["update-ref", &format!("refs/heads/{}", c.branch), sha])
            .await?;
        if committed || !fetched {
            git.run(&[
                "push",
                "-q",
                "origin",
                &format!("{sha}:refs/heads/{}", c.branch),
            ])
            .await
            .context("git push")?;
            pushed = true;
        }
    }
    Ok(SyncOutcome {
        commit,
        committed,
        pushed,
        file: c.path.clone(),
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

/// A path as one POSIX shell word (git runs `GIT_SSH_COMMAND` through `sh`,
/// on Windows too).
fn sh_quote(path: &Path) -> Result<String> {
    let s = path
        .to_str()
        .context("the data folder's path is not UTF-8")?;
    if s.chars().any(char::is_control) {
        bail!("the data folder's path holds control characters");
    }
    Ok(format!("'{}'", s.replace('\'', "'\\''")))
}

/// One bare repository and the environment its commands run with.
struct Git {
    git_dir: PathBuf,
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
    fn new(
        git_dir: &Path,
        data: &Path,
        c: &GitConfig,
        remote: &Remote,
        secret: Option<&str>,
    ) -> Result<Self> {
        let mut config: Vec<(&str, String)> = vec![
            ("credential.helper", String::new()),
            ("core.hooksPath", "/dev/null".into()),
            ("http.followRedirects", "false".into()),
            ("core.symlinks", "false".into()),
        ];
        let mut env: Vec<(String, String)> = vec![
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            // Only the remote's own transport: no `ext::`, no other helpers.
            ("GIT_ALLOW_PROTOCOL".into(), remote.protocol().into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            // No user config (url.insteadOf and the like).
            ("GIT_CONFIG_GLOBAL".into(), null_device().into()),
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
                config.push(("http.extraHeader", format!("Authorization: Basic {basic}")));
            }
            (GitAuth::SshKey, Some(key)) => {
                let path = data.join(format!("{KEY_PREFIX}{}", random_hex()));
                write_private(&path, key)?;
                key_file = Some(path.clone());
                let known = data.join(".design-git-known_hosts");
                env.push((
                    "GIT_SSH_COMMAND".into(),
                    format!(
                        "ssh -i {} -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile={}",
                        sh_quote(&path)?,
                        sh_quote(&known)?
                    ),
                ));
            }
            _ => {}
        }
        env.push(("GIT_CONFIG_COUNT".into(), config.len().to_string()));
        for (i, (k, v)) in config.into_iter().enumerate() {
            env.push((format!("GIT_CONFIG_KEY_{i}"), k.to_string()));
            env.push((format!("GIT_CONFIG_VALUE_{i}"), v));
        }
        Ok(Self {
            git_dir: git_dir.to_path_buf(),
            env,
            key_file,
        })
    }

    async fn init(&self) -> Result<()> {
        let mut cmd = tokio::process::Command::new("git");
        cmd.args(["init", "-q", "--bare"]).arg(&self.git_dir);
        cmd.envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let out = cmd.output().await.context("run git init")?;
        if !out.status.success() {
            bail!(
                "git init failed: {}",
                redact(String::from_utf8_lossy(&out.stderr).trim())
            );
        }
        Ok(())
    }

    async fn run(&self, args: &[&str]) -> Result<String> {
        self.run_env(args, &[], None).await
    }

    async fn run_env(
        &self,
        args: &[&str],
        extra: &[(&str, &str)],
        stdin: Option<&[u8]>,
    ) -> Result<String> {
        use tokio::io::AsyncWriteExt;
        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("--git-dir").arg(&self.git_dir).args(args);
        cmd.env_remove("GIT_WORK_TREE");
        cmd.envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cmd.envs(extra.iter().copied());
        cmd.stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);
        let name = args.first().copied().unwrap_or("");
        let run = async {
            let mut child = cmd.spawn().with_context(|| format!("run git {name}"))?;
            if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
                pipe.write_all(input).await?;
                drop(pipe);
            }
            child
                .wait_with_output()
                .await
                .with_context(|| format!("run git {name}"))
        };
        let out = tokio::time::timeout(COMMAND_TIMEOUT, run)
            .await
            .map_err(|_| anyhow!("git {name} timed out"))??;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("git {name} failed: {}", redact(err.trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

fn random_hex() -> String {
    use ring::rand::SecureRandom;
    let mut r = [0u8; 8];
    let _ = ring::rand::SystemRandom::new().fill(&mut r);
    r.iter().map(|b| format!("{b:02x}")).collect()
}

/// Strip credentials a URL might carry (`https://user:tok@` → `https://***@`).
pub fn redact(msg: &str) -> String {
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

/// Write a secret file only its owner may read (a fresh name: never an
/// existing file or link).
fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
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
        c.branch = "main".into();
        // The working copy stays in the data folder.
        for bad in ["/etc", "../up", "a/../../b", "C:\\x"] {
            c.work_dir = bad.into();
            assert!(c.validate().is_err(), "{bad}");
        }
        c.work_dir = "sync/plant".into();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn remotes_carry_no_credentials_and_tokens_need_https() {
        let mut c = GitConfig {
            enabled: true,
            remote: "https://bob:secret@git.example/x.git".into(),
            ..GitConfig::default()
        };
        assert!(c.validate().is_err(), "userinfo in https");
        c.remote = "https://bob@git.example/x.git".into();
        assert!(c.validate().is_err(), "user in https");
        c.remote = "ssh://git:pw@git.example/x.git".into();
        assert!(c.validate().is_err(), "password in ssh");
        c.remote = "ssh://git@git.example/x.git".into();
        assert!(c.validate().is_ok(), "ssh user is fine");
        c.remote = "http://git.example/x.git".into();
        c.auth = GitAuth::Token;
        let e = c.validate().unwrap_err().to_string();
        assert!(e.contains("clear text"), "{e}");
        c.remote = "git@git.example:org/x.git".into();
        assert!(c.validate().is_err(), "token over ssh");
        c.auth = GitAuth::SshKey;
        assert!(c.validate().is_ok());
        c.remote = "https://git.example/x.git".into();
        assert!(c.validate().is_err(), "ssh key over https");
        c.remote = "gopher://x".into();
        c.auth = GitAuth::None;
        assert!(c.validate().is_err());
    }

    #[test]
    fn local_remotes_stay_in_the_data_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(data.join("plant.git")).unwrap();
        std::fs::create_dir_all(tmp.path().join("outside.git")).unwrap();
        let mut c = GitConfig {
            enabled: true,
            remote: data.join("plant.git").display().to_string(),
            ..GitConfig::default()
        };
        assert!(c.validate_in(&data).is_ok());
        c.remote = format!("file://{}", data.join("plant.git").display());
        assert!(c.validate_in(&data).is_ok());
        c.remote = tmp.path().join("outside.git").display().to_string();
        assert!(c.validate_in(&data).is_err());
        c.remote = format!("{}/../outside.git", data.display());
        assert!(c.validate_in(&data).is_err());
        c.remote = format!("file://{}", tmp.path().join("outside.git").display());
        assert!(c.validate_in(&data).is_err());
    }

    #[test]
    fn shell_quoting_keeps_a_path_one_word() {
        assert_eq!(
            sh_quote(Path::new("/srv/a b/it's")).unwrap(),
            "'/srv/a b/it'\\''s'"
        );
        assert!(sh_quote(Path::new("/srv/a\nb")).is_err());
    }

    #[test]
    fn redacts_credentials_in_urls() {
        assert_eq!(
            redact("fatal: unable to access 'https://bob:secret@git.example/x.git/': 403"),
            "fatal: unable to access 'https://***@git.example/x.git/': 403"
        );
        assert_eq!(redact("no url here"), "no url here");
    }

    #[test]
    fn private_files_are_owner_only_and_never_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(format!("{KEY_PREFIX}x"));
        write_private(&p, "key").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(write_private(&p, "again").is_err(), "create_new");
        cleanup_stale_keys(tmp.path());
        assert!(!p.exists());
    }
}
