// 防套娃（无限生成）专项测试
//
// 这是整个系统最关键的安全测试。验证：系统输出文件永远不会被当作源再次转换，
// 因此不会产生 _q85_q70.jpg 这样的二阶输出，也不会无限循环撑爆磁盘。

mod common;

use common::*;
use picfug::converter;
use picfug::db::Db;
use picfug::engine;
use picfug::model::{Directory, Status};
use picfug::worker;
use std::path::{Path, PathBuf};

/// 核心断言：转换一个源文件后，输出的文件名能被 is_output_file 识别（自身排除）。
#[test]
fn output_is_self_identifying() {
    let (_t, dirs, src) = setup_single_rule(50, 50);
    let task = engine::Task {
        source: src.clone(),
        dir_index: 0,
        rule_index: 0,
    };
    let db = Db::open_memory().unwrap();
    let res = worker::run_task(&task, &dirs, &db);
    assert_eq!(res.status, Status::Done);

    let outputs = list_outputs(src.parent().unwrap());
    assert_eq!(outputs.len(), 1, "应恰好生成 1 个输出");
    for o in &outputs {
        assert!(
            converter::namer::is_output_file(o),
            "输出文件 {} 必须能被自身识别机制命中",
            o.display()
        );
    }
}

/// 防套娃核心：dispatcher 必须拒绝处理输出文件。
#[test]
fn dispatcher_rejects_output_files() {
    let (_t, dirs, src) = setup_single_rule(100, 100);
    // 一个"长得像"输出的文件名
    let fake_output = src.parent().unwrap().join("photo_100x100_q85.png");
    let decisions = engine::classify(&fake_output, &dirs);
    assert!(
        decisions
            .iter()
            .all(|d| matches!(d, engine::Decision::SkipOutput)),
        "输出文件必须被识别并跳过，实际: {:?}",
        decisions
    );
}

/// 端到端防套娃：跑两次，断言输出数量不翻倍、无二阶后缀文件。
#[test]
fn repeated_runs_do_not_multiply_outputs() {
    let (_t, dirs, db, src) = setup_single_rule_with_db(100, 100);

    let files = engine::scanner::scan_all(&dirs);
    run_all(&files, &dirs, &db);
    let after_first = list_outputs(src.parent().unwrap());
    assert_eq!(after_first.len(), 1, "第一次应有 1 个输出");

    // 第二次扫描包含输出文件
    let files2 = engine::scanner::scan_all(&dirs);
    assert!(
        files2.len() > after_first.len(),
        "第二次扫描应包含输出文件"
    );
    run_all(&files2, &dirs, &db);
    let after_second = list_outputs(src.parent().unwrap());
    assert_eq!(
        after_second.len(),
        1,
        "第二次运行后输出数不应增加，实际: {:?}",
        after_second
    );

    // 关键断言：绝不能出现二阶后缀（如 _q85_q85）—— 这是套娃的征兆
    let all_files = list_all_files(src.parent().unwrap());
    for f in &all_files {
        let name = f.file_name().unwrap().to_string_lossy();
        let q_count = name.matches("_q").count();
        assert!(
            q_count <= 1,
            "检测到二阶或多阶后缀（套娃征兆）: {}",
            name
        );
    }
}

/// 多规则同源：应产生多个独立输出，互不干扰，且都自识别。
#[test]
fn multiple_rules_produce_distinct_outputs() {
    let (_t, dirs, db, src) = setup_two_rules_with_db();
    let files = engine::scanner::scan_all(&dirs);
    run_all(&files, &dirs, &db);

    let outputs = list_outputs(src.parent().unwrap());
    assert_eq!(outputs.len(), 2, "两条规则应产出 2 个输出");
    for o in &outputs {
        assert!(converter::namer::is_output_file(o));
    }

    // 再跑一次，不应翻倍
    let files2 = engine::scanner::scan_all(&dirs);
    run_all(&files2, &dirs, &db);
    let outputs2 = list_outputs(src.parent().unwrap());
    assert_eq!(outputs2.len(), 2, "二次运行不应增加输出");
}

/// 嵌套子目录：所有层级都被覆盖，输出与源同目录，防套娃在每层都生效。
#[test]
fn nested_directories_all_covered() {
    let (t, dirs, db) = setup_nested_with_db();
    let root = t.path().to_path_buf();
    let files = engine::scanner::scan_all(&dirs);
    // 包含顶层和子目录的源
    assert!(files.len() >= 2);

    run_all(&files, &dirs, &db);

    // 每层都有输出，且都自识别
    let top_outputs = list_outputs(&root);
    let sub_outputs = list_outputs(&root.join("sub"));
    assert!(!top_outputs.is_empty(), "顶层应有输出");
    assert!(!sub_outputs.is_empty(), "子目录应有输出");
    for o in top_outputs.iter().chain(sub_outputs.iter()) {
        assert!(converter::namer::is_output_file(o));
    }

    // 二次运行不翻倍
    run_all(&engine::scanner::scan_all(&dirs), &dirs, &db);
    assert_eq!(list_outputs(&root).len(), top_outputs.len());
    assert_eq!(list_outputs(&root.join("sub")).len(), sub_outputs.len());
}

/// 源文件变更后，输出数量仍稳定为 1（不产生额外文件）。
#[test]
fn source_change_does_not_duplicate() {
    let (_t, dirs, db, src) = setup_single_rule_with_db(100, 100);
    run_all(&engine::scanner::scan_all(&dirs), &dirs, &db);
    assert_eq!(list_outputs(src.parent().unwrap()).len(), 1);

    // 重写源图（不同内容）
    write_solid_png(&src, 200, 200, image::Rgba([0, 255, 0, 255]));
    run_all(&engine::scanner::scan_all(&dirs), &dirs, &db);
    assert_eq!(
        list_outputs(src.parent().unwrap()).len(),
        1,
        "源变更后输出数仍应为 1"
    );
}

fn run_all(files: &[PathBuf], dirs: &[Directory], db: &Db) {
    for f in files {
        let decisions = engine::classify(f, dirs);
        for d in decisions {
            if let engine::Decision::Convert(task) = d {
                let _ = worker::run_task(&task, dirs, db);
            }
        }
    }
}

// 抑制未使用警告（Path 在 common 中用）
#[allow(dead_code)]
fn _use_path(_p: &Path) {}
