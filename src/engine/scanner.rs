// 目录扫描：递归枚举目录下的所有文件
//
// 用于 once 命令的首次全量，以及 start 守护进程启动时的全量。
// 扫描产出候选路径，由 dispatcher 进一步过滤。

use crate::model::Directory;
use std::path::PathBuf;

/// 扫描单个目录，返回所有文件路径。recursive 控制是否进入子目录。
pub fn scan_dir(dir: &Directory) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if dir.recursive {
        for entry in walkdir::WalkDir::new(&dir.path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                out.push(entry.path().to_path_buf());
            }
        }
    } else {
        if let Ok(entries) = std::fs::read_dir(&dir.path) {
            for e in entries.flatten() {
                if let Ok(ft) = e.file_type() {
                    if ft.is_file() {
                        out.push(e.path());
                    }
                }
            }
        }
    }
    out
}

/// 扫描多个目录，去重。
pub fn scan_all(directories: &[Directory]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for d in directories {
        for p in scan_dir(d) {
            if seen.insert(p.clone()) {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Directory, Rule};
    use std::path::PathBuf;

    fn mk_dir() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::write(root.join("a.png"), b"x").unwrap();
        std::fs::write(root.join("sub/b.jpg"), b"x").unwrap();
        std::fs::write(root.join("sub/deep/c.png"), b"x").unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();
        (t, root)
    }

    #[test]
    fn recursive_finds_all_files() {
        let (_t, root) = mk_dir();
        let d = Directory {
            path: root.clone(),
            recursive: true,
            rules: vec![Rule {
                name: "r".into(),
                resize: None,
                format: None,
                size_limit: None,
                jpeg_bg: None,
            }],
        };
        let files = scan_dir(&d);
        assert_eq!(files.len(), 4); // 3 图 + 1 txt
        assert!(files.iter().any(|p| p.ends_with("c.png")));
    }

    #[test]
    fn non_recursive_only_top() {
        let (_t, root) = mk_dir();
        let d = Directory {
            path: root.clone(),
            recursive: false,
            rules: vec![Rule {
                name: "r".into(),
                resize: None,
                format: None,
                size_limit: None,
                jpeg_bg: None,
            }],
        };
        let files = scan_dir(&d);
        assert_eq!(files.len(), 2); // a.png + notes.txt
    }
}
