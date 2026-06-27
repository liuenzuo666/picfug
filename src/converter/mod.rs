// 转换编排：解码 → 缩放 → 编码压缩 → 决定输出路径
//
// 这是核心业务逻辑的入口。输入一个源文件 + 一条规则，产出待写入字节 + 目标路径。
// 写盘与"存在即跳过"的判断放在 worker 层（需要查 DB / 文件系统），这里只负责纯计算。

pub mod compress;
pub mod format;
pub mod namer;
pub mod resize;

use crate::error::PicfugError;
use crate::model::{OutputFormat, Rule};
use namer::NamingInput;
use std::path::Path;

/// 编码结果（不含写盘逻辑）。
pub struct EncodedOutput {
    pub output: std::path::PathBuf,
    pub bytes: Vec<u8>,
    pub quality: u8,
    pub within_limit: bool,
}

/// 决定源文件的目标输出格式。
/// - 规则指定了 format → 用它
/// - 否则按源扩展名推断；无法推断时报错
pub fn resolve_format(source: &Path, rule: &Rule) -> Result<OutputFormat, PicfugError> {
    if let Some(f) = rule.format {
        return Ok(f);
    }
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => Ok(OutputFormat::Jpeg),
        "png" => Ok(OutputFormat::Png),
        "webp" => Ok(OutputFormat::Webp),
        other => Err(PicfugError::UnsupportedFormat(other.to_string())),
    }
}

/// 是否为可处理的图片扩展名（粗筛，避免对非图片文件尝试解码）。
pub fn is_supported_source_ext(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tiff" | "tif"
    )
}

/// 编排一次完整的"源+规则"转换（纯计算，不写盘）。
///
/// 步骤：
/// 1. 解析目标格式（决定命名后缀与编码方式）
/// 2. 解码源图
/// 3. 应用缩放
/// 4. 编码（可能迭代压缩达标）
/// 5. 用实际质量生成输出路径
pub fn convert(source: &Path, rule: &Rule) -> Result<EncodedOutput, PicfugError> {
    let format = resolve_format(source, rule)?;

    let img = image::open(source).map_err(|e| PicfugError::ImageDecode {
        path: source.to_path_buf(),
        source: e,
    })?;

    let resized = match rule.resize {
        Some(spec) => resize::apply(&img, spec),
        None => img,
    };

    let encoded = compress::encode(&resized, format, rule.size_limit, rule.jpeg_bg.as_deref())?;

    let output = namer::output_path(&NamingInput {
        source,
        resize: rule.resize,
        quality: encoded.quality,
        format: rule.format,
    });

    Ok(EncodedOutput {
        output,
        bytes: encoded.bytes,
        quality: encoded.quality,
        within_limit: encoded.within_limit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fit, OutputFormat, ResizeSpec, Rule, SizeLimit};
    use image::{DynamicImage, RgbaImage};
    use std::path::PathBuf;

    fn write_src(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(400, 300, image::Rgba([200, 100, 50, 255])));
        img.save(&p).unwrap();
        p
    }

    #[test]
    fn resolve_keeps_source_format_when_none() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_src(dir.path(), "a.png");
        let rule = Rule {
            name: "r".into(),
            resize: None,
            format: None,
            size_limit: None,
            jpeg_bg: None,
        };
        assert_eq!(resolve_format(&src, &rule).unwrap(), OutputFormat::Png);
    }

    #[test]
    fn resolve_uses_rule_format() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_src(dir.path(), "a.png");
        let rule = Rule {
            name: "r".into(),
            resize: None,
            format: Some(OutputFormat::Webp),
            size_limit: None,
            jpeg_bg: None,
        };
        assert_eq!(resolve_format(&src, &rule).unwrap(), OutputFormat::Webp);
    }

    #[test]
    fn resolve_unsupported_format_errors() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, "x").unwrap();
        let rule = Rule {
            name: "r".into(),
            resize: None,
            format: None,
            size_limit: None,
            jpeg_bg: None,
        };
        assert!(resolve_format(&src, &rule).is_err());
    }

    #[test]
    fn full_pipeline_produces_self_identifying_output() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_src(dir.path(), "photo.png");
        let rule = Rule {
            name: "thumb".into(),
            resize: Some(ResizeSpec {
                max_width: 100,
                max_height: 100,
                fit: Fit::Inside,
                no_upscale: false,
            }),
            format: None, // 沿用 png
            size_limit: None,
            jpeg_bg: None,
        };
        let out = convert(&src, &rule).unwrap();
        // 输出名可被自身识别（防套娃核心不变量）
        assert!(namer::is_output_file(&out.output));
        assert_eq!(out.output.extension().unwrap(), "png");
        // 应已缩放
        let decoded = image::load_from_memory(&out.bytes).unwrap();
        assert!(decoded.width() <= 100 && decoded.height() <= 100);
    }

    #[test]
    fn full_pipeline_jpeg_with_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_src(dir.path(), "photo.jpg");
        let rule = Rule {
            name: "hd".into(),
            resize: Some(ResizeSpec {
                max_width: 200,
                max_height: 200,
                fit: Fit::Inside,
                no_upscale: false,
            }),
            format: Some(OutputFormat::Jpeg),
            size_limit: Some(SizeLimit {
                max_bytes: 1_000_000,
                max_quality: 85,
                min_quality: 60,
                step: 5,
            }),
            jpeg_bg: None,
        };
        let out = convert(&src, &rule).unwrap();
        assert!(out.output.to_string_lossy().ends_with(".jpg"));
        assert!(namer::is_output_file(&out.output));
        // 大上限应直接达标
        assert!(out.within_limit);
    }
}
