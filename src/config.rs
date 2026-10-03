use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub session_path: PathBuf,
    pub log_path: PathBuf,
    pub export_path: PathBuf,
    pub face_image_path: PathBuf,
    pub multi_session: bool,
    pub mask_acc: bool,
    pub fetch_uploaded_face: bool,
    pub video: VideoConfig,
    pub document: TaskConfig,
    pub work: WorkConfig,
    pub exam: ExamConfig,
    #[serde(deserialize_with = "searcher_list")]
    pub searchers: Vec<Value>,
    pub timeout_secs: u64,
    pub request_retries: u32,
    pub notifications: crate::operations::NotificationConfig,
    pub ocr: crate::operations::OcrConfig,
    pub log_retention_days: u64,
}

fn searcher_list<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Value>, D::Error> {
    Ok(Option::<Vec<Value>>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TaskConfig {
    pub enable: bool,
    pub wait: u64,
}
impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            enable: true,
            wait: 15,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct VideoConfig {
    pub enable: bool,
    pub wait: u64,
    pub speed: f64,
    pub report_rate: u64,
}
impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            enable: true,
            wait: 15,
            speed: 1.0,
            report_rate: 58,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct WorkConfig {
    pub enable: bool,
    pub export: bool,
    pub wait: u64,
    pub fallback_save: bool,
    pub fallback_fuzzer: bool,
}
impl Default for WorkConfig {
    fn default() -> Self {
        Self {
            enable: true,
            export: false,
            wait: 15,
            fallback_save: true,
            fallback_fuzzer: false,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ExamConfig {
    pub fallback_fuzzer: bool,
    pub persubmit_delay: f64,
    pub confirm_submit: bool,
}
impl Default for ExamConfig {
    fn default() -> Self {
        Self {
            fallback_fuzzer: false,
            persubmit_delay: 15.0,
            confirm_submit: true,
        }
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            session_path: "session".into(),
            log_path: "logs".into(),
            export_path: "export".into(),
            face_image_path: "faces".into(),
            multi_session: true,
            mask_acc: true,
            fetch_uploaded_face: true,
            video: VideoConfig::default(),
            document: TaskConfig::default(),
            work: WorkConfig::default(),
            exam: ExamConfig::default(),
            searchers: vec![],
            timeout_secs: 30,
            request_retries: 3,
            notifications: Default::default(),
            ocr: Default::default(),
            log_retention_days: 30,
        }
    }
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).context("无法读取配置文件")?;
        let mut conf: Self = serde_yaml::from_str(&text).context("配置 YAML 无效")?;
        conf.validate()?;
        let base = path.parent().unwrap_or(Path::new("."));
        for dir in [
            &mut conf.session_path,
            &mut conf.log_path,
            &mut conf.export_path,
            &mut conf.face_image_path,
        ] {
            if dir.is_relative() {
                *dir = base.join(&*dir);
            }
        }
        for source in &mut conf.searchers {
            if let Some(p) = source.get_mut("file_path")
                && let Some(name) = p.as_str()
            {
                let file = Path::new(name);
                if file.is_relative() {
                    *p = Value::String(base.join(file).to_string_lossy().into_owned());
                }
            }
        }
        if conf.ocr.executable.is_relative() && conf.ocr.executable.components().count() > 1 {
            conf.ocr.executable = base.join(&conf.ocr.executable);
        }
        Ok(conf)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.timeout_secs > 0 && self.timeout_secs <= 300,
            "timeout_secs 必须为 1..300"
        );
        ensure!(self.request_retries <= 5, "request_retries 不能超过 5");
        ensure!(
            (1..=30).contains(&self.ocr.timeout_secs),
            "OCR 等待时间必须为 1..30 秒"
        );
        ensure!(
            (1..=365).contains(&self.log_retention_days),
            "日志保留时间必须为 1..365 天"
        );
        ensure!(
            self.video.speed.is_finite() && self.video.speed > 0.0,
            "视频倍速必须为正有限数"
        );
        ensure!(
            self.video.report_rate > 0 && self.video.report_rate <= 3600,
            "视频汇报间隔必须为 1..3600 秒"
        );
        ensure!(
            self.exam.persubmit_delay.is_finite()
                && self.exam.persubmit_delay >= 0.0
                && self.exam.persubmit_delay <= 3600.0,
            "提交等待必须为 0..3600 秒"
        );
        ensure!(
            [self.work.wait, self.document.wait, self.video.wait]
                .iter()
                .all(|n| *n <= 3600),
            "任务等待不能超过 3600 秒"
        );
        for source in &self.searchers {
            ensure!(
                source.is_object() && source.get("type").and_then(Value::as_str).is_some(),
                "每个题库必须有 type"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_yaml_null_searchers_and_defaults() {
        let c: Config = serde_yaml::from_str("searchers:\nvideo:\n  speed: 0.5\n").unwrap();
        assert!(c.searchers.is_empty());
        assert_eq!(c.video.speed, 0.5);
        assert!(c.work.fallback_save);
        c.validate().unwrap();
    }
    #[test]
    fn rejects_invalid_timing() {
        let mut c = Config::default();
        c.video.speed = f64::NAN;
        assert!(c.validate().is_err());
        c = Config::default();
        c.ocr.timeout_secs = 30;
        c.validate().unwrap();
        c.ocr.timeout_secs = 31;
        assert!(c.validate().is_err());
    }
}
