// Worker：消费 Task，执行转换，写盘 + 更新状态库
//
// 这一层负责把 dispatcher 产出的"需要转换"的任务真正落地：
//   1. converter::convert 算出输出字节 + 路径
//   2. 原子写入（写 .tmp 再 rename），保证不出现半成品
//   3. 更新 DB 状态
//
// 失败时把完整错误原因记入 files.error_msg，供 list --status failed 定位。

use crate::converter;
use crate::db::Db;
use crate::engine::Task;
use crate::model::{to_unix_secs, FileRecord, Directory, Status};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 单次转换结果（用于聚合统计与日志）。
pub struct TaskResult {
    pub source: PathBuf,
    pub rule_name: String,
    pub status: Status,
    pub error: Option<String>,
    pub output_size: Option<i64>,
}

/// 执行一个任务（同步）。返回结果。
pub fn run_task(task: &Task, directories: &[Directory], db: &Db) -> TaskResult {
    let dir = &directories[task.dir_index];
    let rule = &dir.rules[task.rule_index];
    let rule_name = rule.name.clone();
    let source = &task.source;

    // 预读源文件元数据，用于 DB 记录
    let (mtime, size) = match std::fs::metadata(source) {
        Ok(m) => {
            let mt = m
                .modified()
                .map(to_unix_secs)
                .unwrap_or(0);
            (mt, m.len() as i64)
        }
        Err(e) => {
            return fail(source, &rule_name, db, &format!("读取源文件元数据失败: {e}"));
        }
    };

    let now = to_unix_secs(SystemTime::now());

    // 记录为 pending（如果还没记录）
    let _ = upsert_pending(source, &rule_name, mtime, size, now, db);

    match converter::convert(source, rule) {
        Ok(out) => {
            // 原子写入：先写 .tmp，再 rename
            let tmp = out.output.with_extension(format!(
                "{}.tmp",
                out.output
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("dat")
            ));
            if let Err(e) = std::fs::write(&tmp, &out.bytes) {
                let msg = format!("写临时文件失败 ({}): {e}", tmp.display());
                return fail_with_record(source, &rule_name, &out.output, &msg, now, db);
            }
            if let Err(e) = std::fs::rename(&tmp, &out.output) {
                let msg = format!("重命名失败 ({} → {}): {e}", tmp.display(), out.output.display());
                return fail_with_record(source, &rule_name, &out.output, &msg, now, db);
            }
            let out_size = out.bytes.len() as i64;
            // 完成但超限：记 warn 到 error_msg，状态仍 done
            let warn = if !out.within_limit {
                Some(format!(
                    "输出 {} 字节超过 size_limit 上限",
                    out_size
                ))
            } else {
                None
            };
            let _ = db.mark_done(source, &rule_name, &out.output, out_size, warn.clone(), now);
            tracing::info!(
                source = %source.display(),
                output = %out.output.display(),
                quality = out.quality,
                "转换完成"
            );
            TaskResult {
                source: source.clone(),
                rule_name,
                status: Status::Done,
                error: warn,
                output_size: Some(out_size),
            }
        }
        Err(e) => {
            let msg = format!("{e}");
            // 预估输出路径（用于记录）
            let out_guess = guess_output_path(source, rule);
            fail_with_record(source, &rule_name, &out_guess, &msg, now, db)
        }
    }
}

fn fail(source: &Path, rule_name: &str, db: &Db, msg: &str) -> TaskResult {
    let now = to_unix_secs(SystemTime::now());
    let out = source.with_extension("err");
    fail_with_record(source, rule_name, &out, msg, now, db)
}

fn fail_with_record(
    source: &Path,
    rule_name: &str,
    output: &Path,
    msg: &str,
    now: i64,
    db: &Db,
) -> TaskResult {
    let _ = db.mark_failed(source, rule_name, output, msg, now);
    tracing::warn!(source = %source.display(), rule = rule_name, error = msg, "转换失败");
    TaskResult {
        source: source.to_path_buf(),
        rule_name: rule_name.to_string(),
        status: Status::Failed,
        error: Some(msg.to_string()),
        output_size: None,
    }
}

