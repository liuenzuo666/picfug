// 输出文件命名 + 防套娃识别
//
// 核心安全机制：系统产生的输出文件名遵循固定结构，dispatcher 通过文件名正则识别
// 这类文件并跳过，从而切断"输出再被当源处理"的链路。
//
// 命名规范（确定性函数）：
//   {源stem}_{宽}x{高}_q{质量}.{ext}
//   - 宽/高为 0 表示该维度未限制（按 ResizeSpec 计算）
//   - 质量为实际编码所用质量（无损格式或未压缩记为 100）
//   - ext 由输出格式决定；未指定 format 时沿用源扩展名
//
// 识别规范（固定正则，纯字符串）：
//   ^.+_\d+x\d+_q\d+\.[A-Za-z0-9]+$
// 命中即判定为"系统输出文件"，跳过。该正则对常见源文件名（photo.jpg、IMG_2024.png、
// 截图.png 等）不会误判。

use crate::model::{OutputFormat, ResizeSpec, SizeLimit};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};

static OUTPUT_NAME_RE: Lazy<Regex> = Lazy::new(|| {
    // 匹配整个文件名：任意前缀 + _宽x高 + _q质量 + .扩展名
    Regex::new(r"^.+_\d+x\d+_q\d+\.[A-Za-z0-9]+$").expect("output name regex")
});

/// 判断某个文件名是否"长得像"系统输出的文件名。
///
/// 仅基于文件名（不含目录），命中即跳过。这是防套娃的主防线。
/// 注意：这是基于命名约定的识别，若用户手动放入一个恰好符合该命名的真实源文件，
/// 会被误判跳过——这是可接受的取舍，重命名即可恢复。
pub fn is_output_file_name(name: &str) -> bool {
    OUTPUT_NAME_RE.is_match(name)
}

/// 判断某个路径是否为系统输出文件（取文件名部分判断）。
pub fn is_output_file(path: &Path) -> bool {
    match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => is_output_file_name(name),
        None => false,
    }
}

/// 命名所需的所有参数。`Rule` 调用方负责填充。
pub struct NamingInput<'a> {
    /// 源文件路径
    pub source: &'a Path,
    /// 规则里声明的目标尺寸（用于后缀显示），None 表示无缩放维度
    pub resize: Option<ResizeSpec>,
    /// 最终编码使用的质量（迭代后的实际值；无损格式记 100）
    pub quality: u8,
    /// 目标格式，None 表示沿用源扩展名
    pub format: Option<OutputFormat>,
}

/// 计算输出文件路径（与源同目录）。
///
/// 注意：调用此函数前请确保传入的 quality 已确定。命名是确定性的——
/// 相同输入永远产生相同输出路径，因此 is_output_file 对它的识别也永远成立。
pub fn output_path(input: &NamingInput) -> PathBuf {
    let stem = input
        .source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let ext = output_extension(input.source, input.format);

    let (w, h) = match input.resize {
        Some(r) => (r.max_width, r.max_height),
        None => (0, 0),
    };

    let new_name = format!("{stem}_{w}x{h}_q{}.{ext}", input.quality);
    input.source.with_file_name(new_name)
}

/// 决定输出扩展名：指定 format 用 format 的；否则沿用源扩展名（小写）。
pub fn output_extension(source: &Path, format: Option<OutputFormat>) -> String {
    match format {
        Some(f) => f.extension().to_string(),
        None => source
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_else(|| "bin".to_string()),
    }
}

