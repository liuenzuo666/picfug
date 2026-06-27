// 文件系统监听（notify + debounce）
//
// 监听 directories 下的文件变更事件，去抖后把"创建/修改"事件投递到 channel。
// 输出文件的事件也会到达这里——但 worker 在转换前会再过一次 dispatcher 的防套娃识别，
// 所以即使监听到也不会被处理。
//
// 重要：watcher 本身只负责"通知有变化"，不做任何业务判定。判定全部集中在 dispatcher，
// 保证防套娃逻辑只有单一实现点。
//
// 注意：new_debouncer 的返回类型依赖平台（Linux=Inotify+NoCache, macOS=FSEvent+FileIdMap...），
// 因此不写死类型别名，调用方用类型推导持有（let _guard = ...）。

use crate::model::Directory;
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// 启动监听。返回一个接收器，每次去抖后产出发生变化的文件路径列表。
/// 返回的 debouncer guard 必须保持存活，否则监听停止。
///
/// 注意：返回的 debouncer 类型依赖平台，故用 `impl` 让调用方按推导持有。
pub fn spawn(
    directories: &[Directory],
    debounce: Duration,
) -> anyhow::Result<(
    mpsc::Receiver<Vec<PathBuf>>,
    impl std::any::Any, // 平台相关的 Debouncer 类型，仅需保活
)> {
    let (tx, rx) = mpsc::channel::<Vec<PathBuf>>();

    let tx_clone = tx.clone();
    let mut debouncer = new_debouncer(debounce, None, move |res: DebounceEventResult| {
        if let Ok(events) = res {
            let paths: Vec<PathBuf> = events
                .iter()
                .filter(|e| {
                    use notify::EventKind;
                    matches!(
                        e.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Any
                    )
                })
                .flat_map(|e| e.paths.clone())
                .collect();
            if !paths.is_empty() {
                let _ = tx_clone.send(paths);
            }
        }
    })?;

    for d in directories {
        let mode = if d.recursive {
            notify::RecursiveMode::Recursive
        } else {
            notify::RecursiveMode::NonRecursive
        };
        debouncer.watch(&d.path, mode)?;
    }

    Ok((rx, debouncer))
}
