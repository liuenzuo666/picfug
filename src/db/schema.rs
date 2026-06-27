// SQLite schema
//
// files 表记录每个 (源文件, 规则) 的处理状态。
// 注意：防套娃不依赖此表（走文件名识别），此表仅用于状态查询/失败定位/重试。

pub const SCHEMA: &str = r#"
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;

CREATE TABLE IF NOT EXISTS files (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  source_path   TEXT NOT NULL,
  source_mtime  INTEGER NOT NULL,
  source_size   INTEGER NOT NULL,
  rule_name     TEXT NOT NULL,
  output_path   TEXT NOT NULL,
  status        TEXT NOT NULL,        -- pending|done|failed|skipped
  error_msg     TEXT,
  attempt_count INTEGER NOT NULL DEFAULT 0,
  output_size   INTEGER,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  UNIQUE(source_path, rule_name)
);

CREATE INDEX IF NOT EXISTS idx_files_status ON files(status);
CREATE INDEX IF NOT EXISTS idx_files_output ON files(output_path);
CREATE INDEX IF NOT EXISTS idx_files_source ON files(source_path);
"#;
