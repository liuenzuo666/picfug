// engine 模块入口：扫描 / 监听 / 轮询 / 调度

pub mod dispatcher;
pub mod poller;
pub mod scanner;
pub mod watcher;

pub use dispatcher::{classify, Decision, Task};
