// 配置加载与校验
//
// YAML → Config（serde 反序列化）→ 业务校验（目录存在、数值合法、规则非空...）。
// 校验失败的错误必须明确指出问题，便于用户修正。

use crate::error::PicfugError;
use crate::model::{Directory, Fit, OutputFormat, ResizeSpec, Rule, SizeLimit};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// 顶层配置。
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub log: Option<LogConfig>,
    pub database: DatabaseConfig,
    pub watcher: Option<WatcherConfig>,
    #[serde(default)]
    pub directories: Vec<DirectoryConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_dir")]
    pub dir: PathBuf,
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_rotation")]
    pub rotation: String,
}

fn default_log_dir() -> PathBuf {
    PathBuf::from("./logs")
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_rotation() -> String {
    "daily".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatcherConfig {
    #[serde(default = "default_poll_interval")]
    pub poll_interval: DurationCfg,
    #[serde(default = "default_debounce")]
    pub debounce: DurationCfg,
    #[serde(default = "default_worker_concurrency")]
    pub worker_concurrency: usize,
}

fn default_poll_interval() -> DurationCfg {
    DurationCfg(std::time::Duration::from_secs(30))
}
fn default_debounce() -> DurationCfg {
    DurationCfg(std::time::Duration::from_secs(2))
}
fn default_worker_concurrency() -> usize {
    4
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval: default_poll_interval(),
            debounce: default_debounce(),
            worker_concurrency: default_worker_concurrency(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DirectoryConfig {
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub recursive: bool,
    pub rules: Vec<RuleConfig>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuleConfig {
    pub name: String,
    pub resize: Option<ResizeSpecConfig>,
    /// None 表示沿用源格式
    pub format: Option<OutputFormatConfig>,
    pub size_limit: Option<SizeLimitConfig>,
    pub jpeg_bg: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ResizeSpecConfig {
    pub max_width: u32,
    pub max_height: u32,
    #[serde(default = "default_fit")]
    pub fit: FitConfig,
    #[serde(default = "default_true")]
    pub no_upscale: bool,
}

fn default_fit() -> FitConfig {
    FitConfig::Inside
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FitConfig {
    Inside,
    Cover,
    Exact,
}

impl From<FitConfig> for Fit {
    fn from(f: FitConfig) -> Self {
        match f {
            FitConfig::Inside => Fit::Inside,
            FitConfig::Cover => Fit::Cover,
            FitConfig::Exact => Fit::Exact,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormatConfig {
    Jpeg,
    Png,
    Webp,
}

impl From<OutputFormatConfig> for OutputFormat {
    fn from(f: OutputFormatConfig) -> Self {
        match f {
            OutputFormatConfig::Jpeg => OutputFormat::Jpeg,
            OutputFormatConfig::Png => OutputFormat::Png,
            OutputFormatConfig::Webp => OutputFormat::Webp,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SizeLimitConfig {
    pub max_bytes: u64,
    #[serde(default = "default_max_quality")]
    pub max_quality: u8,
    #[serde(default = "default_min_quality")]
    pub min_quality: u8,
    #[serde(default = "default_step")]
    pub step: u8,
}

fn default_max_quality() -> u8 {
    90
}
fn default_min_quality() -> u8 {
    70
}
fn default_step() -> u8 {
    5
}

// humantime 不直接支持 serde，这里用一个 newtype 包一下手动解析（接受 "30s" / "2s" 等）。
// 用 serde 的内部标记会复杂化 schema，因此采用简单的字符串解析路径。
#[derive(Debug, Clone, Copy)]
pub struct DurationCfg(pub std::time::Duration);

impl<'de> serde::Deserialize<'de> for DurationCfg {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        humantime::parse_duration(&s)
            .map(DurationCfg)
            .map_err(serde::de::Error::custom)
    }
}

impl Config {
    /// 从 YAML 文件加载并校验。
    pub fn load(path: &Path) -> Result<Self, PicfugError> {
        if !path.exists() {
            return Err(PicfugError::ConfigNotFound(path.to_path_buf()));
        }
        let content = std::fs::read_to_string(path).map_err(|e| PicfugError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        let mut cfg: Config =
            serde_yaml::from_str(&content).map_err(|source| PicfugError::ConfigParse {
                path: path.to_path_buf(),
                source,
            })?;
        // 给 database.path 一个相对路径基准（相对配置文件所在目录），让用户写相对路径更友好
        cfg.normalize_paths(path.parent().unwrap_or(Path::new(".")));
        cfg.validate()?;
        Ok(cfg)
    }

    /// 把相对路径补全为相对配置文件所在目录的绝对路径，避免 CWD 漂移导致找不到。
    fn normalize_paths(&mut self, base: &Path) {
        normalize(&mut self.database.path, base);
        if let Some(log) = self.log.as_mut() {
            normalize(&mut log.dir, base);
        }
        for d in self.directories.iter_mut() {
            normalize(&mut d.path, base);
        }
    }

    /// 业务校验：所有会直接导致运行期失败的问题都在这里前置拦截。
    pub fn validate(&self) -> Result<(), PicfugError> {
        if self.directories.is_empty() {
            return Err(PicfugError::ConfigInvalid(
                "至少需要配置一个 directory".to_string(),
            ));
        }
        for d in &self.directories {
            if !d.path.exists() {
                return Err(PicfugError::ConfigInvalid(format!(
                    "目录不存在: {}",
                    d.path.display()
                )));
            }
            if !d.path.is_dir() {
                return Err(PicfugError::ConfigInvalid(format!(
                    "路径不是目录: {}",
                    d.path.display()
                )));
            }
            if d.rules.is_empty() {
                return Err(PicfugError::ConfigInvalid(format!(
                    "目录 {} 没有配置任何 rule",
                    d.path.display()
                )));
            }
            for r in &d.rules {
                validate_rule(r)?;
            }
        }
        if let Some(w) = &self.watcher {
            if w.worker_concurrency == 0 {
                return Err(PicfugError::ConfigInvalid(
                    "watcher.worker_concurrency 必须 > 0".to_string(),
                ));
            }
            if w.poll_interval.0.is_zero() {
                return Err(PicfugError::ConfigInvalid(
                    "watcher.poll_interval 必须 > 0".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// 转为业务模型 Directory 列表。
    pub fn to_directories(&self) -> Vec<Directory> {
        self.directories
            .iter()
            .map(|dc| Directory {
                path: dc.path.clone(),
                recursive: dc.recursive,
                rules: dc.rules.iter().map(rule_config_to_model).collect(),
            })
            .collect()
    }
}

fn normalize(p: &mut PathBuf, base: &Path) {
    if !p.is_absolute() {
        *p = base.join(&*p);
    }
}

fn rule_config_to_model(rc: &RuleConfig) -> Rule {
    Rule {
        name: rc.name.clone(),
        resize: rc.resize.map(|r| ResizeSpec {
            max_width: r.max_width,
            max_height: r.max_height,
            fit: r.fit.into(),
            no_upscale: r.no_upscale,
        }),
        format: rc.format.map(|f| f.into()),
        size_limit: rc.size_limit.map(|s| SizeLimit {
            max_bytes: s.max_bytes,
            max_quality: s.max_quality,
            min_quality: s.min_quality,
            step: s.step,
        }),
        jpeg_bg: rc.jpeg_bg.clone(),
    }
}

fn validate_rule(r: &RuleConfig) -> Result<(), PicfugError> {
    if r.name.trim().is_empty() {
        return Err(PicfugError::ConfigInvalid("rule.name 不能为空".to_string()));
    }
    if let Some(rz) = r.resize {
        if rz.max_width == 0 && rz.max_height == 0 {
            return Err(PicfugError::ConfigInvalid(format!(
                "rule '{}' 的 resize.max_width 和 max_height 不能同时为 0",
                r.name
            )));
        }
    }
    if let Some(sl) = r.size_limit {
        if sl.max_quality < 1 || sl.max_quality > 100 {
            return Err(PicfugError::ConfigInvalid(format!(
                "rule '{}' 的 size_limit.max_quality 必须在 1..=100",
                r.name
            )));
        }
        if sl.min_quality < 1 || sl.min_quality > 100 {
            return Err(PicfugError::ConfigInvalid(format!(
                "rule '{}' 的 size_limit.min_quality 必须在 1..=100",
                r.name
            )));
        }
        if sl.min_quality > sl.max_quality {
            return Err(PicfugError::ConfigInvalid(format!(
                "rule '{}' 的 size_limit.min_quality 不能大于 max_quality",
                r.name
            )));
        }
        if sl.step == 0 {
            return Err(PicfugError::ConfigInvalid(format!(
                "rule '{}' 的 size_limit.step 不能为 0",
                r.name
            )));
        }
        if sl.max_bytes == 0 {
            return Err(PicfugError::ConfigInvalid(format!(
                "rule '{}' 的 size_limit.max_bytes 不能为 0",
                r.name
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_cfg(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let watched = dir.path().join("pics");
        std::fs::create_dir(&watched).unwrap();
        let cfg_path = dir.path().join("config.yaml");
        std::fs::write(&cfg_path, content.replace("__PICS__", watched.to_str().unwrap())).unwrap();
        (dir, cfg_path)
    }

    #[test]
    fn load_minimal_valid() {
        let (_t, p) = tmp_cfg(
            r#"
database: { path: ./picfug.db }
directories:
  - path: __PICS__
    recursive: true
    rules:
      - name: thumb
        resize: { max_width: 100, max_height: 100 }
"#,
        );
        let cfg = Config::load(&p).unwrap();
        assert_eq!(cfg.directories.len(), 1);
        assert_eq!(cfg.directories[0].rules.len(), 1);
    }

    #[test]
    fn reject_empty_directories() {
        let (_t, p) = tmp_cfg("database: { path: ./db }\ndirectories: []");
        let err = Config::load(&p).unwrap_err();
        assert!(matches!(err, PicfugError::ConfigInvalid(_)));
    }

    #[test]
    fn reject_bad_quality_range() {
        let (_t, p) = tmp_cfg(
            r#"
database: { path: ./db }
directories:
  - path: __PICS__
    rules:
      - name: x
        size_limit: { max_bytes: 1000, max_quality: 50, min_quality: 80, step: 5 }
"#,
        );
        let err = Config::load(&p).unwrap_err();
        match err {
            PicfugError::ConfigInvalid(m) => assert!(m.contains("min_quality")),
            e => panic!("unexpected {e:?}"),
        }
    }

    #[test]
    fn reject_nonexistent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("c.yaml");
        std::fs::write(
            &cfg_path,
            "database: { path: ./db }\ndirectories:\n  - path: /no/such/dir/xyz\n    rules:\n      - { name: r }\n",
        )
        .unwrap();
        let err = Config::load(&cfg_path).unwrap_err();
        assert!(matches!(err, PicfugError::ConfigInvalid(_)));
    }

    #[test]
    fn parse_humantime_duration() {
        let (_t, p) = tmp_cfg(
            r#"
database: { path: ./db }
watcher:
  poll_interval: 15s
  debounce: 1s
  worker_concurrency: 8
directories:
  - path: __PICS__
    rules:
      - { name: r }
"#,
        );
        let cfg = Config::load(&p).unwrap();
        let w = cfg.watcher.unwrap();
        assert_eq!(w.poll_interval.0, std::time::Duration::from_secs(15));
        assert_eq!(w.worker_concurrency, 8);
    }
}
