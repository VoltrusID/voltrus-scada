//! Designer comments (docs/collaboration.md#comments): threads pinned to a
//! widget, a connection or a point of a screen. A thread is a root comment and
//! its replies; resolving acts on the root. `@name` mentions a user, who gets
//! a `mention` notification.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::Storage;

/// The longest comment body, characters.
pub const MAX_BODY: usize = 4000;
/// Most comments on one screen.
pub const MAX_PER_SCREEN: i64 = 5000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Comment {
    pub id: i64,
    pub screen_id: String,
    /// The thread's root comment; `None` for a root.
    pub parent_id: Option<i64>,
    /// The widget or connection the pin is on, if any.
    pub object_id: Option<String>,
    /// `widget` or `connection`.
    pub object_type: Option<String>,
    /// Stage coordinates of the pin (relative to the object's top left when
    /// it is on one).
    pub x: f64,
    pub y: f64,
    pub body: String,
    pub author: String,
    /// Users named with `@name` that exist.
    pub mentions: Vec<String>,
    pub resolved: bool,
    pub resolved_by: Option<String>,
    pub resolved_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// A new comment.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewComment {
    pub body: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub object_id: Option<String>,
    #[serde(default)]
    pub object_type: Option<String>,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

/// A change to a comment: its text (author only) or the thread's state.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentPatch {
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub resolved: Option<bool>,
}

/// Why a comment write was refused (answered 400, 403 or 404).
#[derive(Debug)]
pub enum CommentRefusal {
    Invalid(String),
    Forbidden(String),
    NotFound,
}

impl std::fmt::Display for CommentRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommentRefusal::Invalid(m) | CommentRefusal::Forbidden(m) => f.write_str(m),
            CommentRefusal::NotFound => f.write_str("comment not found"),
        }
    }
}

impl std::error::Error for CommentRefusal {}

const COLUMNS: &str = "id, screen_id, parent_id, object_id, object_type, x, y, body, author, mentions, resolved, resolved_by, resolved_at_ms, created_at_ms, updated_at_ms";

fn map(r: &rusqlite::Row<'_>) -> rusqlite::Result<Comment> {
    let mentions: String = r.get(9)?;
    Ok(Comment {
        id: r.get(0)?,
        screen_id: r.get(1)?,
        parent_id: r.get(2)?,
        object_id: r.get(3)?,
        object_type: r.get(4)?,
        x: r.get(5)?,
        y: r.get(6)?,
        body: r.get(7)?,
        author: r.get(8)?,
        mentions: serde_json::from_str(&mentions).unwrap_or_default(),
        resolved: r.get::<_, i64>(10)? != 0,
        resolved_by: r.get(11)?,
        resolved_at_ms: r.get(12)?,
        created_at_ms: r.get(13)?,
        updated_at_ms: r.get(14)?,
    })
}

pub fn get(conn: &Connection, screen_id: &str, id: i64) -> Result<Option<Comment>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM design_comments WHERE id = ?1 AND screen_id = ?2"),
            params![id, screen_id],
            map,
        )
        .optional()?)
}

/// The comments of a screen, oldest first; without `resolved`, only the
/// threads that are open.
pub fn list(conn: &Connection, screen_id: &str, resolved: bool) -> Result<Vec<Comment>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM design_comments c WHERE screen_id = ?1
           AND (?2 OR NOT EXISTS (SELECT 1 FROM design_comments r
                 WHERE r.id = COALESCE(c.parent_id, c.id) AND r.resolved = 1))
         ORDER BY created_at_ms, id"
    ))?;
    let rows = stmt.query_map(params![screen_id, resolved], map)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// `@name` tokens of `body` that name an existing user.
