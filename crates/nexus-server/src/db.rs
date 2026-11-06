//! SQLite store. The database file holds session hashes and in-progress device codes.

use std::path::Path;
use std::sync::Mutex;

use nexus_protocol::{AdminApp, Grant, Visibility};
use rusqlite::{params, Connection};

use crate::error::ApiFailure;

pub struct Database {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub github_id: u64,
    pub login: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct FlowRow {
    pub device_code: String,
    pub interval_seconds: i64,
    pub expires_at: String,
    pub next_poll_at: String,
}

#[derive(Debug, Clone)]
pub struct AppRow {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub category: Option<String>,
    pub homepage: Option<String>,
    pub repo: Option<String>,
    pub visibility: Visibility,
    pub published: bool,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
        }
        let conn = Connection::open(path).map_err(|err| err.to_string())?;
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS users (
                github_id INTEGER PRIMARY KEY,
                login TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                token_hash TEXT PRIMARY KEY,
                github_id INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS device_flows (
                poll_hash TEXT PRIMARY KEY,
                device_code TEXT NOT NULL,
                interval_seconds INTEGER NOT NULL,
                expires_at TEXT NOT NULL,
                next_poll_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS apps (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                category TEXT,
                homepage TEXT,
                repo TEXT,
                visibility TEXT NOT NULL,
                published INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS grants (
                github_id INTEGER NOT NULL,
                app_id TEXT NOT NULL,
                login TEXT NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (github_id, app_id)
            );
            ",
        )
        .map_err(|err| err.to_string())?;
        #[cfg(unix)]
        tighten_db_file(path);
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, ApiFailure> {
        self.conn.lock().map_err(|_| ApiFailure::internal())
    }

    pub fn upsert_user(&self, github_id: u64, login: &str, now: &str) -> Result<(), ApiFailure> {
        self.lock()?.execute(
            "INSERT INTO users (github_id, login, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(github_id) DO UPDATE SET login = excluded.login, updated_at = excluded.updated_at",
            params![github_id as i64, login, now],
        )?;
        Ok(())
    }

    pub fn insert_session(
        &self,
        token_hash: &str,
        github_id: u64,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), ApiFailure> {
        self.lock()?.execute(
            "INSERT INTO sessions (token_hash, github_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, github_id as i64, created_at, expires_at],
        )?;
        Ok(())
    }

    pub fn session(&self, token_hash: &str) -> Result<Option<SessionRow>, ApiFailure> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT s.github_id, u.login, s.expires_at
             FROM sessions s JOIN users u ON u.github_id = s.github_id
             WHERE s.token_hash = ?1",
        )?;
        let mut rows = stmt.query(params![token_hash])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(SessionRow {
                github_id: row.get::<_, i64>(0)? as u64,
                login: row.get(1)?,
                expires_at: row.get(2)?,
            }));
        }
        Ok(None)
    }

    pub fn delete_session(&self, token_hash: &str) -> Result<(), ApiFailure> {
        self.lock()?.execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            params![token_hash],
        )?;
        Ok(())
    }

    pub fn insert_flow(&self, flow: &NewFlow) -> Result<(), ApiFailure> {
        self.lock()?.execute(
            "INSERT INTO device_flows (poll_hash, device_code, interval_seconds, expires_at, next_poll_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                flow.poll_hash,
                flow.device_code,
                flow.interval_seconds,
                flow.expires_at,
                flow.next_poll_at
            ],
        )?;
        Ok(())
    }

    pub fn flow(&self, poll_hash: &str) -> Result<Option<FlowRow>, ApiFailure> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT device_code, interval_seconds, expires_at, next_poll_at FROM device_flows WHERE poll_hash = ?1",
        )?;
        let mut rows = stmt.query(params![poll_hash])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(FlowRow {
                device_code: row.get(0)?,
                interval_seconds: row.get(1)?,
                expires_at: row.get(2)?,
                next_poll_at: row.get(3)?,
            }));
        }
        Ok(None)
    }

    pub fn touch_flow(
        &self,
        poll_hash: &str,
        interval_seconds: i64,
        next_poll_at: &str,
    ) -> Result<(), ApiFailure> {
        self.lock()?.execute(
            "UPDATE device_flows SET interval_seconds = ?2, next_poll_at = ?3 WHERE poll_hash = ?1",
            params![poll_hash, interval_seconds, next_poll_at],
        )?;
        Ok(())
    }

    pub fn delete_flow(&self, poll_hash: &str) -> Result<(), ApiFailure> {
        self.lock()?.execute(
            "DELETE FROM device_flows WHERE poll_hash = ?1",
            params![poll_hash],
        )?;
        Ok(())
    }

    pub fn upsert_app(&self, app: &AppRow, now: &str) -> Result<(), ApiFailure> {
        self.lock()?.execute(
            "INSERT INTO apps (id, name, summary, category, homepage, repo, visibility, published, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                summary = excluded.summary,
                category = excluded.category,
                homepage = excluded.homepage,
                repo = excluded.repo,
                visibility = excluded.visibility,
                published = excluded.published,
                updated_at = excluded.updated_at",
            params![
                app.id,
                app.name,
                app.summary,
                app.category,
                app.homepage,
                app.repo,
                app.visibility.as_str(),
                i64::from(app.published),
                now
            ],
        )?;
        Ok(())
    }

    pub fn set_published(&self, id: &str, published: bool, now: &str) -> Result<bool, ApiFailure> {
        let changed = self.lock()?.execute(
            "UPDATE apps SET published = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, i64::from(published), now],
        )?;
        Ok(changed == 1)
    }

    pub fn delete_app(&self, id: &str) -> Result<bool, ApiFailure> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM grants WHERE app_id = ?1", params![id])?;
        let changed = conn.execute("DELETE FROM apps WHERE id = ?1", params![id])?;
        Ok(changed == 1)
    }

    pub fn app(&self, id: &str) -> Result<Option<AppRow>, ApiFailure> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, summary, category, homepage, repo, visibility, published FROM apps WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(read_app(row)?));
        }
        Ok(None)
    }

    pub fn apps(&self) -> Result<Vec<AppRow>, ApiFailure> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, summary, category, homepage, repo, visibility, published
             FROM apps ORDER BY id",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(read_app(row)?);
        }
        Ok(out)
    }

    pub fn grant(
        &self,
        github_id: u64,
        login: &str,
        app_id: &str,
        now: &str,
    ) -> Result<(), ApiFailure> {
        self.lock()?.execute(
            "INSERT INTO grants (github_id, app_id, login, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(github_id, app_id) DO UPDATE SET login = excluded.login",
            params![github_id as i64, app_id, login, now],
        )?;
        Ok(())
    }

    pub fn revoke(&self, github_id: u64, app_id: &str) -> Result<bool, ApiFailure> {
        let changed = self.lock()?.execute(
            "DELETE FROM grants WHERE github_id = ?1 AND app_id = ?2",
            params![github_id as i64, app_id],
        )?;
        Ok(changed == 1)
    }

    pub fn grants_for_app(&self, app_id: &str) -> Result<Vec<Grant>, ApiFailure> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT github_id, login FROM grants WHERE app_id = ?1 ORDER BY login")?;
        let mut rows = stmt.query(params![app_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(Grant {
                github_id: row.get::<_, i64>(0)? as u64,
                login: row.get(1)?,
            });
        }
        Ok(out)
    }

    pub fn has_grant(&self, github_id: u64, app_id: &str) -> Result<bool, ApiFailure> {
        let conn = self.lock()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM grants WHERE github_id = ?1 AND app_id = ?2",
            params![github_id as i64, app_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

pub struct NewFlow<'a> {
    pub poll_hash: &'a str,
    pub device_code: &'a str,
    pub interval_seconds: i64,
    pub expires_at: &'a str,
    pub next_poll_at: &'a str,
}

fn read_app(row: &rusqlite::Row<'_>) -> Result<AppRow, rusqlite::Error> {
    let visibility = Visibility::parse(&row.get::<_, String>(6)?).unwrap_or(Visibility::Private);
    Ok(AppRow {
        id: row.get(0)?,
        name: row.get(1)?,
        summary: row.get(2)?,
        category: row.get(3)?,
        homepage: row.get(4)?,
        repo: row.get(5)?,
        visibility,
        published: row.get::<_, i64>(7)? != 0,
    })
}

impl From<&AppRow> for AdminApp {
    fn from(app: &AppRow) -> Self {
        Self {
            id: app.id.clone(),
            name: app.name.clone(),
            summary: app.summary.clone(),
            category: app.category.clone(),
            homepage: app.homepage.clone(),
            repo: app.repo.clone(),
            visibility: app.visibility,
            published: app.published,
        }
    }
}

#[cfg(unix)]
fn tighten_db_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        let _ = std::fs::set_permissions(path, permissions);
    }
}

impl Database {
    pub fn categories(&self) -> Result<Vec<String>, ApiFailure> {
        let conn = self.lock()?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS categories (name TEXT PRIMARY KEY);")?;
        let mut stmt = conn.prepare("SELECT name FROM categories UNION SELECT category FROM apps WHERE category IS NOT NULL AND trim(category) != '' ORDER BY 1 COLLATE NOCASE")?;
        let rows = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(rows)
    }

    pub fn change_category(&self, name: &str, replacement: Option<&str>) -> Result<(), ApiFailure> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS categories (name TEXT PRIMARY KEY);")?;
        if let Some(new_name) = replacement {
            tx.execute(
                "INSERT OR IGNORE INTO categories(name) VALUES (?)",
                [new_name],
            )?;
        }
        if !name.is_empty() {
            tx.execute(
                "UPDATE apps SET category = ? WHERE category = ?",
                params![replacement, name],
            )?;
            if replacement != Some(name) {
                tx.execute("DELETE FROM categories WHERE name = ?", [name])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
