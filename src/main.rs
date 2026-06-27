// picfug CLI 入口
// 核心逻辑在 picfug 库（lib.rs）；这里只做命令分发。

use clap::Parser;
use picfug::cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Check(args) => picfug::cli::run_check(args).await,
        Command::Once(args) => picfug::cli::run_once(args).await,
        Command::Start(args) => picfug::cli::run_start(args).await,
        Command::Status(args) => picfug::cli::run_status(args).await,
        Command::List(args) => picfug::cli::run_list(args).await,
        Command::Retry(args) => picfug::cli::run_retry(args).await,
        Command::Logs(args) => picfug::cli::run_logs(args).await,
    }
}
