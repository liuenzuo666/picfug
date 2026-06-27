// CLI 命令层
//
// clap derive 定义子命令，每个 run_* 是一个命令的入口。
// 共用的"加载 config + 打开 db + 初始化日志"逻辑抽到 common() 里。

use crate::config::Config;
use crate::db::Db;
use crate::engine::{self};
use crate::model::{Directory, Status};
use crate::worker;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "picfug", version, about = "图片文件自动转换 CLI 守护进程")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// 校验配置文件
    Check(ConfigArgs),
    /// 单次全量扫描转换后退出
    Once(OnceArgs),
    /// 启动守护进程，持续监听 + 轮询
    Start(StartArgs),
    /// 查看状态汇总
    Status(ConfigArgs),
    /// 按状态列出文件（默认列出失败项，方便定位修复）
    List(ListArgs),
    /// 重试所有失败项
    Retry(RetryArgs),
    /// 查看日志
    Logs(LogsArgs),
}

#[derive(clap::Args, Debug)]
pub struct ConfigArgs {
    /// 配置文件路径
    #[arg(short, long, default_value = "config.yaml")]
    pub config: PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct OnceArgs {
    #[arg(short, long, default_value = "config.yaml")]
    pub config: PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct StartArgs {
    #[arg(short, long, default_value = "config.yaml")]
    pub config: PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct ListArgs {
    #[arg(short, long, default_value = "config.yaml")]
    pub config: PathBuf,
    /// 状态过滤：done/failed/pending/skipped（默认 failed）
    #[arg(long, default_value = "failed")]
    pub status: String,
    /// 目录前缀过滤
    #[arg(long)]
    pub dir: Option<String>,
    /// 规则名过滤
    #[arg(long)]
    pub rule: Option<String>,
    /// 限制条数
    #[arg(long, default_value_t = 50)]
    pub limit: i64,
    /// 偏移
    #[arg(long, default_value_t = 0)]
    pub offset: i64,
    /// JSON 输出（便于脚本消费）
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct RetryArgs {
    #[arg(short, long, default_value = "config.yaml")]
    pub config: PathBuf,
    #[arg(long)]
    pub dir: Option<String>,
    #[arg(long)]
    pub rule: Option<String>,
    /// 重试后是否立即跑一次转换（默认是）
    #[arg(long, default_value_t = true)]
    pub run: bool,
}

#[derive(clap::Args, Debug)]
pub struct LogsArgs {
    #[arg(short, long, default_value = "config.yaml")]
    pub config: PathBuf,
    /// 持续跟踪（类似 tail -f）
    #[arg(short, long, default_value_t = false)]
    pub follow: bool,
}

// ===== 共用加载 =====

fn load_config_and_db(path: &PathBuf) -> anyhow::Result<(Config, Db)> {
    let cfg = Config::load(path)?;
    let db = Db::open(&cfg.database.path)?;
    Ok((cfg, db))
}

/// 把一批候选文件跑一遍 dispatcher → worker，返回处理统计。
fn process_files(
    files: Vec<PathBuf>,
    directories: &[Directory],
    db: &Db,
) -> ProcessSummary {
    let mut summary = ProcessSummary::default();
    for file in &files {
        let decisions = engine::classify(file, directories);
        if decisions.is_empty() {
            continue; // 不在任何目录管辖
        }
        for d in decisions {
            match d {
                engine::Decision::Convert(task) => {
                    let res = worker::run_task(&task, directories, db);
                    match res.status {
                        Status::Done => summary.done += 1,
                        Status::Failed => summary.failed += 1,
                        _ => {}
                    }
                }
                engine::Decision::SkipOutput => summary.skipped_output += 1,
                engine::Decision::SkipNotImage => {}
                engine::Decision::SkipExists => summary.skipped_exists += 1,
            }
        }
    }
    summary
}

#[derive(Debug, Default, Clone, Copy)]
struct ProcessSummary {
    done: usize,
    failed: usize,
    skipped_output: usize,
    skipped_exists: usize,
}

// ===== 命令实现 =====

pub async fn run_check(args: ConfigArgs) -> anyhow::Result<()> {
    let cfg = Config::load(&args.config)?;
    println!("✓ 配置校验通过");
    println!("  目录数: {}", cfg.directories.len());
    for d in &cfg.directories {
        let rule_names: Vec<&str> = d.rules.iter().map(|r| r.name.as_str()).collect();
        println!(
            "  - {} (recursive={}, rules=[{}])",
            d.path.display(),
            d.recursive,
            rule_names.join(", ")
        );
    }
    println!("  数据库: {}", cfg.database.path.display());
    Ok(())
}

pub async fn run_once(args: OnceArgs) -> anyhow::Result<()> {
    let (cfg, db) = load_config_and_db(&args.config)?;
    let _guard = crate::log::init(cfg.log.as_ref(), false)?;
    let directories = cfg.to_directories();

    tracing::info!("开始全量扫描转换");
    let files = engine::scanner::scan_all(&directories);
    tracing::info!("扫描到 {} 个候选文件", files.len());

    let summary = process_files(files, &directories, &db);
    println!("全量转换完成:");
    println!("  ✓ 完成 (done)      : {}", summary.done);
    println!("  ✗ 失败 (failed)    : {}", summary.failed);
    println!("  ⤷ 跳过-输出已存在  : {}", summary.skipped_exists);
    println!("  ⤷ 跳过-识别为输出  : {}", summary.skipped_output);
    if summary.failed > 0 {
        println!("\n提示: 使用 `picfug list --status failed` 查看失败文件及原因");
    }
    Ok(())
}

pub async fn run_start(args: StartArgs) -> anyhow::Result<()> {
    let (cfg, db) = load_config_and_db(&args.config)?;
    let _guard = crate::log::init(cfg.log.as_ref(), false)?;
    let directories = cfg.to_directories();
    let watcher_cfg = cfg.watcher.clone().unwrap_or_default();

    tracing::info!("启动守护进程");

    // 1. 首次全量
    tracing::info!("执行首次全量扫描");
    let files = engine::scanner::scan_all(&directories);
    let s = process_files(files, &directories, &db);
    tracing::info!("首次全量完成: done={} failed={}", s.done, s.failed);

    // 2. 启动 watcher + poller
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<PathBuf>>(1024);

    // poller
    {
        let tx = tx.clone();
        let dirs = directories.clone();
        tokio::spawn(crate::engine::poller::spawn(dirs, watcher_cfg.poll_interval.0, tx));
    }

    // watcher（notify 是同步的，单独线程转发到 tokio channel）
    let _debouncer = {
        let (std_rx, debouncer) = crate::engine::watcher::spawn(&directories, watcher_cfg.debounce.0)?;
        let tx = tx.clone();
        tokio::task::spawn_blocking(move || {
            while let Ok(paths) = std_rx.recv() {
                if tx.blocking_send(paths).is_err() {
                    break;
                }
            }
        });
        // 注意：debouncer 必须保活，这里用泄漏语义——守护进程生命周期内常驻。
        // 为简洁，把它装进一个会被一直持有的变量。
        std::mem::forget(debouncer);
    };

    tracing::info!("守护进程就绪，开始监听");

    // 3. 主循环：消费事件
    loop {
        match rx.recv().await {
            Some(paths) => {
                let s = process_files(paths, &directories, &db);
                if s.done + s.failed > 0 {
                    tracing::info!("处理一批事件: done={} failed={}", s.done, s.failed);
                }
            }
            None => break,
        }
    }
    Ok(())
}

pub async fn run_status(args: ConfigArgs) -> anyhow::Result<()> {
    let (cfg, db) = load_config_and_db(&args.config)?;
    let counts = db.count_by_status()?;
    println!("picfug 状态汇总 (db: {})", cfg.database.path.display());
    println!("  ✓ 完成 (done)    : {}", counts.done);
    println!("  ✗ 失败 (failed)  : {}", counts.failed);
    println!("  ⏳ 待处理(pending): {}", counts.pending);
    println!("  ⤷ 跳过 (skipped) : {}", counts.skipped);
    Ok(())
}

pub async fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let (_cfg, db) = load_config_and_db(&args.config)?;
    let status = Status::parse(&args.status);
    let records = db.list_by_status(
        status,
        args.dir.as_deref(),
        args.rule.as_deref(),
        args.limit,
        args.offset,
    )?;

    if args.json {
        let out: Vec<serde_json_lite::RecordJson> =
            records.iter().map(serde_json_lite::RecordJson::from).collect();
        println!("{}", serde_json_lite::to_string(&out));
        return Ok(());
    }

    if records.is_empty() {
        println!("无 {} 记录", args.status);
        return Ok(());
    }
    println!("共 {} 条 {} 记录:", records.len(), args.status);
    println!("{:-<100}", "");
    for r in &records {
        println!("源文件 : {}", r.source_path.display());
        println!("规则   : {}", r.rule_name);
        if let Some(msg) = &r.error_msg {
            println!("原因   : {msg}");
        }
        println!("尝试   : {}", r.attempt_count);
        println!("时间   : {}", fmt_ts(r.updated_at));
        println!("{:-<100}", "");
    }
    Ok(())
}

pub async fn run_retry(args: RetryArgs) -> anyhow::Result<()> {
    let (cfg, db) = load_config_and_db(&args.config)?;
    let now = crate::model::to_unix_secs(std::time::SystemTime::now());
    let n = db.reset_failed_to_pending(args.dir.as_deref(), args.rule.as_deref(), now)?;
    println!("已重置 {} 条失败记录为 pending", n);

    if args.run && n > 0 {
        let directories = cfg.to_directories();
        // 只重跑那些 pending 且原本是 failed 的——简化为全量扫描再过滤
        let pending = db.list_by_status(Some(Status::Pending), None, None, i64::MAX, 0)?;
        let summary = process_files(
            pending.iter().map(|r| r.source_path.clone()).collect(),
            &directories,
            &db,
        );
        println!("重试完成: done={} failed={}", summary.done, summary.failed);
    }
    Ok(())
}

pub async fn run_logs(args: LogsArgs) -> anyhow::Result<()> {
    let cfg = Config::load(&args.config)?;
    let log_dir = cfg
        .log
        .as_ref()
        .map(|l| l.dir.clone())
        .unwrap_or_else(|| PathBuf::from("./logs"));
    // 找最新的日志文件
    let mut latest: Option<PathBuf> = None;
    if log_dir.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(&log_dir)?
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("picfug.log")
            })
            .collect();
        entries.sort_by_key(|e| e.path());
        latest = entries.last().map(|e| e.path());
    }
    let Some(file) = latest else {
        println!("未找到日志文件于 {}", log_dir.display());
        return Ok(());
    };

