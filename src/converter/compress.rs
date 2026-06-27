// 编码 + 迭代压缩
//
// 各格式行为（受 image crate 0.25 能力限制）：
// - Jpeg：有 quality 参数，可迭代降质逼近 size_limit 硬上限。这是唯一能"达标"的格式。
// - WebP：image crate 的 WebPEncoder 仅支持 lossless（无 quality 参数）。因此 WebP
//   与 PNG/BMP/TIFF 一样只能编码一次，"尽力而为"，无法保证达标。这是底层库的客观限制，
//   非缺陷——若必须控体积且需迭代，请使用 jpeg。
//
// 返回 (字节数据, 实际使用质量, 是否达标)。"实际使用质量"用于文件名后缀。

use crate::converter::format as fmt;
use crate::error::PicfugError;
use crate::model::{OutputFormat, SizeLimit};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{DynamicImage, ImageEncoder};
use std::io::Cursor;

/// 迭代压缩结果。
pub struct Encoded {
    pub bytes: Vec<u8>,
    /// 最终编码使用的质量（无损格式为 100）
    pub quality: u8,
    /// 是否达成 max_bytes 目标
    pub within_limit: bool,
}

/// 编码图像到字节。format 决定编码方式；size_limit 决定是否迭代。
pub fn encode(
    img: &DynamicImage,
    format: OutputFormat,
    limit: Option<SizeLimit>,
    bg: Option<&str>,
) -> Result<Encoded, PicfugError> {
    // 透明通道转 jpeg 需要合成背景（webp/png 保持 alpha）
    let img = fmt::compose_background(img, format, bg);

    match format {
        OutputFormat::Jpeg => encode_jpeg(img, limit),
        OutputFormat::Webp | OutputFormat::Png => encode_lossless(img, format, limit),
    }
}

/// JPEG：迭代降质逼近硬上限。
fn encode_jpeg(img: DynamicImage, limit: Option<SizeLimit>) -> Result<Encoded, PicfugError> {
    let Some(sl) = limit else {
        // 无 size_limit：用默认质量 85 编码一次
        let bytes = write_jpeg(&img, 85)?;
        return Ok(Encoded {
            bytes,
            quality: 85,
            within_limit: true,
        });
    };

    let mut quality = sl.max_quality;
    loop {
        let bytes = write_jpeg(&img, quality)?;
        if (bytes.len() as u64) <= sl.max_bytes {
            return Ok(Encoded {
                bytes,
                quality,
                within_limit: true,
            });
        }
        // 未达标，尝试降质
        if quality <= sl.min_quality {
            return Ok(Encoded {
                bytes,
                quality,
                within_limit: false,
            });
        }
        quality = quality.saturating_sub(sl.step);
        if quality < sl.min_quality {
            quality = sl.min_quality;
        }
    }
}

/// 无损格式（WebP lossless / PNG）：只编码一次，尽力而为。
fn encode_lossless(
    img: DynamicImage,
    format: OutputFormat,
    limit: Option<SizeLimit>,
) -> Result<Encoded, PicfugError> {
    let bytes = match format {
        OutputFormat::Png => write_png(&img)?,
        OutputFormat::Webp => write_webp(&img)?,
        _ => unreachable!(),
    };
    let within = match limit {
        Some(sl) => bytes.len() as u64 <= sl.max_bytes,
        None => true,
    };
    Ok(Encoded {
        bytes,
        quality: 100,
        within_limit: within,
    })
}

fn write_jpeg(img: &DynamicImage, q: u8) -> Result<Vec<u8>, PicfugError> {
    let mut buf = Cursor::new(Vec::new());
    let enc = JpegEncoder::new_with_quality(&mut buf, q);
    enc.write_image(img.as_bytes(), img.width(), img.height(), img.color().into())
        .map_err(|source| PicfugError::ImageEncode { source })?;
    Ok(buf.into_inner())
}