/// 根据格式与 size_limit 决定迭代起始质量。
pub fn initial_quality(format: Option<OutputFormat>, limit: Option<SizeLimit>) -> u8 {
    match (format, limit) {
        (Some(OutputFormat::Jpeg), Some(sl)) => sl.max_quality,
        (Some(OutputFormat::Webp), Some(sl)) => sl.max_quality,
        // 无损格式或无限制：记 100（仅作命名标识，不参与迭代）
        _ => 100,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fit, OutputFormat, ResizeSpec, SizeLimit};
    use std::path::Path;

    fn rs(w: u32, h: u32) -> ResizeSpec {
        ResizeSpec {
            max_width: w,
            max_height: h,
            fit: Fit::Inside,
            no_upscale: false,
        }
    }

    // ===== 命名 =====

    #[test]
    fn names_basic() {
        let p = output_path(&NamingInput {
            source: Path::new("/a/photo.jpg"),
            resize: Some(rs(1920, 1080)),
            quality: 85,
            format: Some(OutputFormat::Webp),
        });
        assert_eq!(p.file_name().unwrap(), "photo_1920x1080_q85.webp");
    }

    #[test]
    fn names_keep_source_ext_when_no_format() {
        let p = output_path(&NamingInput {
            source: Path::new("/a/photo.png"),
            resize: Some(rs(300, 300)),
            quality: 100,
            format: None,
        });
        assert_eq!(p.file_name().unwrap(), "photo_300x300_q100.png");
    }

    #[test]
    fn names_no_resize_zero_dims() {
        let p = output_path(&NamingInput {
            source: Path::new("/a/big.tiff"),
            resize: None,
            quality: 100,
            format: None,
        });
        assert_eq!(p.file_name().unwrap(), "big_0x0_q100.tiff");
    }

    #[test]
    fn names_zero_height_when_only_width() {
        // ResizeSpec 里 max_height=0 表示不限高度，后缀如实记录
        let p = output_path(&NamingInput {
            source: Path::new("/a/x.jpg"),
            resize: Some(rs(1920, 0)),
            quality: 80,
            format: Some(OutputFormat::Jpeg),
        });
        assert_eq!(p.file_name().unwrap(), "x_1920x0_q80.jpg");
    }

    #[test]
    fn names_stays_in_same_directory() {
        let p = output_path(&NamingInput {
            source: Path::new("/deep/nested/dir/a.jpg"),
            resize: Some(rs(10, 10)),
            quality: 50,
            format: None,
        });
        assert_eq!(p, Path::new("/deep/nested/dir/a_10x10_q50.jpg"));
    }

    // ===== 防套娃识别：正例 =====

    #[test]
    fn detects_real_outputs() {
        for name in [
            "photo_1920x1080_q85.webp",
            "a_300x300_q100.png",
            "big_0x0_q100.tiff",
            "IMG_001_1920x1080_q78.jpg",
            "x_1x1_q1.j",
            "deep_under-scores_50x50_q90.jpeg",
        ] {
            assert!(is_output_file_name(name), "应识别为输出: {name}");
        }
    }

    // ===== 防套娃识别：反例（真实源文件绝不能被误判）=====

    #[test]
    fn does_not_flag_normal_sources() {
        for name in [
            "photo.jpg",
            "photo.png",
            "IMG_20240101.png",
            "截图.png",
            "image_001.png",         // 只有 _数字
            "icon_64.png",            // 只有 _数字，无 x
            "thumb_64x64.png",        // 缺 _q
            "pic_1920x1080.png",      // 缺 _q数字
            "a_q85.jpg",              // 缺尺寸
            "no_ext",                 // 无扩展名
            ".hidden",                // 仅扩展名
            "DSC0001.JPG",            // 普通大写扩展名，无后缀模式
            "a_1920x1080_q85",        // 缺扩展名
            "a_1920x1080_q85.",       // 空扩展名
        ] {
            assert!(!is_output_file_name(name), "不应误判为输出: {name}");
        }
    }

    #[test]
    fn detect_by_path() {
        assert!(is_output_file(Path::new("/x/y/a_10x10_q50.jpg")));
        assert!(!is_output_file(Path::new("/x/y/photo.jpg")));
    }

    #[test]
    fn names_are_deterministic_and_self_identifying() {
        // 关键不变量：output_path 的产物一定能被 is_output_file 识别
        for (w, h, q, f) in [
            (1920u32, 1080u32, 85u8, Some(OutputFormat::Webp)),
            (300, 300, 100, None),
            (0, 0, 100, None),
            (1, 1, 1, Some(OutputFormat::Jpeg)),
        ] {
            let p = output_path(&NamingInput {
                source: Path::new("/a/photo.jpg"),
                resize: (w != 0 || h != 0).then(|| rs(w, h)),
                quality: q,
                format: f,
            });
            assert!(
                is_output_file(&p),
                "输出文件未被自身识别: {}",
                p.display()
            );
        }
    }
}
