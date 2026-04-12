use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::AppPaths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountProfile {
    pub puid: u64,
    pub name: String,
    pub phone: String,
    pub school: String,
    pub sex: Option<String>,
    pub student_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Course {
    pub course_id: u64,
    pub class_id: u64,
    pub cpi: u64,
    pub key: u64,
    pub name: String,
    pub teacher_name: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Chapter {
    pub chapter_id: u64,
    pub jobs: u64,
    pub index: u64,
    pub name: String,
    pub label: String,
    pub layer: u64,
    pub status: String,
    pub point_total: u64,
    pub point_finished: u64,
    #[serde(default)]
    pub task_points: Vec<TaskPointSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskPointSummary {
    pub card_index: usize,
    pub point_index: usize,
    pub module: String,
    pub title: String,
    pub resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iframe_data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_metadata: Option<TaskPointAttachmentMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_work_metadata: Option<ChapterWorkMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskAttachmentSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fid: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ktoken: Option<String>,
    #[serde(default)]
    pub attachments: Vec<TaskAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskPointAttachmentMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fid: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<TaskAttachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_status: Option<VideoAttachmentStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterWorkMetadata {
    pub work_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub school_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterWorkAttachmentMetadata {
    pub work_id: String,
    pub job_id: String,
    pub ktoken: String,
    pub enc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterWorkRuntimeRequest {
    pub course_id: u64,
    pub class_id: u64,
    pub knowledge_id: u64,
    pub user_id: u64,
    pub cpi: u64,
    pub card_index: usize,
    pub work_id: String,
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub school_id: Option<String>,
}

impl ChapterWorkRuntimeRequest {
    #[must_use]
    pub fn relation_work_id(&self) -> String {
        self.school_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|school_id| format!("{school_id}-{}", self.work_id))
            .unwrap_or_else(|| self.work_id.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskAttachment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vdo_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jtoken: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_info: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_rate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_passed: Option<bool>,
    #[serde(default)]
    pub is_job: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoAttachmentStatus {
    pub status: String,
    pub filename: String,
    pub duration_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dtoken: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoPlayReportRequest {
    pub cpi: u64,
    pub class_id: u64,
    pub user_id: u64,
    pub dtoken: String,
    pub object_id: String,
    pub job_id: String,
    pub other_info: String,
    pub playing_time_secs: u64,
    pub duration_secs: u64,
    pub clip_time: String,
    pub enc: String,
    pub playback_rate: String,
    pub report_timestamp_millis: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoPlayReportAck {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playing_time_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocumentReadingReportRequest {
    pub course_id: u64,
    pub class_id: u64,
    pub knowledge_id: u64,
    pub job_id: String,
    pub jtoken: String,
    pub report_timestamp_millis: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocumentReadingReportAck {
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveProgressReportRequest {
    pub stream_name: String,
    pub vdo_id: String,
    pub user_id: u64,
    pub is_start: bool,
    pub course_id: u64,
    pub report_timestamp_millis: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveProgressReportAck {
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CourseExam {
    pub exam_id: u64,
    pub course_id: u64,
    pub class_id: u64,
    pub cpi: u64,
    pub enc_task: String,
    pub name: String,
    pub status: String,
    pub expire_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ExamMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExamMeta {
    pub entry_state: String,
    pub title: Option<String>,
    pub exam_answer_id: Option<u64>,
    pub monitor_enc: Option<String>,
    pub need_code: bool,
    pub need_face: bool,
    pub need_captcha: bool,
    pub captcha_id: Option<String>,
    pub blocked_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuestionRichContent {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_html: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExamQuestionOption {
    pub key: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich_content: Option<QuestionRichContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExamQuestionSummary {
    pub question_index: usize,
    pub question_id: u64,
    pub question_type: u64,
    pub question_type_label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub question_kind: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ExamQuestionOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blanks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterWorkQuestionSummary {
    pub question_index: usize,
    pub question_id: u64,
    pub question_type: u64,
    pub question_type_label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub question_kind: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ExamQuestionOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blanks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterWorkFormSnapshot {
    pub title: String,
    pub work_answer_id: u64,
    pub total_question_num: usize,
    pub work_relation_id: u64,
    pub full_score: String,
    pub enc_work: String,
    pub questions: Vec<ChapterWorkQuestionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExamPreviewQuery {
    pub exam_answer_id: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub enc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remain_time_param: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_answer_last_update_time: Option<u64>,
}

impl ExamPreviewQuery {
    pub fn from_exam_answer_id(exam_answer_id: u64) -> Self {
        Self {
            exam_answer_id,
            enc: String::new(),
            remain_time_param: None,
            relation_answer_last_update_time: None,
        }
    }

    pub fn from_exam_meta(meta: &ExamMeta) -> Option<Self> {
        meta.exam_answer_id.map(Self::from_exam_answer_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportManifest {
    pub generated_at: DateTime<Utc>,
    pub account: AccountProfile,
    pub course: Course,
    pub exams: Vec<CourseExam>,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExamPreviewExportManifest {
    pub generated_at: DateTime<Utc>,
    pub account: AccountProfile,
    pub course: Course,
    pub exam: CourseExam,
    pub preview: ExamPreviewQuery,
    pub questions: Vec<ExamQuestionSummary>,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorCheck {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorReport {
    pub config_path: PathBuf,
    pub selected_profile: Option<String>,
    pub automation_paths: AppPaths,
    pub session_dir: PathBuf,
    pub log_dir: PathBuf,
    pub export_dir: PathBuf,
    pub face_dir: PathBuf,
    pub searcher_count: usize,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CookieSnapshot {
    pub hosts: BTreeMap<String, String>,
}

impl CookieSnapshot {
    pub fn empty() -> Self {
        Self {
            hosts: BTreeMap::new(),
        }
    }

    pub fn get(&self, host: &str) -> Option<&str> {
        self.hosts.get(host).map(String::as_str)
    }

    pub fn insert(&mut self, host: impl Into<String>, value: impl Into<String>) {
        self.hosts.insert(host.into(), value.into());
    }
}
