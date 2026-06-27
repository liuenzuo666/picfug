// Dispatcher：决定一个文件"该不该被转换"
//
// 这是防套娃的核心。所有来源（scanner/watcher/poller）产出的候选文件都先经过这里过滤。
//
// 判定逻辑（确定性、无并发问题）：
//   1. is_output_file(name) == true → 跳过（系统自己的输出，绝不二次处理）
//   2. 非图片扩展名 → 跳过
//   3. 输出已存在 → 跳过（当处理过）
//   4. 否则 → 需要转换
//
// 关键不变量：系统写入的输出文件名恒满足 is_output_file，因此第 1 步永远命中，
// 输出永远不会回流为输入。这条链在第一步就被切断。

use crate::converter;
use crate::model::{Directory, Rule};
use std::path::{Path, PathBuf};

/// 一个待转换的任务：源文件 + 所属目录 + 命中的规则。
/// 注意一个源文件可能命中多条规则 → 产生多个 Task。
#[derive(Debug, Clone)]
pub struct Task {
    pub source: PathBuf,
    pub dir_index: usize,
    pub rule_index: usize,
}

/// 判定决策。
#[derive(Debug)]
pub enum Decision {
    /// 需要转换，附命中规则下标
    Convert(Task),
    /// 跳过：是系统输出文件
    SkipOutput,
    /// 跳过：非图片
    SkipNotImage,
    /// 跳过：所有规则的输出都已存在
    SkipExists,
}

/// 对单个文件做判定。directories 提供 dir_index → (path, rules) 映射。
pub fn classify(
    file: &Path,
    directories: &[Directory],
) -> Vec<Decision> {
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // 防套娃第一道：系统输出文件直接跳过
    if converter::namer::is_output_file_name(name) {
        return vec![Decision::SkipOutput];
    }
    // 非图片跳过
    if !converter::is_supported_source_ext(file) {
        return vec![Decision::SkipNotImage];
    }

    let mut out = Vec::new();
    for (di, dir) in directories.iter().enumerate() {
        // 文件必须在该目录的管辖范围内（含递归子目录）
        if !is_under(&dir.path, file, dir.recursive) {
            continue;
        }
        for (ri, rule) in dir.rules.iter().enumerate() {
            match decide_one(file, rule) {
                RuleDecision::Convert => out.push(Decision::Convert(Task {
                    source: file.to_path_buf(),
                    dir_index: di,
                    rule_index: ri,
                })),
                RuleDecision::SkipExists => out.push(Decision::SkipExists),
            }
        }
    }
    // 若没有任何目录管辖此文件，返回空（调用方忽略）
    out
}

enum RuleDecision {
    Convert,
    SkipExists,
}

/// 针对单条规则：输出已存在则跳过，否则转换。
/// 注意：当前以"该源的任意输出是否存在"为判据（见 output_exists_for），
/// rule 参数保留用于未来按规则精确判定的扩展，故标记 _。
fn decide_one(source: &Path, _rule: &Rule) -> RuleDecision {
    // 用规则的最大可能质量来预判输出路径；存在即跳过。
    // 注意：实际质量由迭代决定，命名后缀里的质量是迭代后的真实值。
    // 因此"存在即跳过"用"任何质量"的输出是否存在来判定更稳妥——但为简化，
    // 我们用"先算一次转换得到真实质量下的路径，存在则跳过"。
    // 这里只做轻量的存在性预判：若该源的任意 _WxH_q*.ext 输出存在则跳过。
    if output_exists_for(source) {
        return RuleDecision::SkipExists;
    }
    RuleDecision::Convert
}

/// 检查该源文件是否已有同 stem 的输出文件存在（任意质量/尺寸）。
/// 用前缀匹配 stem_ 下的 _WxH_q*.ext 模式。
fn output_exists_for(source: &Path) -> bool {
    let Some(dir) = source.parent() else {
        return false;
    };
    let Some(stem) = source.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let prefix = format!("{stem}_");
    for e in entries.flatten() {
        if let Some(name) = e.file_name().to_str() {
            // 是输出文件名 且 以本源 stem 开头 → 已有输出
            if name.starts_with(&prefix)
                && converter::namer::is_output_file_name(name)
            {
                return true;
            }
        }
    }
    false
}

/// 判断 file 是否在 base 目录管辖下（或其子目录，当 recursive）。
fn is_under(base: &Path, file: &Path, recursive: bool) -> bool {
    let Some(file_dir) = file.parent() else {
        return false;
    };
    if file_dir == base {
        return true;
    }
    if recursive {
        return file_dir.starts_with(base);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Directory, Fit, ResizeSpec, Rule};
    use std::path::PathBuf;

    fn dirs() -> Vec<Directory> {
        vec![Directory {
            path: PathBuf::from("/photos"),
            recursive: true,
            rules: vec![Rule {
                name: "thumb".into(),
                resize: Some(ResizeSpec {
                    max_width: 100,
                    max_height: 100,
                    fit: Fit::Inside,
                    no_upscale: false,
                }),
                format: None,
                size_limit: None,
                jpeg_bg: None,
            }],
        }]
    }

    #[test]
    fn skips_output_files() {
        let dirs = dirs();
        let out = classify(&PathBuf::from("/photos/a_100x100_q85.png"), &dirs);
        assert!(matches!(out[0], Decision::SkipOutput));
    }

    #[test]
    fn skips_non_images() {
        let dirs = dirs();
        let out = classify(&PathBuf::from("/photos/notes.txt"), &dirs);
        assert!(matches!(out[0], Decision::SkipNotImage));
    }

    #[test]
    fn classifies_nested_source() {
        let dirs = dirs();
        let out = classify(&PathBuf::from("/photos/sub/deep/a.png"), &dirs);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], Decision::Convert(_)));
    }

    #[test]
    fn ignores_files_outside_directories() {
        let dirs = dirs();
        let out = classify(&PathBuf::from("/other/a.png"), &dirs);
        assert!(out.is_empty());
    }
}
