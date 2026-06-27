// 定时轮询兜底
//
// 守护进程下，每隔 poll_interval 重新扫描全部目录一次。作用：
// - 兜住 watcher 漏掉的事件（跨平台差异、大文件写入中途触发）
// - 对已有源重新跑 dispatcher：因输出已存在会被跳过，开销很小
//
// 轮询产出的候选经过 dispatcher 过滤后，仅"新文件/输出未存在"的才会真正转换。

use crate::model::Directory;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// 启动一个后台轮询任务，周期性把扫描到的文件列表发到 channel。
pub async fn spawn(
    directories: Vec<Directory>,
    interval: std::time::Duration,
    tx: mpsc::Sender<Vec<PathBuf>>,
) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        let mut all = Vec::new();
        for d in &directories {
            for p in crate::engine::scanner::scan_dir(d) {
                all.push(p);
            }
        }
        if !all.is_empty() {
            if tx.send(all).await.is_err() {
                break; // 接收端关闭，退出
            }
        }
    }
}
