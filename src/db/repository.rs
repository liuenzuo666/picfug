// 数据库访问层（repository）
//
// 所有 SQL 集中在此。Db 是对 rusqlite::Connection 的薄封装。
// 写入采用 upsert（INSERT ... ON CONFLICT UPDATE），保证 (source,rule) 唯一去重。

use crate::db::schema::SCHEMA;
use crate::error::PicfugError;
use crate::model::{FileRecord, Status};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 线程安全的数据库句柄。
/// 由于 rusqlite::Connection 本身非 Sync，用 Mutex 保护。
/// 转换是 IO+CPU 密集，DB 操作占比极小，单连接 + Mutex 足够。
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// 打开/创建数据库文件并初始化 schema。
    pub fn open(path: &Path) -> Result<Self, PicfugError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PicfugError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        let conn = Connection::open(path).map_err(|e| PicfugError::DatabaseInit {
            path: path.to_path_buf(),
            source: e,
        })?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    /// 内存数据库（仅用于测试）。
    pub fn open_memory() -> Result<Self, PicfugError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    /// Upsert：若 (source,rule) 已存在则更新，否则插入。返回行 id。
    pub fn upsert(&self, rec: &FileRecord) -> Result<i64, PicfugError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"INSERT INTO files
                 (source_path, source_mtime, source_size, rule_name, output_path,
                  status, error_msg, attempt_count, output_size, created_at, updated_at)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
               ON CONFLICT(source_path, rule_name) DO UPDATE SET
                 source_mtime=excluded.source_mtime,
                 source_size=excluded.source_size,
                 output_path=excluded.output_path,
                 status=excluded.status,
                 error_msg=excluded.error_msg,
                 attempt_count=excluded.attempt_count,
                 output_size=excluded.output_size,
                 updated_at=excluded.updated_at"#,
            params![
                rec.source_path.to_string_lossy(),
                rec.source_mtime,
                rec.source_size,
                rec.rule_name,
                rec.output_path.to_string_lossy(),
                rec.status.as_str(),
                rec.error_msg,
                rec.attempt_count,
                rec.output_size,
                rec.created_at,
                rec.updated_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 按 (source, rule) 查询单条记录。
    pub fn find(&self, source: &Path, rule: &str) -> Result<Option<FileRecord>, PicfugError> {
        let conn = self.conn.lock().unwrap();
        let rec = conn
            .query_row(
                "SELECT * FROM files WHERE source_path=?1 AND rule_name=?2",
                params![source.to_string_lossy(), rule],
                row_to_record,
            )
            .optional()?;
        Ok(rec)
    }

    /// 把指定 (source,rule) 标记为 done 并 +1 attempt_count。
    /// error_msg 用于记录"完成但超限"之类的软告警（仍算 done，区别于真正的 failed）。
    pub fn mark_done(
        &self,
        source: &Path,
        rule: &str,
        output: &Path,
        output_size: i64,
        error_msg: Option<String>,
        now: i64,
    ) -> Result<(), PicfugError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"UPDATE files SET status='done', output_path=?1, output_size=?2,
               error_msg=?3, attempt_count=attempt_count+1, updated_at=?4
               WHERE source_path=?5 AND rule_name=?6"#,
            params![
                output.to_string_lossy(),
                output_size,
                error_msg,
                now,
                source.to_string_lossy(),
                rule
            ],
        )?;
        Ok(())
    }

    pub fn mark_failed(
        &self,
        source: &Path,
        rule: &str,
        output: &Path,
        error_msg: &str,
        now: i64,
    ) -> Result<(), PicfugError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"UPDATE files SET status='failed', output_path=?1, error_msg=?2,
               attempt_count=attempt_count+1, updated_at=?3
               WHERE source_path=?4 AND rule_name=?5"#,
            params![
                output.to_string_lossy(),
                error_msg,
                now,
                source.to_string_lossy(),
                rule
            ],
        )?;
        Ok(())
    }

    /// 按状态查询记录，可选过滤目录。按 updated_at 倒序。
    pub fn list_by_status(
        &self,
        status: Option<Status>,
        dir_filter: Option<&str>,
        rule_filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<FileRecord>, PicfugError> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from("SELECT * FROM files WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(s) = status {
            sql.push_str(" AND status=?");
            args.push(Box::new(s.as_str().to_string()));
        }
        if let Some(d) = dir_filter {
            sql.push_str(" AND source_path LIKE ?");
            args.push(Box::new(format!("{}%", d)));
        }
        if let Some(r) = rule_filter {
            sql.push_str(" AND rule_name=?");
            args.push(Box::new(r.to_string()));
        }
        sql.push_str(" ORDER BY updated_at DESC LIMIT ? OFFSET ?");
        args.push(Box::new(limit));
        args.push(Box::new(offset));

        let mut stmt = conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(params_ref.as_slice(), row_to_record)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// 按状态计数。
    pub fn count_by_status(&self) -> Result<StatusCounts, PicfugError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM files GROUP BY status")?;
        let rows = stmt.query_map([], |row| {
            let s: String = row.get(0)?;
            let c: i64 = row.get(1)?;
            Ok((s, c))
        })?;
        let mut counts = StatusCounts::default();
        for r in rows {
            let (s, c) = r?;
            match Status::parse(&s) {
                Some(Status::Pending) => counts.pending = c,
                Some(Status::Done) => counts.done = c,
                Some(Status::Failed) => counts.failed = c,
                Some(Status::Skipped) => counts.skipped = c,
                None => {}
            }
        }
        Ok(counts)
    }

    /// 把所有 failed 记录重置为 pending（供 retry 命令）。
    /// 可按目录/规则过滤。返回受影响行数。
    pub fn reset_failed_to_pending(
        &self,
        dir_filter: Option<&str>,
        rule_filter: Option<&str>,
        now: i64,
    ) -> Result<i64, PicfugError> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from("UPDATE files SET status='pending', updated_at=? WHERE status='failed'");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now)];
        if let Some(d) = dir_filter {
            sql.push_str(" AND source_path LIKE ?");
            args.push(Box::new(format!("{}%", d)));
        }
        if let Some(r) = rule_filter {
            sql.push_str(" AND rule_name=?");
            args.push(Box::new(r.to_string()));
        }
        let params_ref: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let n = conn.execute(&sql, params_ref.as_slice())?;
        Ok(n as i64)
    }
}

