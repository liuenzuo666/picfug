// 核心数据模型
//
// 这一层是纯数据结构 + 与配置/DB 的转换，不含业务逻辑。
// Rule / Directory / Status / FileRecord 都是领域概念。

use std::path::PathBuf;
use std::time::SystemTime;

/// 单条转换规则。对应 YAML 里 directories[].rules[] 的一个元素。
#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub resize: Option<ResizeSpec>,
    /// None 表示沿用源格式
    pub format: Option<OutputFormat>,
    pub size_limit: Option<SizeLimit>,
    /// 透明转 jpeg/不透明格式时的背景填充色，如 "#ffffff"
    pub jpeg_bg: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ResizeSpec {
    pub max_width: u32,
    pub max_height: u32,
    pub fit: Fit,
    /// true 则图像小于目标尺寸时不放大
    pub no_upscale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// 保持比例，整体放入框内（不裁剪），常用缩略图
    Inside,
    /// 保持比例，填满框（可能裁掉超出部分）
    Cover,
    /// 强制拉伸到精确尺寸（不保持比例）
    Exact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Jpeg,
    Png,
    Webp,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
            OutputFormat::Webp => "webp",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SizeLimit {
    /// 输出文件大小硬上限（字节）。迭代压缩目标。
    pub max_bytes: u64,
    /// 起始质量（迭代起点）
    pub max_quality: u8,
    /// 质量下限，低于则停止迭代
    pub min_quality: u8,
    /// 每次迭代降低的质量步长
    pub step: u8,
}

/// 一个被监听的目录及其规则集合。
#[derive(Debug, Clone)]
pub struct Directory {
    pub path: PathBuf,
    /// 是否递归处理子目录
    pub recursive: bool,
    pub rules: Vec<Rule>,
}

/// 文件处理状态（持久化在 SQLite files.status 列）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 待处理（已入队但尚未完成）
    Pending,
    /// 已完成（输出文件已生成）
    Done,
    /// 失败（error_msg 记录原因）
    Failed,
    /// 跳过（输出已存在，当处理过）
    Skipped,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Skipped => "skipped",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Status::Pending),
            "done" => Some(Status::Done),
            "failed" => Some(Status::Failed),
            "skipped" => Some(Status::Skipped),
            _ => None,
        }
    }
}

/// files 表的一行记录。
#[derive(Debug, Clone)]
pub struct FileRecord {
    pub id: Option<i64>,
    pub source_path: PathBuf,
    pub source_mtime: i64,
    pub source_size: i64,
    pub rule_name: String,
    pub output_path: PathBuf,
    pub status: Status,
    pub error_msg: Option<String>,
    pub attempt_count: i64,
    pub output_size: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 把 SystemTime 转为 unix 秒（统一时间戳格式）。
pub fn to_unix_secs(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
