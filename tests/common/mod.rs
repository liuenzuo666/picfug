// 测试公共辅助：构造目录、规则、源图片，列举输出文件等。
//
// 所有防套娃/集成测试共享这些工具，保证测试本身简洁。

use image::{DynamicImage, Rgba, RgbaImage};
use picfug::db::Db;
use picfug::model::{Directory, Fit, ResizeSpec, Rule, SizeLimit};
use std::path::{Path, PathBuf};

/// 构造单规则环境，返回 (临时目录, 目录配置, 源文件路径)。
/// 用内存 DB（外层自行 open_memory）。
pub fn setup_single_rule(w: u32, h: u32) -> (tempfile::TempDir, Vec<Directory>, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("photo.png");
    write_solid_png(&src, 400, 300, Rgba([100, 150, 200, 255]));
    let dirs = vec![Directory {
        path: t.path().to_path_buf(),
        recursive: true,
        rules: vec![make_rule("r", w, h, None)],
    }];
    (t, dirs, src)
}

/// 带 DB 的单规则环境。
pub fn setup_single_rule_with_db(w: u32, h: u32) -> (tempfile::TempDir, Vec<Directory>, Db, PathBuf) {
    let (t, dirs, src) = setup_single_rule(w, h);
    let db = Db::open_memory().unwrap();
    (t, dirs, db, src)
}

/// 两条不同规则。
pub fn setup_two_rules_with_db() -> (tempfile::TempDir, Vec<Directory>, Db, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("photo.png");
    write_solid_png(&src, 400, 300, Rgba([100, 150, 200, 255]));
    let dirs = vec![Directory {
        path: t.path().to_path_buf(),
        recursive: true,
        rules: vec![
            make_rule("thumb", 50, 50, None),
            make_rule("hd", 200, 200, None),
        ],
    }];
    (t, dirs, Db::open_memory().unwrap(), src)
}

/// 嵌套子目录环境。
pub fn setup_nested_with_db() -> (tempfile::TempDir, Vec<Directory>, Db) {
    let t = tempfile::tempdir().unwrap();
    write_solid_png(&t.path().join("top.png"), 300, 300, Rgba([10, 20, 30, 255]));
    std::fs::create_dir_all(t.path().join("sub")).unwrap();
    write_solid_png(
        &t.path().join("sub/deep.png"),
        300,
        300,
        Rgba([40, 50, 60, 255]),
    );
    let dirs = vec![Directory {
        path: t.path().to_path_buf(),
        recursive: true,
        rules: vec![make_rule("r", 100, 100, None)],
    }];
    (t, dirs, Db::open_memory().unwrap())
}

fn make_rule(name: &str, w: u32, h: u32, limit_max_bytes: Option<u64>) -> Rule {
    Rule {
        name: name.into(),
        resize: Some(ResizeSpec {
            max_width: w,
            max_height: h,
            fit: Fit::Inside,
            no_upscale: false,
        }),
        format: None,
        size_limit: limit_max_bytes.map(|mb| SizeLimit {
            max_bytes: mb,
            max_quality: 85,
            min_quality: 60,
            step: 5,
        }),
        jpeg_bg: None,
    }
}

/// 写一张纯色 PNG。
pub fn write_solid_png(path: &Path, w: u32, h: u32, color: Rgba<u8>) {
    let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, color));
    img.save(path).unwrap();
}

/// 列出某目录下"长得像系统输出"的文件（用 is_output_file 判定）。
pub fn list_outputs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if picfug::converter::namer::is_output_file_name(name) {
                    out.push(e.path());
                }
            }
        }
    }
    out
}

/// 列出某目录下所有文件（含子目录第一层）。
pub fn list_all_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_files(dir, &mut out);
    out
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                collect_files(&path, out);
            } else {
                out.push(path);
            }
        }
    }
}