fn upsert_pending(source: &Path, rule_name: &str, mtime: i64, size: i64, now: i64, db: &Db) {
    let rec = FileRecord {
        id: None,
        source_path: source.to_path_buf(),
        source_mtime: mtime,
        source_size: size,
        rule_name: rule_name.to_string(),
        output_path: source.with_extension("pending"),
        status: Status::Pending,
        error_msg: None,
        attempt_count: 0,
        output_size: None,
        created_at: now,
        updated_at: now,
    };
    let _ = db.upsert(&rec);
}

/// 转换失败时，无法从 converter 拿到真实输出路径，用规则的最大质量估一个，仅用于 DB 记录。
fn guess_output_path(source: &Path, rule: &crate::model::Rule) -> PathBuf {
    use crate::converter::namer::{output_path, NamingInput};
    output_path(&NamingInput {
        source,
        resize: rule.resize,
        quality: rule
            .size_limit
            .map(|s| s.max_quality)
            .unwrap_or(100),
        format: rule.format,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::model::{Directory, Fit, ResizeSpec, Rule};
    use image::{DynamicImage, RgbaImage};

    fn mk(dir: &Path) -> (Vec<Directory>, Db, PathBuf) {
        let src = dir.join("photo.png");
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 200, image::Rgba([10, 20, 30, 255])));
        img.save(&src).unwrap();
        let directories = vec![Directory {
            path: dir.to_path_buf(),
            recursive: true,
            rules: vec![Rule {
                name: "thumb".into(),
                resize: Some(ResizeSpec {
                    max_width: 50,
                    max_height: 50,
                    fit: Fit::Inside,
                    no_upscale: false,
                }),
                format: None,
                size_limit: None,
                jpeg_bg: None,
            }],
        }];
        let db = Db::open_memory().unwrap();
        (directories, db, src)
    }

    #[test]
    fn run_task_writes_output_and_marks_done() {
        let tmp = tempfile::tempdir().unwrap();
        let (dirs, db, src) = mk(tmp.path());
        let task = Task {
            source: src.clone(),
            dir_index: 0,
            rule_index: 0,
        };
        let res = run_task(&task, &dirs, &db);
        assert_eq!(res.status, Status::Done);

        // 输出文件存在且可被防套娃识别
        let out = tmp.path().join("photo_50x50_q100.png");
        assert!(out.exists(), "输出应存在");
        assert!(converter::namer::is_output_file(&out));

        // DB 状态
        let rec = db.find(&src, "thumb").unwrap().unwrap();
        assert_eq!(rec.status, Status::Done);
    }

    #[test]
    fn run_task_records_failure() {
        let tmp = tempfile::tempdir().unwrap();
        // 写一个损坏的 png
        let src = tmp.path().join("bad.png");
        std::fs::write(&src, b"not an image").unwrap();
        let dirs = vec![Directory {
            path: tmp.path().to_path_buf(),
            recursive: true,
            rules: vec![Rule {
                name: "r".into(),
                resize: None,
                format: None,
                size_limit: None,
                jpeg_bg: None,
            }],
        }];
        let db = Db::open_memory().unwrap();
        let task = Task {
            source: src.clone(),
            dir_index: 0,
            rule_index: 0,
        };
        let res = run_task(&task, &dirs, &db);
        assert_eq!(res.status, Status::Failed);
        assert!(res.error.is_some());
        let rec = db.find(&src, "r").unwrap().unwrap();
        assert_eq!(rec.status, Status::Failed);
        assert!(rec.error_msg.clone().unwrap().contains("解码")
            || rec.error_msg.clone().unwrap().contains("decode")
            || rec.error_msg.clone().unwrap().contains("Decode"));
    }

    #[test]
    fn atomic_write_no_partial_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (dirs, db, src) = mk(tmp.path());
        let task = Task {
            source: src.clone(),
            dir_index: 0,
            rule_index: 0,
        };
        run_task(&task, &dirs, &db);
        // 不应残留 .tmp 文件
        let tmps: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .collect();
        assert!(tmps.is_empty(), "不应残留临时文件: {tmps:?}");
    }
}
