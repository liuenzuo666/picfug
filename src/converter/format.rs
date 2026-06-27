// 格式处理：透明通道背景合成
//
// JPEG 不支持 alpha 通道。源图为带透明（RGBA/LA）时，转 JPEG 前 must 合成背景色，
// 否则透明区域会变黑。PNG/WebP 支持 alpha，保持原样。

use crate::error::PicfugError;
use crate::model::OutputFormat;
use image::DynamicImage;

/// 解析 "#rrggbb" / "#rgb" 颜色字符串为 (r,g,b)。
pub fn parse_hex_color(s: &str) -> Result<(u8, u8, u8), PicfugError> {
    let s = s.trim_start_matches('#');
    let parse = |hex: &str| {
        u8::from_str_radix(hex, 16)
            .map_err(|_| PicfugError::ConfigInvalid(format!("非法颜色值: #{hex}")))
    };
    match s.len() {
        6 => Ok((parse(&s[0..2])?, parse(&s[2..4])?, parse(&s[4..6])?)),
        3 => {
            let r = s.as_bytes()[0] as char;
            let g = s.as_bytes()[1] as char;
            let b = s.as_bytes()[2] as char;
            let expand = |c: char| {
                let hex = format!("{c}{c}");
                parse(&hex)
            };
            Ok((expand(r)?, expand(g)?, expand(b)?))
        }
        _ => Err(PicfugError::ConfigInvalid(format!("颜色值格式应为 #rrggbb 或 #rgb: {s}"))),
    }
}

/// 若目标格式不支持 alpha 且源图有 alpha，则用背景色合成并转为 RGB；否则原样返回。
///
/// 注意：合成后必须返回 RGB 类型（非 RGBA），否则 JPEG 编码会因不支持 alpha 而报错。
pub fn compose_background(img: &DynamicImage, format: OutputFormat, bg: Option<&str>) -> DynamicImage {
    let needs_compose = matches!(format, OutputFormat::Jpeg) && has_alpha(img);
    if !needs_compose {
        return img.clone();
    }
    let (br, bg_, bb) = bg
        .and_then(|s| parse_hex_color(s).ok())
        .unwrap_or((255, 255, 255)); // 默认白底
    flatten_alpha(img, br, bg_, bb)
}

fn has_alpha(img: &DynamicImage) -> bool {
    matches!(
        img.color(),
        image::ColorType::Rgba8
            | image::ColorType::Rgba16
            | image::ColorType::La8
            | image::ColorType::La16
    )
}

/// 将 alpha 合成到指定背景色上，返回 RGB 图像。
fn flatten_alpha(img: &DynamicImage, br: u8, bg: u8, bb: u8) -> DynamicImage {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut out = image::RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = rgba.get_pixel(x, y);
            let a = p[3] as f32 / 255.0;
            let r = (p[0] as f32 * a + br as f32 * (1.0 - a)).round() as u8;
            let g = (p[1] as f32 * a + bg as f32 * (1.0 - a)).round() as u8;
            let b = (p[2] as f32 * a + bb as f32 * (1.0 - a)).round() as u8;
            out.put_pixel(x, y, image::Rgb([r, g, b]));
        }
    }
    DynamicImage::ImageRgb8(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};

    #[test]
    fn parse_hex_colors() {
        assert_eq!(parse_hex_color("#ffffff").unwrap(), (255, 255, 255));
        assert_eq!(parse_hex_color("#000000").unwrap(), (0, 0, 0));
        assert_eq!(parse_hex_color("#fff").unwrap(), (255, 255, 255));
        assert_eq!(parse_hex_color("#f00").unwrap(), (255, 0, 0));
        assert!(parse_hex_color("#xyz").is_err());
        assert!(parse_hex_color("12345").is_err());
    }

    #[test]
    fn flatten_keeps_opaque() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(10, 10, Rgba([100, 50, 25, 255])));
        let out = compose_background(&img, OutputFormat::Jpeg, Some("#ffffff"));
        let rgb = out.to_rgb8();
        let p = rgb.get_pixel(0, 0);
        // 完全不透明，颜色不变
        assert_eq!(p.0, [100, 50, 25]);
    }

    #[test]
    fn flatten_transparent_to_bg() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(10, 10, Rgba([0, 0, 0, 0])));
        let out = compose_background(&img, OutputFormat::Jpeg, Some("#ff0000"));
        let rgb = out.to_rgb8();
        let p = rgb.get_pixel(0, 0);
        // 全透明 → 背景红
        assert_eq!(p.0, [255, 0, 0]);
    }

    #[test]
    fn flatten_semitransparent_blends() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(10, 10, Rgba([0, 0, 0, 128])));
        let out = compose_background(&img, OutputFormat::Jpeg, Some("#ffffff"));
        let rgb = out.to_rgb8();
        let p = rgb.get_pixel(0, 0);
        // alpha≈0.5，黑(0) + 白(255) → ~128
        assert!((p[0] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn png_keeps_alpha() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(10, 10, Rgba([0, 0, 0, 0])));
        let out = compose_background(&img, OutputFormat::Png, Some("#ffffff"));
        // PNG 不合成，保持透明
        assert_eq!(out.to_rgba8().get_pixel(0, 0).0, [0, 0, 0, 0]);
    }
}