pub fn mentions(conn: &Connection, body: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for token in body.split(|c: char| c.is_whitespace() || ",;:()[]!?\"'".contains(c)) {
        let Some(name) = token.strip_prefix('@') else {
            continue;
        };
        let name = name.trim_end_matches('.');
        if name.is_empty() || out.iter().any(|n| n == name) {
            continue;
        }
        let exists: Option<i64> = conn
            .query_row("SELECT id FROM users WHERE username = ?1", [name], |r| {
                r.get(0)
            })
            .optional()?;
        if exists.is_some() {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

fn check_body(body: &str) -> std::result::Result<String, CommentRefusal> {
    let body = body.trim();
    if body.is_empty() {
        return Err(CommentRefusal::Invalid("a comment needs text".into()));
    }
    if body.chars().count() > MAX_BODY {
        return Err(CommentRefusal::Invalid(format!(
            "a comment is at most {MAX_BODY} characters"
        )));
    }
    Ok(body.to_string())
}

fn notify(conn: &Connection, author: &str, users: &[String], c: &Comment) -> Result<()> {
    let from: Option<i64> = conn
        .query_row("SELECT id FROM users WHERE username = ?1", [author], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(from) = from else {
        return Ok(());
    };
    let mut snippet: String = c.body.chars().take(140).collect();
    if c.body.chars().count() > 140 {
        snippet.push('…');
    }
    for name in users {
        if name == author {
            continue;
        }
        let to: Option<i64> = conn
            .query_row("SELECT id FROM users WHERE username = ?1", [name], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(to) = to {
            conn.execute(
                "INSERT INTO user_notifications (user_id, from_user_id, notification_type, message, reference_id, read, created_at)
                 VALUES (?1, ?2, 'mention', ?3, ?4, 0, ?5)",
                params![
                    to,
                    from,
                    format!("{author} mentioned you on screen {}: {snippet}", c.screen_id),
                    c.id,
                    crate::timeutil::now_secs()
                ],
            )?;
        }
    }
    Ok(())
}

/// Add a comment by `author`. A reply joins its parent's thread (a reply to
/// a reply joins the root) and takes its pin.
pub fn create(conn: &Connection, screen_id: &str, author: &str, n: &NewComment) -> Result<Comment> {
    let body = check_body(&n.body)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM design_comments WHERE screen_id = ?1",
        [screen_id],
        |r| r.get(0),
    )?;
    if count >= MAX_PER_SCREEN {
        bail!(CommentRefusal::Invalid(format!(
            "a screen has at most {MAX_PER_SCREEN} comments; delete resolved ones first"
        )));
    }
    let (parent, object_id, object_type, x, y) = match n.parent_id {
        Some(pid) => {
            let Some(p) = get(conn, screen_id, pid)? else {
                bail!(CommentRefusal::Invalid(format!(
                    "no comment {pid} on this screen"
                )));
            };
            (
                Some(p.parent_id.unwrap_or(p.id)),
                p.object_id,
                p.object_type,
                p.x,
                p.y,
            )
        }
        None => {
            if !n.x.is_finite()
                || !n.y.is_finite()
                || n.x.abs() > 100_000.0
                || n.y.abs() > 100_000.0
            {
                bail!(CommentRefusal::Invalid("the pin is off the stage".into()));
            }
            let object_type = match (&n.object_id, n.object_type.as_deref()) {
                (None, _) => None,
                (Some(_), None | Some("widget")) => Some("widget".to_string()),
                (Some(_), Some("connection")) => Some("connection".to_string()),
                (Some(_), Some(other)) => {
                    bail!(CommentRefusal::Invalid(format!(
                        "unknown object type '{other}'"
                    )))
                }
            };
            if n.object_id
                .as_ref()
                .is_some_and(|o| o.is_empty() || o.len() > 128)
            {
                bail!(CommentRefusal::Invalid("bad object id".into()));
            }
            (None, n.object_id.clone(), object_type, n.x, n.y)
        }
    };
    let mentioned = mentions(conn, &body)?;
    let now = crate::timeutil::now_millis();
    conn.execute(
        "INSERT INTO design_comments (screen_id, parent_id, object_id, object_type, x, y, body, author, mentions, resolved, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?10)",
        params![
            screen_id,
            parent,
            object_id,
            object_type,
            x,
            y,
            body,
            author,
            serde_json::to_string(&mentioned)?,
            now
        ],
    )
    .context("insert design comment")?;
    let c = get(conn, screen_id, conn.last_insert_rowid())?.context("comment just written")?;
    notify(conn, author, &mentioned, &c)?;
    Ok(c)
}

/// Change a comment: its text (only its author), or resolve/reopen its
/// thread (anyone who may comment).
pub fn update(
    conn: &Connection,
    screen_id: &str,
    id: i64,
    actor: &str,
    p: &CommentPatch,
) -> Result<Comment> {
    let Some(c) = get(conn, screen_id, id)? else {
        bail!(CommentRefusal::NotFound);
    };
    let now = crate::timeutil::now_millis();
    if let Some(body) = &p.body {
        if c.author != actor {
            bail!(CommentRefusal::Forbidden(
                "only its author edits a comment".into()
            ));
        }
        let body = check_body(body)?;
        let mentioned = mentions(conn, &body)?;
        let new: Vec<String> = mentioned
            .iter()
            .filter(|m| !c.mentions.contains(m))
            .cloned()
            .collect();
        conn.execute(
            "UPDATE design_comments SET body = ?1, mentions = ?2, updated_at_ms = ?3 WHERE id = ?4",
            params![body, serde_json::to_string(&mentioned)?, now, id],
        )?;
        let c = get(conn, screen_id, id)?.context("comment just written")?;
        notify(conn, actor, &new, &c)?;
    }
    if let Some(resolved) = p.resolved {
        let root = c.parent_id.unwrap_or(c.id);
        conn.execute(
            "UPDATE design_comments SET resolved = ?1, resolved_by = ?2, resolved_at_ms = ?3, updated_at_ms = ?4 WHERE id = ?5",
            params![
                resolved,
                resolved.then_some(actor),
                resolved.then_some(now),
                now,
                root
            ],
        )?;
    }
    get(conn, screen_id, id)?.context("comment just written")
}

/// Delete a comment (its author or an administrator); a root takes its
/// replies with it.
pub fn delete(
    conn: &Connection,
    screen_id: &str,
    id: i64,
    actor: &str,
    admin: bool,
) -> Result<Comment> {
    let Some(c) = get(conn, screen_id, id)? else {
        bail!(CommentRefusal::NotFound);
    };
    if c.author != actor && !admin {
        bail!(CommentRefusal::Forbidden(
            "only its author or an administrator deletes a comment".into()
        ));
    }
    conn.execute(
        "DELETE FROM design_comments WHERE id = ?1 OR parent_id = ?1",
        [id],
    )?;
    Ok(c)
}

impl Storage {
    pub async fn design_comments(&self, screen_id: &str, resolved: bool) -> Result<Vec<Comment>> {
        let screen = screen_id.to_string();
        self.with_conn(move |c| list(c, &screen, resolved)).await
    }
}
