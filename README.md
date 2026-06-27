# picfug

图片文件自动转换 CLI 守护进程。扫描指定目录中的图片，按规则进行缩放、压缩、格式转换，
在**原目录**生成带规格后缀的新文件（原文件保留），适合图片批量处理、缩略图生成等场景。

## 特性

- 🔄 **守护进程**：文件监听 + 定时轮询双机制，持续转换新增图片
- 🖼️ **多规则**：一个目录可配多条规则，分别产出不同规格的输出
- 📐 **缩放**：inside / cover / exact 三种模式，支持不放大选项
- 🗜️ **压缩**：JPEG 支持硬上限迭代压缩（逐级降质达标）
- 🎨 **格式**：可指定输出格式，或沿用源格式；透明图转 JPEG 自动合成背景色
- 🛡️ **防套娃**：输出文件永不回流为输入，杜绝无限生成
- 📊 **可追溯**：SQLite 记录每个文件的转换状态、失败原因，便于定位修复
- 📝 **完整 CLI**：check / once / start / status / list / retry / logs

## 安装

```bash
cargo build --release
# 二进制位于 target/release/picfug
```

## 快速开始

```bash
# 1. 准备配置（参考 config.example.yaml）
cp config.example.yaml config.yaml
# 编辑 config.yaml，指向你的图片目录

# 2. 校验配置
picfug check -c config.yaml

# 3. 单次全量转换
picfug once -c config.yaml

# 4. 或启动守护进程持续监听
picfug start -c config.yaml

# 5. 查看状态
picfug status -c config.yaml
```

## 命令一览

| 命令 | 说明 |
|------|------|
| `check` | 校验配置文件，列出目录与规则 |
| `once` | 单次全量扫描转换后退出 |
| `start` | 启动守护进程（首次全量 + 持续监听/轮询） |
| `status` | 汇总 done / failed / pending / skipped 计数 |
| `list` | 按状态列出文件（默认 failed，含路径与失败原因） |
| `retry` | 重置失败项为 pending 并重试 |
| `logs` | 查看日志（`-f` 跟踪） |

`list` 常用选项：`--status done|failed|pending`、`--dir <前缀>`、`--rule <名>`、`--json`、`--limit/--offset`。

## 配置文件

```yaml
log:
  dir: ./logs
  level: info              # trace | debug | info | warn | error
  rotation: daily          # daily | hourly | never

database:
  path: ./picfug.db

watcher:
  poll_interval: 30s       # 轮询兜底间隔
  debounce: 2s             # 文件事件去抖
  worker_concurrency: 4    # 并发数

directories:
  - path: ./photos         # 相对路径基于配置文件所在目录
    recursive: true        # 是否处理子目录（默认 true）
    rules:
      - name: thumbnail
        resize:
          max_width: 300
          max_height: 300
          fit: inside      # inside | cover | exact
          no_upscale: true
        format: jpeg        # 不指定则沿用源格式
        size_limit:
          max_bytes: 102400
          max_quality: 90
          min_quality: 70
          step: 5
        jpeg_bg: "#ffffff"  # 透明转 jpeg 背景色
```

## 输出文件命名

源文件 `{stem}.{ext}` → 输出 `{stem}_{宽}x{高}_q{质量}.{ext}`，与源同目录：

| 源 | 规则 | 输出 |
|----|------|------|
| `photo.png` | 100×100, 默认格式 | `photo_100x100_q100.png` |
| `logo.png` | 300×300, jpeg, 压缩到 q78 | `logo_300x300_q78.jpg` |
| `big.tiff` | 仅压缩无缩放 | `big_0x0_q100.tiff` |

## 防套娃机制

这是本工具的核心安全保障。系统输出文件名遵循固定结构 `_{宽}x{高}_q{质量}.{ext}`，
调度器（dispatcher）通过正则识别这类文件名并**永远跳过**，因此：

- 输出文件被监听到也不会再次转换
- 不会出现 `_q85_q70.jpg` 这样的二阶输出
- 不会因无限循环撑爆磁盘

判定逻辑单一、确定、无并发问题。详见 `src/converter/namer.rs` 与 `tests/infinite_loop_guard.rs`。

## 已知限制（诚实声明）

受 [image crate 0.25](https://crates.io/crates/image) 能力限制：

- **WebP 仅支持无损编码**：无法通过降质量逼近 `size_limit`，与 PNG/BMP/TIFF 一样只能"尽力而为"。
  若需对某目录严格控体积，建议源文件用 JPEG 格式（唯一支持迭代压缩达标）。
- **AVIF/HEIC 不支持**：image crate 当前不支持这些格式的编码。

这些是底层库的客观限制，非缺陷。

## 项目结构

```
src/
├── lib.rs            库入口
├── main.rs           CLI 入口
├── cli.rs            命令定义与实现
├── config.rs         YAML 配置加载与校验
├── model.rs          领域模型
├── error.rs          统一错误类型
├── log.rs            日志（滚动文件 + 控制台）
├── db/               SQLite 状态库
├── converter/        转换核心：namer / resize / compress / format
├── engine/           调度：dispatcher(防套娃) / scanner / watcher / poller
└── worker.rs         任务执行：转换 + 写盘 + 状态更新
tests/
├── common/           测试辅助
└── infinite_loop_guard.rs   防套娃专项测试
```

## 测试

```bash
cargo test
```

包含 52 个单元测试 + 6 个防套娃专项集成测试，覆盖命名识别、缩放、迭代压缩、格式转换、
状态库、防套娃端到端等。
