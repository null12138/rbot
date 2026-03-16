use chrono::{DateTime, Local, NaiveDate};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct MemoryStore {
    conn: Arc<Mutex<Connection>>,
    base_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub role: String,
    pub content: String,
}

impl MemoryStore {
    pub fn new(db_path: &str, base_dir: &str) -> anyhow::Result<Self> {
        if let Some(parent) = Path::new(db_path).parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir_all(base_dir)?;
        let conn = Connection::open(db_path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            base_dir: PathBuf::from(base_dir),
        };
        store.init_tables()?;
        Ok(store)
    }

    fn init_tables(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY,
                chat_id INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                ts TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS summaries (
                id INTEGER PRIMARY KEY,
                chat_id INTEGER NOT NULL,
                day TEXT NOT NULL,
                summary TEXT NOT NULL,
                ts TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS long_memory (
                id INTEGER PRIMARY KEY,
                chat_id INTEGER NOT NULL,
                content TEXT NOT NULL,
                tags TEXT,
                ts TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS allowlist (
                id INTEGER PRIMARY KEY,
                tool TEXT NOT NULL,
                command TEXT NOT NULL,
                added_by INTEGER NOT NULL,
                ts TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS schedules (
                id INTEGER PRIMARY KEY,
                chat_id INTEGER NOT NULL,
                cron TEXT NOT NULL,
                action_type TEXT NOT NULL,
                action_payload TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                ts TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn append_message(&self, chat_id: i64, role: &str, content: &str) -> anyhow::Result<()> {
        let ts = Local::now().to_rfc3339();
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO messages (chat_id, role, content, ts) VALUES (?1, ?2, ?3, ?4)",
                params![chat_id, role, content, ts],
            )?;
        }
        self.append_daily_log(chat_id, role, content)?;
        Ok(())
    }

    pub fn get_recent_messages(
        &self,
        chat_id: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<StoredMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT role, content FROM messages WHERE chat_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![chat_id, limit as i64], |row| {
            Ok(StoredMessage {
                role: row.get(0)?,
                content: row.get(1)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out.reverse();
        Ok(out)
    }

    pub fn get_summary(&self, chat_id: i64, day: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT summary FROM summaries WHERE chat_id = ?1 AND day = ?2 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![chat_id, day])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn set_summary(&self, chat_id: i64, day: &str, summary: &str) -> anyhow::Result<()> {
        let ts = Local::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO summaries (chat_id, day, summary, ts) VALUES (?1, ?2, ?3, ?4)",
            params![chat_id, day, summary, ts],
        )?;
        Ok(())
    }

    pub fn add_long_memory(&self, chat_id: i64, content: &str, tags: &str) -> anyhow::Result<()> {
        let ts = Local::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO long_memory (chat_id, content, tags, ts) VALUES (?1, ?2, ?3, ?4)",
            params![chat_id, content, tags, ts],
        )?;
        Ok(())
    }

    pub fn search_long_memory(&self, chat_id: i64, query: &str, limit: usize) -> anyhow::Result<Vec<String>> {
        let like = format!("%{}%", query);
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT content FROM long_memory WHERE chat_id = ?1 AND content LIKE ?2 ORDER BY id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![chat_id, like, limit as i64], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn add_allowlist(&self, tool: &str, command: &str, added_by: i64) -> anyhow::Result<()> {
        let ts = Local::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO allowlist (tool, command, added_by, ts) VALUES (?1, ?2, ?3, ?4)",
            params![tool, command, added_by, ts],
        )?;
        Ok(())
    }

    pub fn load_allowlist(&self, tool: &str) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT command FROM allowlist WHERE tool = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![tool], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }


    pub fn add_schedule(
        &self,
        chat_id: i64,
        cron: &str,
        action_type: &str,
        action_payload: &str,
    ) -> anyhow::Result<i64> {
        let ts = Local::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO schedules (chat_id, cron, action_type, action_payload, enabled, ts) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
            params![chat_id, cron, action_type, action_payload, ts],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn disable_schedule(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE schedules SET enabled = 0 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn list_schedules(&self) -> anyhow::Result<Vec<(i64, i64, String, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, chat_id, cron, action_type, action_payload FROM schedules WHERE enabled = 1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
    pub fn list_chats_for_day(&self, day: &str) -> anyhow::Result<Vec<i64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT chat_id FROM messages WHERE ts LIKE ?1",
        )?;
        let pattern = format!("{}%", day);
        let rows = stmt.query_map(params![pattern], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn daily_log_path(&self, chat_id: i64, date: NaiveDate) -> PathBuf {
        self.base_dir
            .join(chat_id.to_string())
            .join(format!("{}.md", date))
    }

    fn memory_file_path(&self, chat_id: i64) -> PathBuf {
        self.base_dir
            .join(chat_id.to_string())
            .join("MEMORY.md")
    }

    fn sleep_file_path(&self, chat_id: i64, date: NaiveDate) -> PathBuf {
        self.base_dir
            .join(chat_id.to_string())
            .join("sleep")
            .join(format!("{}.md", date))
    }

    fn append_daily_log(&self, chat_id: i64, role: &str, content: &str) -> anyhow::Result<()> {
        let date = Local::now().date_naive();
        let path = self.daily_log_path(chat_id, date);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let line = format!("- [{}] {}\n", role, content.replace('\n', " "));
        fs::write(&path, format!("{}{}", self.read_file(&path)?, line))?;
        Ok(())
    }

    pub fn read_daily_log(&self, chat_id: i64, date: NaiveDate) -> anyhow::Result<String> {
        let path = self.daily_log_path(chat_id, date);
        self.read_file(&path)
    }

    pub fn append_long_memory_file(&self, chat_id: i64, content: &str) -> anyhow::Result<()> {
        let path = self.memory_file_path(chat_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let entry = format!("- {}\n", content.trim());
        fs::write(&path, format!("{}{}", self.read_file(&path)?, entry))?;
        Ok(())
    }

    pub fn write_sleep_archive(&self, chat_id: i64, date: NaiveDate, content: &str) -> anyhow::Result<()> {
        let path = self.sleep_file_path(chat_id, date);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)?;
        Ok(())
    }

    fn read_file(&self, path: &Path) -> anyhow::Result<String> {
        if !path.exists() {
            return Ok(String::new());
        }
        Ok(fs::read_to_string(path)?)
    }

    pub fn write_heartbeat(&self, content: &str) -> anyhow::Result<()> {
        let path = self.base_dir.join("heartbeat.txt");
        fs::write(path, content)?;
        Ok(())
    }

    pub fn now_date(&self) -> NaiveDate {
        Local::now().date_naive()
    }

    pub fn now_rfc3339(&self) -> String {
        Local::now().to_rfc3339()
    }

    pub fn parse_date(&self, s: &str) -> anyhow::Result<NaiveDate> {
        Ok(NaiveDate::parse_from_str(s, "%Y-%m-%d")?)
    }
}

pub fn local_day_string(ts: DateTime<Local>) -> String {
    ts.format("%Y-%m-%d").to_string()
}
