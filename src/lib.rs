// picfug 库入口：暴露核心模块供集成测试与外部使用。
// main.rs 只负责 CLI 解析与命令分发。

pub mod cli;
pub mod config;
pub mod converter;
pub mod db;
pub mod engine;
pub mod error;
pub mod log;
pub mod model;
pub mod worker;