fn write_png(img: &DynamicImage) -> Result<Vec<u8>, PicfugError> {
    let mut buf = Cursor::new(Vec::new());
    let enc = PngEncoder::new(&mut buf);
    enc.write_image(img.as_bytes(), img.width(), img.height(), img.color().into())
        .map_err(|source| PicfugError::ImageEncode { source })?;
    Ok(buf.into_inner())
}

fn write_webp(img: &DynamicImage) -> Result<Vec<u8>, PicfugError> {
    let mut buf = Cursor::new(Vec::new());
    let enc = WebPEncoder::new_lossless(&mut buf);
    enc.write_image(img.as_bytes(), img.width(), img.height(), img.color().into())
        .map_err(|source| PicfugError::ImageEncode { source })?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OutputFormat;
    use image::{DynamicImage, RgbImage, RgbaImage};

    fn solid_rgba(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255])))
    }

    fn solid_rgb(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, image::Rgb([255, 0, 0])))
    }

    fn sl(max: u64, max_q: u8, min_q: u8, step: u8) -> SizeLimit {
        SizeLimit {
            max_bytes: max,
            max_quality: max_q,
            min_quality: min_q,
            step,
        }
    }

    #[test]
    fn jpeg_quality_lower_smaller() {
        let img = solid_rgba(200, 200);
        let high = encode(&img, OutputFormat::Jpeg, Some(sl(100000, 90, 10, 10)), None).unwrap();
        let low = encode(&img, OutputFormat::Jpeg, Some(sl(100000, 20, 10, 10)), None).unwrap();
        assert!(low.bytes.len() < high.bytes.len(), "低质量应更小");
    }

    #[test]
    fn jpeg_no_limit_uses_default() {
        let img = solid_rgba(50, 50);
        let e = encode(&img, OutputFormat::Jpeg, None, None).unwrap();
        assert_eq!(e.quality, 85);
        assert!(e.within_limit);
    }

    #[test]
    fn iterates_until_under_limit() {
        // 极小上限，迫使迭代降质
        let img = solid_rgba(500, 500);
        let e = encode(&img, OutputFormat::Jpeg, Some(sl(1, 90, 10, 5)), None).unwrap();
        assert!(e.quality < 90, "应已迭代降质");
        // 上限为 1 字节几乎不可能达到，应报告未达标
        assert!(!e.within_limit);
    }

    #[test]
    fn jpeg_bg_compose() {
        // 透明图转 jpeg，应合成背景（不报错、产出有效 jpeg）
        let img =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(20, 20, image::Rgba([0, 0, 0, 0])));
        let e = encode(&img, OutputFormat::Jpeg, None, Some("#ffffff")).unwrap();
        assert!(!e.bytes.is_empty());
    }

    #[test]
    fn png_lossless_best_effort() {
        let img = solid_rgba(100, 100);
        // 极小上限，PNG 无损不可能达标
        let e = encode(&img, OutputFormat::Png, Some(sl(1, 90, 10, 5)), None).unwrap();
        assert_eq!(e.quality, 100);
        assert!(!e.within_limit);
    }

    #[test]
    fn webp_lossless_best_effort() {
        let img = solid_rgba(100, 100);
        let e = encode(&img, OutputFormat::Webp, Some(sl(1, 90, 10, 5)), None).unwrap();
        assert_eq!(e.quality, 100);
        assert!(!e.within_limit);
    }

    #[test]
    fn png_no_limit_ok() {
        let img = solid_rgb(50, 50);
        let e = encode(&img, OutputFormat::Png, None, None).unwrap();
        assert!(e.within_limit);
        assert!(!e.bytes.is_empty());
    }

    #[test]
    fn jpeg_meets_generous_limit() {
        let img = solid_rgba(100, 100);
        let e = encode(&img, OutputFormat::Jpeg, Some(sl(1_000_000, 90, 70, 5)), None).unwrap();
        assert!(e.within_limit);
        assert_eq!(e.quality, 90); // 一次就达标，不降质
    }
}
