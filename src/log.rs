// 日志初始化：滚动文件 + 控制台双输出
//
// rotation: daily / hourly / never
// 控制台始终输出到 stderr（不污染可能的管道输出，如 status --json）。

use crate::config::LogConfig;
use anyhow::Result;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// 初始化全局日志。返回的 WorkerGuard 必须在程序生命周期内存活，
/// 否则非阻塞 writer 会丢日志。调用方应持有到退出。
pub fn init(cfg: Option<&LogConfig>, console_only: bool) -> Result<Option<WorkerGuard>> {
    let cfg = match cfg {
        Some(c) => c,
        None => {
            // 没配置则只控制台
            let filter =
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(std::io::stderr))
                .init();
            return Ok(None);
        }
    };

    let level = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cfg.level.clone()));

    if console_only {
        tracing_subscriber::registry()
            .with(level)
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
        return Ok(None);
    }

    // 确保日志目录存在
    std::fs::create_dir_all(&cfg.dir)?;

    let file_appender = match cfg.rotation.as_str() {
        "hourly" => tracing_appender::rolling::hourly(&cfg.dir, "picfug.log"),
        "never" => tracing_appender::rolling::never(&cfg.dir, "picfug.log"),
        _ => tracing_appender::rolling::daily(&cfg.dir, "picfug.log"),
    };
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false); // 文件里不要 ANSI 颜色码

    let console_layer = fmt::layer().with_writer(std::io::stderr);

    tracing_subscriber::registry()
        .with(level)
        .with(file_layer)
        .with(console_layer)
        .init();

    Ok(Some(guard))
}