#[derive(Debug, Clone, Default)]
pub struct StatusCounts {
    pub pending: i64,
    pub done: i64,
    pub failed: i64,
    pub skipped: i64,
}

fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<FileRecord> {
    Ok(FileRecord {
        id: Some(row.get("id")?),
        source_path: PathBuf::from(row.get::<_, String>("source_path")?),
        source_mtime: row.get("source_mtime")?,
        source_size: row.get("source_size")?,
        rule_name: row.get("rule_name")?,
        output_path: PathBuf::from(row.get::<_, String>("output_path")?),
        status: Status::parse(&row.get::<_, String>("status")?).unwrap_or(Status::Failed),
        error_msg: row.get("error_msg")?,
        attempt_count: row.get("attempt_count")?,
        output_size: row.get("output_size")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FileRecord;
    use std::time::SystemTime;

    fn rec(source: &str, rule: &str, status: Status) -> FileRecord {
        let now = crate::model::to_unix_secs(SystemTime::now());
        FileRecord {
            id: None,
            source_path: PathBuf::from(source),
            source_mtime: now,
            source_size: 100,
            rule_name: rule.to_string(),
            output_path: PathBuf::from("/out/o.png"),
            status,
            error_msg: None,
            attempt_count: 0,
            output_size: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn upsert_dedupes_by_source_rule() {
        let db = Db::open_memory().unwrap();
        let now = crate::model::to_unix_secs(SystemTime::now());
        let mut r = rec("/a/x.png", "thumb", Status::Pending);
        db.upsert(&r).unwrap();
        r.status = Status::Done;
        r.updated_at = now;
        db.upsert(&r).unwrap();
        let got = db.find(Path::new("/a/x.png"), "thumb").unwrap().unwrap();
        assert_eq!(got.status, Status::Done);
    }

    #[test]
    fn mark_done_and_failed() {
        let db = Db::open_memory().unwrap();
        let now = crate::model::to_unix_secs(SystemTime::now());
        let r = rec("/a/y.png", "hd", Status::Pending);
        db.upsert(&r).unwrap();
        db.mark_done(Path::new("/a/y.png"), "hd", Path::new("/o"), 500, None, now)
            .unwrap();
        let got = db.find(Path::new("/a/y.png"), "hd").unwrap().unwrap();
        assert_eq!(got.status, Status::Done);
        assert_eq!(got.output_size, Some(500));
        assert_eq!(got.attempt_count, 1);

        db.mark_failed(Path::new("/a/y.png"), "hd", Path::new("/o"), "boom", now)
            .unwrap();
        let got = db.find(Path::new("/a/y.png"), "hd").unwrap().unwrap();
        assert_eq!(got.status, Status::Failed);
        assert_eq!(got.error_msg.as_deref(), Some("boom"));
    }

    #[test]
    fn list_by_status_and_filters() {
        let db = Db::open_memory().unwrap();
        db.upsert(&rec("/dir/a.png", "thumb", Status::Done)).unwrap();
        db.upsert(&rec("/dir/b.png", "thumb", Status::Failed)).unwrap();
        db.upsert(&rec("/dir/c.png", "hd", Status::Failed)).unwrap();
        db.upsert(&rec("/other/d.png", "thumb", Status::Done)).unwrap();

        let failed = db.list_by_status(Some(Status::Failed), None, None, 100, 0).unwrap();
        assert_eq!(failed.len(), 2);

        let dir_failed = db
            .list_by_status(Some(Status::Failed), Some("/dir"), None, 100, 0)
            .unwrap();
        assert_eq!(dir_failed.len(), 2);

        let rule_failed = db
            .list_by_status(Some(Status::Failed), None, Some("hd"), 100, 0)
            .unwrap();
        assert_eq!(rule_failed.len(), 1);
        assert_eq!(rule_failed[0].rule_name, "hd");
    }

    #[test]
    fn counts() {
        let db = Db::open_memory().unwrap();
        db.upsert(&rec("/a", "r", Status::Done)).unwrap();
        db.upsert(&rec("/b", "r", Status::Done)).unwrap();
        db.upsert(&rec("/c", "r", Status::Failed)).unwrap();
        db.upsert(&rec("/d", "r", Status::Pending)).unwrap();
        let c = db.count_by_status().unwrap();
        assert_eq!(c.done, 2);
        assert_eq!(c.failed, 1);
        assert_eq!(c.pending, 1);
    }

    #[test]
    fn retry_resets_failed() {
        let db = Db::open_memory().unwrap();
        let now = crate::model::to_unix_secs(SystemTime::now());
        db.upsert(&rec("/dir/a.png", "thumb", Status::Failed)).unwrap();
        db.upsert(&rec("/other/b.png", "thumb", Status::Failed)).unwrap();
        let n = db.reset_failed_to_pending(Some("/dir"), None, now).unwrap();
        assert_eq!(n, 1);
        let pending = db
            .list_by_status(Some(Status::Pending), None, None, 100, 0)
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].source_path, PathBuf::from("/dir/a.png"));
    }
}