    // 输出全部内容（简单实现；follow 用 tail 循环）
    let content = std::fs::read_to_string(&file)?;
    print!("{content}");
    if args.follow {
        use std::io::{BufRead, Seek, SeekFrom};
        let f = std::fs::OpenOptions::new().read(true).open(&file)?;
        let mut reader = std::io::BufReader::new(f);
        reader.seek(SeekFrom::End(0))?;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? > 0 {
                print!("{line}");
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
    Ok(())
}

fn fmt_ts(ts: i64) -> String {
    time_fmt::format_utc(ts)
}

// 极简的 JSON / 时间格式化辅助（避免引入额外依赖）
mod serde_json_lite {
    use crate::model::FileRecord;
    use std::fmt::Write;

    pub struct RecordJson<'a>(pub &'a FileRecord);

    impl<'a> From<&'a FileRecord> for RecordJson<'a> {
        fn from(r: &'a FileRecord) -> Self {
            RecordJson(r)
        }
    }

    pub fn to_string(recs: &[RecordJson]) -> String {
        let mut s = String::from("[");
        for (i, r) in recs.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&r.0.to_json());
        }
        s.push(']');
        s
    }

    impl FileRecord {
        pub fn to_json(&self) -> String {
            let mut s = String::new();
            write!(
                s,
                r#"{{"source_path":{:?},"rule_name":{:?},"status":{:?},"error_msg":{:?},"attempt_count":{},"output_path":{:?}}}"#,
                self.source_path.to_string_lossy(),
                self.rule_name,
                self.status.as_str(),
                self.error_msg,
                self.attempt_count,
                self.output_path.to_string_lossy(),
            )
            .unwrap();
            s
        }
    }
}

/// 极简的 UTC 时间格式化（仅用标准库，避免引入 chrono）。
/// 把 unix 秒转为 "YYYY-MM-DD HH:MM:SS UTC"。
mod time_fmt {
    pub fn format_utc(secs: i64) -> String {
        let days_since_epoch = secs.div_euclid(86400);
        let secs_of_day = secs.rem_euclid(86400);
        let (y, m, d) = civil_from_days(days_since_epoch);
        let hh = secs_of_day / 3600;
        let mm = (secs_of_day % 3600) / 60;
        let ss = secs_of_day % 60;
        format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} UTC")
    }

    /// Howard Hinnant 的 days→civil 算法（无外部依赖）。
    fn civil_from_days(z: i64) -> (i64, u32, u32) {
        let z = z + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = (z - era * 146097) as i64; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
        (if m <= 2 { y + 1 } else { y }, m, d)
    }
}
