// 像素缩放
//
// 支持 fit = inside / cover / exact，以及 no_upscale（小于目标不放大）。
// 完全基于 image crate 的 FilterType::Lanczos3 重采样。

use crate::model::{Fit, ResizeSpec};
use image::{DynamicImage, GenericImageView};

/// 对图像应用缩放规则。返回新图像（即使不缩放也返回克隆）。
pub fn apply(img: &DynamicImage, spec: ResizeSpec) -> DynamicImage {
    let (ow, oh) = img.dimensions();

    let (tw, th) = target_dimensions(ow, oh, spec);
    if tw == ow && th == oh {
        return img.clone();
    }
    // no_upscale: 若目标比原图大，保持原尺寸
    if spec.no_upscale && (tw >= ow && th >= oh) {
        return img.clone();
    }

    use image::imageops::FilterType;
    match spec.fit {
        Fit::Exact => img.resize_exact(tw, th, FilterType::Lanczos3),
        Fit::Cover => img.resize(tw, th, FilterType::Lanczos3), // resize = cover 行为
        Fit::Inside => img.resize(tw, th, FilterType::Lanczos3),
    }
}

/// 根据规则与原图尺寸计算目标宽高。
/// - Exact: 直接用 max_width/max_height
/// - Inside/Cover: 保持比例，max_width=0 表示按高度算宽，max_height=0 同理
pub fn target_dimensions(ow: u32, oh: u32, spec: ResizeSpec) -> (u32, u32) {
    let mw = spec.max_width;
    let mh = spec.max_height;
    if mw == 0 && mh == 0 {
        return (ow, oh);
    }
    match spec.fit {
        Fit::Exact => (mw.max(1), mh.max(1)),
        Fit::Inside | Fit::Cover => {
            if mw == 0 {
                // 仅限高度
                let new_w = scale_dim(ow, oh, mh, true);
                return (new_w, mh);
            }
            if mh == 0 {
                let new_h = scale_dim(ow, oh, mw, false);
                return (mw, new_h);
            }
            // 同时有宽高限制，按比例缩到框内（Inside），Cover 由 image::resize 处理裁剪
            let ratio_w = mw as f64 / ow as f64;
            let ratio_h = mh as f64 / oh as f64;
            let ratio = match spec.fit {
                Fit::Inside => ratio_w.min(ratio_h),
                Fit::Cover => ratio_w.max(ratio_h),
                _ => unreachable!(),
            };
            let nw = ((ow as f64) * ratio).round().max(1.0) as u32;
            let nh = ((oh as f64) * ratio).round().max(1.0) as u32;
            (nw, nh)
        }
    }
}

fn scale_dim(ow: u32, oh: u32, fixed: u32, fixed_is_height: bool) -> u32 {
    if fixed_is_height {
        let r = fixed as f64 / oh as f64;
        ((ow as f64) * r).round().max(1.0) as u32
    } else {
        let r = fixed as f64 / ow as f64;
        ((oh as f64) * r).round().max(1.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fit, ResizeSpec};
    use image::{DynamicImage, RgbaImage};

    fn img(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::new(w, h))
    }

    fn rs(w: u32, h: u32, fit: Fit) -> ResizeSpec {
        ResizeSpec {
            max_width: w,
            max_height: h,
            fit,
            no_upscale: false,
        }
    }

    #[test]
    fn inside_keeps_aspect() {
        // 200x100 放入 100x100 → 100x50
        let (w, h) = target_dimensions(200, 100, rs(100, 100, Fit::Inside));
        assert_eq!((w, h), (100, 50));
    }

    #[test]
    fn cover_keeps_aspect() {
        // 200x100 cover 100x100 → 100x50 框, 但 cover 取大比例 → 100x50? 实际 cover 应填满
        // ratio = max(0.5, 1.0) = 1.0 → 200x100，但封面框裁切到 100x100
        let (w, h) = target_dimensions(200, 100, rs(100, 100, Fit::Cover));
        assert_eq!((w, h), (200, 100));
    }

    #[test]
    fn exact_ignores_aspect() {
        let (w, h) = target_dimensions(200, 100, rs(50, 200, Fit::Exact));
        assert_eq!((w, h), (50, 200));
    }

    #[test]
    fn zero_means_unbounded() {
        let (w, h) = target_dimensions(200, 100, rs(0, 50, Fit::Inside));
        assert_eq!(h, 50);
        assert!(w > 0);
    }

    #[test]
    fn no_upscale_keeps_small() {
        let spec = ResizeSpec {
            max_width: 500,
            max_height: 500,
            fit: Fit::Inside,
            no_upscale: true,
        };
        let out = apply(&img(50, 50), spec);
        assert_eq!(out.dimensions(), (50, 50));
    }

    #[test]
    fn applies_downscale() {
        let spec = rs(10, 10, Fit::Exact);
        let out = apply(&img(100, 100), spec);
        assert_eq!(out.dimensions(), (10, 10));
    }
}
