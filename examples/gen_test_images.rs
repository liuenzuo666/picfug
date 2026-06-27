// 辅助：生成一组测试图片到指定目录，用于手工验证 CLI。
// 用法: cargo run --example gen_test_images -- <输出目录>
use image::{ColorType, RgbImage};
use std::path::PathBuf;

fn main() {
    let dir: PathBuf = std::env::args().nth(1).expect("需要输出目录参数").into();
    std::fs::create_dir_all(&dir).unwrap();

    // 彩色 jpeg，大尺寸便于测试压缩
    let img = RgbImage::from_fn(800, 600, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
    });
    image::save_buffer(dir.join("rainbow.jpg"), &img, 800, 600, ColorType::Rgb8).unwrap();

    // 纯色 png
    let img2 = image::RgbaImage::from_pixel(400, 400, image::Rgba([255, 100, 50, 255]));
    image::save_buffer(dir.join("solid.png"), &img2, 400, 400, ColorType::Rgba8).unwrap();

    // 带透明通道的 png（测试 jpeg_bg 合成）
    let img3 = image::RgbaImage::from_pixel(200, 200, image::Rgba([0, 0, 0, 0]));
    image::save_buffer(dir.join("transparent.png"), &img3, 200, 200, ColorType::Rgba8).unwrap();

    // 嵌套子目录里的图
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    image::save_buffer(dir.join("sub/nested.jpg"), &img, 800, 600, ColorType::Rgb8).unwrap();

    println!("已生成测试图片到 {}", dir.display());
}
