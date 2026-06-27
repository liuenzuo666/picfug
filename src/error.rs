// 统一错误类型
//
// 业务层使用 PicfugError（thiserror 派生），便于精确分类；
// 命令入口用 anyhow::Result 统一收敛 + 友好打印。

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum PicfugError {
    #[error("配置文件不存在: {0}")]
    ConfigNotFound(PathBuf),

    #[error("配置解析失败 ({path}): {source}")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("配置校验失败: {0}")]
    ConfigInvalid(String),

    #[error("数据库错误: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("数据库初始化失败 ({path}): {source}")]
    DatabaseInit {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error("图像解码失败 ({path}): {source}")]
    ImageDecode {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    #[error("图像编码失败: {source}")]
    ImageEncode {
        #[source]
        source: image::ImageError,
    },

    #[error("IO 错误 ({path}): {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("不支持的图像格式: {0}")]
    UnsupportedFormat(String),

    #[error("文件系统监听错误: {0}")]
    Watcher(#[from] notify::Error),
}

/// 转换阶段产生的"失败原因"，会被持久化到 files.error_msg，供 list --status failed 定位。
/// 这里保留字符串形式：原因来源多样（解码失败、磁盘满、迭代压缩到下限仍超限...），
/// 用 String 既足够人读，也方便 CLI 直接展示与 JSON 序列化。
pub type ConversionFailure = String;
