use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::chaoxing::ChaoxingClient;
use crate::error::{CpassError, Result};
use crate::event::{CourseRunExecutionState, CourseRunQueueEntryState, RunEvent, RunEventSink};
use crate::models::{
    AccountProfile, Chapter, ChapterWorkMetadata, ChapterWorkRuntimeRequest, Course, CourseExam,
    DocumentReadingReportRequest, ExamPreviewQuery, ExamQuestionOption, ExamQuestionSummary,
    LiveProgressReportRequest, TaskPointAttachmentMetadata, TaskPointSummary,
    VideoPlayReportRequest,
};
use crate::question_kind::NormalizedQuestionKind;
use crate::searcher::{
    ChapterWorkCandidateSelectionBatch, ChapterWorkQueryBatch, SearcherPipeline,
};
use crate::task_executor::{
    TaskExecutorKey, TaskExecutorQueueResolution, TaskExecutorRegistration, TaskExecutorRegistry,
    TaskExecutorResolution,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CourseRunTarget {
    pub course_id: Option<u64>,
    pub course_index: Option<usize>,
}

impl CourseRunTarget {
    pub fn new(course_id: Option<u64>, course_index: Option<usize>) -> Result<Self> {
        match (course_id, course_index) {
            (Some(_), Some(_)) => Err(CpassError::Validation(
                "only one of --course-id or --course-index may be set".to_owned(),
            )),
            (None, None) => Err(CpassError::Validation(
                "one of --course-id or --course-index is required".to_owned(),
            )),
            _ => Ok(Self {
                course_id,
                course_index,
            }),
        }
    }

    pub fn resolve_course(self, courses: Vec<Course>) -> Result<Course> {
        if let Some(course_id) = self.course_id {
            return courses
                .into_iter()
                .find(|course| course.course_id == course_id)
                .ok_or_else(|| CpassError::Validation(format!("course_id {course_id} not found")));
        }

        if let Some(course_index) = self.course_index {
            return courses.into_iter().nth(course_index).ok_or_else(|| {
                CpassError::Validation(format!("course index {course_index} not found"))
            });
        }

        Err(CpassError::Validation(
            "one of --course-id or --course-index is required".to_owned(),
        ))
    }
}

pub struct CourseRunner<'a> {
    client: &'a ChaoxingClient,
    sink: Option<Arc<dyn RunEventSink>>,
}

impl<'a> CourseRunner<'a> {
    #[must_use]
    pub fn new(client: &'a ChaoxingClient) -> Self {
        Self { client, sink: None }
    }

    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    pub async fn build_plan(&self, target: CourseRunTarget) -> Result<CourseRunPlan> {
        self.emit(RunEvent::CourseRunPlanningStarted {
            course_id: target.course_id,
            course_index: target.course_index,
        });
        let course = target.resolve_course(self.client.fetch_courses().await?)?;
        let chapters = self
            .client
            .fetch_task_scan_with_attachment_metadata(&course)
            .await?;
        let plan = CourseRunPlan::from_scanned_chapters(course, chapters);
        self.emit(RunEvent::CourseRunPlanningFinished {
            course_id: plan.course.course_id,
            chapters: plan.chapters.len(),
            task_points: plan.total_task_points,
        });
        Ok(plan)
    }

    fn emit(&self, event: RunEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "module", rename_all = "snake_case")]
pub enum CourseRunTaskKind {
    Video,
    Document,
    Live,
    ChapterWork,
    Unknown(String),
}

impl CourseRunTaskKind {
    #[must_use]
    pub fn from_module(module: &str) -> Self {
        match module {
            "insertvideo" => Self::Video,
            "insertdoc" => Self::Document,
            "insertlive" => Self::Live,
            "work" => Self::ChapterWork,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CourseRunTaskPoint {
    pub card_index: usize,
    pub point_index: usize,
    pub module: String,
    pub task_kind: CourseRunTaskKind,
    pub title: String,
    pub resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iframe_data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_metadata: Option<TaskPointAttachmentMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_work_metadata: Option<ChapterWorkMetadata>,
}

impl From<TaskPointSummary> for CourseRunTaskPoint {
    fn from(value: TaskPointSummary) -> Self {
        Self {
            card_index: value.card_index,
            point_index: value.point_index,
            task_kind: CourseRunTaskKind::from_module(&value.module),
            module: value.module,
            title: value.title,
            resource_id: value.resource_id,
            iframe_data: value.iframe_data,
            attachment_metadata: value.attachment_metadata,
            chapter_work_metadata: value.chapter_work_metadata,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CourseRunQueueEntry {
    pub queue_index: usize,
    pub chapter_id: u64,
    pub chapter_index: u64,
    pub chapter_name: String,
    pub chapter_label: String,
    pub card_index: usize,
    pub point_index: usize,
    pub module: String,
    pub task_kind: CourseRunTaskKind,
    pub title: String,
    pub resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iframe_data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_metadata: Option<TaskPointAttachmentMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_work_metadata: Option<ChapterWorkMetadata>,
}

impl CourseRunQueueEntry {
    pub fn video_execution_context(&self) -> Result<VideoTaskExecutionContext> {
        VideoTaskExecutionContext::try_from(self)
    }

    pub fn live_execution_context(&self) -> Result<LiveTaskExecutionContext> {
        LiveTaskExecutionContext::try_from(self)
    }

    pub fn document_execution_context(&self) -> Result<DocumentTaskExecutionContext> {
        DocumentTaskExecutionContext::try_from(self)
    }

    pub fn chapter_work_execution_context(&self) -> Result<ChapterWorkTaskExecutionContext> {
        ChapterWorkTaskExecutionContext::try_from(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoTaskExecutionContext {
    pub queue_index: usize,
    pub chapter_id: u64,
    pub card_index: usize,
    pub point_index: usize,
    pub object_id: String,
    pub title: String,
    pub job_id: String,
    pub other_info: String,
    pub playback_rate: String,
    pub fid: u64,
    pub duration_secs: u64,
    pub dtoken: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveTaskExecutionContext {
    pub queue_index: usize,
    pub chapter_id: u64,
    pub card_index: usize,
    pub point_index: usize,
    pub title: String,
    pub live_id: String,
    pub job_id: String,
    pub stream_name: String,
    pub vdo_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocumentTaskExecutionContext {
    pub queue_index: usize,
    pub chapter_id: u64,
    pub card_index: usize,
    pub point_index: usize,
    pub object_id: String,
    pub title: String,
    pub job_id: String,
    pub jtoken: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterWorkTaskExecutionContext {
    pub queue_index: usize,
    pub chapter_id: u64,
    pub card_index: usize,
    pub point_index: usize,
    pub title: String,
    pub work_id: String,
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub school_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LiveTaskIframeSnapshot {
    #[serde(default, rename = "liveId", alias = "liveid")]
    live_id: Option<String>,
    #[serde(default, rename = "vdoid", alias = "vdoId")]
    vdo_id: Option<String>,
    #[serde(default, rename = "streamName", alias = "stream_name")]
    stream_name: Option<String>,
}

const VIDEO_PLAY_REPORT_ENC_SALT: &str = "d_yHJ!$pdA~5";
const MD5_SHIFT_AMOUNTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];
const MD5_TABLE: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

impl TryFrom<&CourseRunQueueEntry> for VideoTaskExecutionContext {
    type Error = CpassError;

    fn try_from(entry: &CourseRunQueueEntry) -> Result<Self> {
        if entry.module != "insertvideo" || entry.task_kind != CourseRunTaskKind::Video {
            return Err(CpassError::Validation(format!(
                "queue entry {} is not a video task point",
                entry.queue_index
            )));
        }

        let metadata = entry.attachment_metadata.as_ref().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing video attachment metadata",
                entry.queue_index
            ))
        })?;
        let attachment = metadata.attachment.as_ref().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing video attachment details",
                entry.queue_index
            ))
        })?;

        if attachment.attachment_type.as_deref() != Some("video") {
            return Err(CpassError::Validation(format!(
                "queue entry {} attachment type is not video",
                entry.queue_index
            )));
        }
        if !attachment.is_job {
            return Err(CpassError::Validation(format!(
                "queue entry {} does not map to a legacy video job attachment",
                entry.queue_index
            )));
        }

        let object_id = match attachment.object_id.as_deref() {
            Some(object_id) if object_id == entry.resource_id => object_id.to_owned(),
            Some(object_id) => {
                return Err(CpassError::Validation(format!(
                    "queue entry {} object_id {} does not match resource_id {}",
                    entry.queue_index, object_id, entry.resource_id
                )));
            }
            None => entry.resource_id.clone(),
        };

        let playback_rate = attachment
            .playback_rate
            .clone()
            .unwrap_or_else(|| "0.9".to_owned());
        let parsed_rate = playback_rate.parse::<f32>().map_err(|_| {
            CpassError::Validation(format!(
                "queue entry {} has invalid video playback_rate {}",
                entry.queue_index, playback_rate
            ))
        })?;
        if !(parsed_rate.is_finite() && parsed_rate > 0.0) {
            return Err(CpassError::Validation(format!(
                "queue entry {} has non-positive video playback_rate {}",
                entry.queue_index, playback_rate
            )));
        }

        let job_id = attachment.job_id.clone().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing video job_id",
                entry.queue_index
            ))
        })?;
        let other_info = attachment.other_info.clone().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing video other_info",
                entry.queue_index
            ))
        })?;
        let fid = metadata.fid.ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing video fid",
                entry.queue_index
            ))
        })?;
        let video_status = metadata.video_status.as_ref().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing video status metadata",
                entry.queue_index
            ))
        })?;
        if video_status.duration_secs == 0 {
            return Err(CpassError::Validation(format!(
                "queue entry {} has zero video duration",
                entry.queue_index
            )));
        }
        let dtoken = video_status.dtoken.clone().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing video dtoken",
                entry.queue_index
            ))
        })?;

        Ok(Self {
            queue_index: entry.queue_index,
            chapter_id: entry.chapter_id,
            card_index: entry.card_index,
            point_index: entry.point_index,
            object_id,
            title: entry.title.clone(),
            job_id,
            other_info,
            playback_rate,
            fid,
            duration_secs: video_status.duration_secs,
            dtoken,
        })
    }
}

impl TryFrom<&CourseRunQueueEntry> for LiveTaskExecutionContext {
    type Error = CpassError;

    fn try_from(entry: &CourseRunQueueEntry) -> Result<Self> {
        if entry.module != "insertlive" || entry.task_kind != CourseRunTaskKind::Live {
            return Err(CpassError::Validation(format!(
                "queue entry {} is not a live task point",
                entry.queue_index
            )));
        }

        let require_live_field =
            |value: Option<&str>, source: &str, field: &str| -> Result<String> {
                match value.map(str::trim) {
                    Some(value) if !value.is_empty() => Ok(value.to_owned()),
                    _ => Err(CpassError::Validation(format!(
                        "queue entry {} is missing live {field} in {source}",
                        entry.queue_index
                    ))),
                }
            };
        let ensure_match =
            |field: &str, expected: &str, actual: &str, actual_source: &str| -> Result<()> {
                if expected == actual {
                    Ok(())
                } else {
                    Err(CpassError::Validation(format!(
                        "queue entry {} live {field} in {} {} does not match {}",
                        entry.queue_index, actual_source, actual, expected
                    )))
                }
            };

        let iframe_data = entry.iframe_data.as_deref().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing live iframe_data",
                entry.queue_index
            ))
        })?;
        let iframe_snapshot: LiveTaskIframeSnapshot =
            serde_json::from_str(iframe_data).map_err(|error| {
                CpassError::Validation(format!(
                    "queue entry {} has invalid live iframe_data: {error}",
                    entry.queue_index
                ))
            })?;
        let iframe_live_id =
            require_live_field(iframe_snapshot.live_id.as_deref(), "iframe_data", "live_id")?;
        let iframe_stream_name = require_live_field(
            iframe_snapshot.stream_name.as_deref(),
            "iframe_data",
            "stream_name",
        )?;
        let iframe_vdo_id =
            require_live_field(iframe_snapshot.vdo_id.as_deref(), "iframe_data", "vdo_id")?;

        ensure_match(
            "live_id",
            &entry.resource_id,
            &iframe_live_id,
            "iframe_data",
        )?;

        let metadata = entry.attachment_metadata.as_ref().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing live attachment metadata",
                entry.queue_index
            ))
        })?;
        let attachment = metadata.attachment.as_ref().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing live attachment details",
                entry.queue_index
            ))
        })?;

        if attachment.attachment_type.as_deref() != Some("live") {
            return Err(CpassError::Validation(format!(
                "queue entry {} attachment type is not live",
                entry.queue_index
            )));
        }
        if !attachment.is_job {
            return Err(CpassError::Validation(format!(
                "queue entry {} does not map to a legacy live job attachment",
                entry.queue_index
            )));
        }

        let attachment_live_id = require_live_field(
            attachment.live_id.as_deref(),
            "attachment metadata",
            "live_id",
        )?;
        let attachment_stream_name = require_live_field(
            attachment.stream_name.as_deref(),
            "attachment metadata",
            "stream_name",
        )?;
        let attachment_vdo_id = require_live_field(
            attachment.vdo_id.as_deref(),
            "attachment metadata",
            "vdo_id",
        )?;
        let job_id = require_live_field(
            attachment.job_id.as_deref(),
            "attachment metadata",
            "job_id",
        )?;

        ensure_match(
            "live_id",
            &iframe_live_id,
            &attachment_live_id,
            "attachment metadata",
        )?;
        ensure_match(
            "stream_name",
            &iframe_stream_name,
            &attachment_stream_name,
            "attachment metadata",
        )?;
        ensure_match(
            "vdo_id",
            &iframe_vdo_id,
            &attachment_vdo_id,
            "attachment metadata",
        )?;

        Ok(Self {
            queue_index: entry.queue_index,
            chapter_id: entry.chapter_id,
            card_index: entry.card_index,
            point_index: entry.point_index,
            title: attachment
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| entry.title.clone()),
            live_id: iframe_live_id,
            job_id,
            stream_name: iframe_stream_name,
            vdo_id: iframe_vdo_id,
        })
    }
}

impl TryFrom<&CourseRunQueueEntry> for DocumentTaskExecutionContext {
    type Error = CpassError;

    fn try_from(entry: &CourseRunQueueEntry) -> Result<Self> {
        if entry.module != "insertdoc" || entry.task_kind != CourseRunTaskKind::Document {
            return Err(CpassError::Validation(format!(
                "queue entry {} is not a document task point",
                entry.queue_index
            )));
        }

        let metadata = entry.attachment_metadata.as_ref().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing document attachment metadata",
                entry.queue_index
            ))
        })?;
        let attachment = metadata.attachment.as_ref().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing document attachment details",
                entry.queue_index
            ))
        })?;

        if attachment.attachment_type.as_deref() != Some("document") {
            return Err(CpassError::Validation(format!(
                "queue entry {} attachment type is not document",
                entry.queue_index
            )));
        }
        if !attachment.is_job {
            return Err(CpassError::Validation(format!(
                "queue entry {} does not map to a legacy document job attachment",
                entry.queue_index
            )));
        }

        let object_id = match attachment.object_id.as_deref() {
            Some(object_id) if object_id == entry.resource_id => object_id.to_owned(),
            Some(object_id) => {
                return Err(CpassError::Validation(format!(
                    "queue entry {} object_id {} does not match resource_id {}",
                    entry.queue_index, object_id, entry.resource_id
                )));
            }
            None => entry.resource_id.clone(),
        };

        let job_id = attachment.job_id.clone().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing document job_id",
                entry.queue_index
            ))
        })?;
        let jtoken = attachment.jtoken.clone().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing document jtoken",
                entry.queue_index
            ))
        })?;

        Ok(Self {
            queue_index: entry.queue_index,
            chapter_id: entry.chapter_id,
            card_index: entry.card_index,
            point_index: entry.point_index,
            object_id,
            title: attachment
                .title
                .clone()
                .unwrap_or_else(|| entry.title.clone()),
            job_id,
            jtoken,
        })
    }
}

impl TryFrom<&CourseRunQueueEntry> for ChapterWorkTaskExecutionContext {
    type Error = CpassError;

    fn try_from(entry: &CourseRunQueueEntry) -> Result<Self> {
        if entry.module != "work" || entry.task_kind != CourseRunTaskKind::ChapterWork {
            return Err(CpassError::Validation(format!(
                "queue entry {} is not a chapter work task point",
                entry.queue_index
            )));
        }

        let metadata = entry.chapter_work_metadata.as_ref().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing chapter work metadata",
                entry.queue_index
            ))
        })?;

        if metadata.work_id != entry.resource_id {
            return Err(CpassError::Validation(format!(
                "queue entry {} work_id {} does not match resource_id {}",
                entry.queue_index, metadata.work_id, entry.resource_id
            )));
        }

        let job_id = metadata.job_id.clone().ok_or_else(|| {
            CpassError::Validation(format!(
                "queue entry {} is missing chapter work job_id",
                entry.queue_index
            ))
        })?;

        Ok(Self {
            queue_index: entry.queue_index,
            chapter_id: entry.chapter_id,
            card_index: entry.card_index,
            point_index: entry.point_index,
            title: entry.title.clone(),
            work_id: metadata.work_id.clone(),
            job_id,
            school_id: metadata.school_id.clone(),
        })
    }
}

impl VideoTaskExecutionContext {
    pub fn build_play_report_request(
        &self,
        course: &Course,
        account: &AccountProfile,
        playing_time_secs: u64,
        report_timestamp_millis: i64,
    ) -> Result<VideoPlayReportRequest> {
        if playing_time_secs > self.duration_secs {
            return Err(CpassError::Validation(format!(
                "video playing_time_secs {} exceeds duration {} for queue entry {}",
                playing_time_secs, self.duration_secs, self.queue_index
            )));
        }

        let clip_time = format!("0_{}", self.duration_secs);
        let enc_payload = format!(
            "[{}][{}][{}][{}][{}][{}][{}][{}]",
            course.class_id,
            account.puid,
            self.job_id,
            self.object_id,
            playing_time_secs * 1000,
            VIDEO_PLAY_REPORT_ENC_SALT,
            self.duration_secs * 1000,
            clip_time,
        );
        let enc = md5_hex(&enc_payload);

        Ok(VideoPlayReportRequest {
            cpi: course.cpi,
            class_id: course.class_id,
            user_id: account.puid,
            dtoken: self.dtoken.clone(),
            object_id: self.object_id.clone(),
            job_id: self.job_id.clone(),
            other_info: self.other_info.clone(),
            playing_time_secs,
            duration_secs: self.duration_secs,
            clip_time,
            enc,
            playback_rate: self.playback_rate.clone(),
            report_timestamp_millis,
        })
    }
}

impl DocumentTaskExecutionContext {
    #[must_use]
    pub fn build_reading_report_request(
        &self,
        course: &Course,
        report_timestamp_millis: i64,
    ) -> DocumentReadingReportRequest {
        DocumentReadingReportRequest {
            course_id: course.course_id,
            class_id: course.class_id,
            knowledge_id: self.chapter_id,
            job_id: self.job_id.clone(),
            jtoken: self.jtoken.clone(),
            report_timestamp_millis,
        }
    }
}

impl LiveTaskExecutionContext {
    #[must_use]
    pub fn build_progress_report_request(
        &self,
        course: &Course,
        account: &AccountProfile,
        is_start: bool,
        report_timestamp_millis: i64,
    ) -> LiveProgressReportRequest {
        LiveProgressReportRequest {
            stream_name: self.stream_name.clone(),
            vdo_id: self.vdo_id.clone(),
            user_id: account.puid,
            is_start,
            course_id: course.course_id,
            report_timestamp_millis,
        }
    }
}

impl ChapterWorkTaskExecutionContext {
    #[must_use]
    pub fn build_runtime_request(
        &self,
        course: &Course,
        account: &AccountProfile,
    ) -> ChapterWorkRuntimeRequest {
        ChapterWorkRuntimeRequest {
            course_id: course.course_id,
            class_id: course.class_id,
            knowledge_id: self.chapter_id,
            user_id: account.puid,
            cpi: course.cpi,
            card_index: self.card_index,
            work_id: self.work_id.clone(),
            job_id: self.job_id.clone(),
            school_id: self.school_id.clone(),
        }
    }
}

fn md5_hex(input: &str) -> String {
    let digest = md5_digest(input.as_bytes());
    let mut output = String::with_capacity(32);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("write to string");
    }
    output
}

fn md5_digest(input: &[u8]) -> [u8; 16] {
    let mut message = input.to_vec();
    let bit_len = (message.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());

    let mut a0 = 0x6745_2301_u32;
    let mut b0 = 0xefcd_ab89_u32;
    let mut c0 = 0x98ba_dcfe_u32;
    let mut d0 = 0x1032_5476_u32;

    for chunk in message.chunks_exact(64) {
        let mut words = [0_u32; 16];
        for (index, word) in words.iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_le_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }

        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);

        for round in 0..64 {
            let (f, g) = if round < 16 {
                ((b & c) | ((!b) & d), round)
            } else if round < 32 {
                ((d & b) | ((!d) & c), (5 * round + 1) % 16)
            } else if round < 48 {
                (b ^ c ^ d, (3 * round + 5) % 16)
            } else {
                (c ^ (b | !d), (7 * round) % 16)
            };

            let rotated = a
                .wrapping_add(f)
                .wrapping_add(MD5_TABLE[round])
                .wrapping_add(words[g])
                .rotate_left(MD5_SHIFT_AMOUNTS[round]);

            let next_a = d;
            let next_d = c;
            let next_c = b;
            let next_b = b.wrapping_add(rotated);
            (a, b, c, d) = (next_a, next_b, next_c, next_d);
        }

        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut digest = [0_u8; 16];
    digest[0..4].copy_from_slice(&a0.to_le_bytes());
    digest[4..8].copy_from_slice(&b0.to_le_bytes());
    digest[8..12].copy_from_slice(&c0.to_le_bytes());
    digest[12..16].copy_from_slice(&d0.to_le_bytes());
    digest
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CourseRunChapterPlan {
    pub chapter_id: u64,
    pub jobs: u64,
    pub chapter_index: u64,
    pub chapter_name: String,
    pub chapter_label: String,
    pub layer: u64,
    pub status: String,
    pub point_total: u64,
    pub point_finished: u64,
    pub task_points: Vec<CourseRunTaskPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CourseRunPlan {
    pub course: Course,
    pub chapters: Vec<CourseRunChapterPlan>,
    pub execution_queue: Vec<CourseRunQueueEntry>,
    pub total_task_points: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CourseRunExecutionPreflightEntry {
    pub queue_index: usize,
    pub resolution: TaskExecutorResolution,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CourseRunExecutionPreflight {
    pub all_entries_registered: bool,
    pub total_entries: usize,
    pub registered_entries: usize,
    pub blocked_entries: usize,
    pub queue: Vec<CourseRunExecutionPreflightEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CourseRunQueueExecutionResult {
    pub queue_index: usize,
    pub state: CourseRunQueueEntryState,
    pub resolution: TaskExecutorResolution,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CourseRunExecutionResult {
    pub course_id: u64,
    pub state: CourseRunExecutionState,
    pub total_entries: usize,
    pub completed_entries: usize,
    pub blocked_entries: usize,
    pub queue: Vec<CourseRunQueueExecutionResult>,
}

impl CourseRunExecutionResult {
    #[must_use]
    pub fn from_preflight(course_id: u64, preflight: &CourseRunExecutionPreflight) -> Self {
        Self {
            course_id,
            state: CourseRunExecutionState::Pending,
            total_entries: preflight.total_entries,
            completed_entries: 0,
            blocked_entries: 0,
            queue: preflight
                .queue
                .iter()
                .map(|entry| CourseRunQueueExecutionResult {
                    queue_index: entry.queue_index,
                    state: CourseRunQueueEntryState::Pending,
                    resolution: entry.resolution.clone(),
                })
                .collect(),
        }
    }

    pub fn start(&mut self) -> Result<()> {
        match self.state {
            CourseRunExecutionState::Pending => {
                self.state = CourseRunExecutionState::Running;
                Ok(())
            }
            state => Err(CpassError::Validation(format!(
                "course run execution cannot start while state is {}",
                state.as_str()
            ))),
        }
    }

    pub fn mark_queue_entry_running(&mut self, queue_index: usize) -> Result<()> {
        self.ensure_running("advance queue entry state")?;
        self.transition_queue_entry(queue_index, CourseRunQueueEntryState::Running)
    }

    pub fn mark_queue_entry_completed(&mut self, queue_index: usize) -> Result<()> {
        self.ensure_running("advance queue entry state")?;
        self.transition_queue_entry(queue_index, CourseRunQueueEntryState::Completed)
    }

    pub fn mark_queue_entry_blocked(&mut self, queue_index: usize) -> Result<()> {
        self.ensure_running("advance queue entry state")?;
        self.transition_queue_entry(queue_index, CourseRunQueueEntryState::Blocked)
    }

    pub fn finish(&mut self) -> Result<()> {
        self.ensure_running("finish")?;

        if self.blocked_entries > 0 {
            self.state = CourseRunExecutionState::Blocked;
            return Ok(());
        }

        if self.completed_entries == self.total_entries {
            self.state = CourseRunExecutionState::Completed;
            return Ok(());
        }

        Err(CpassError::Validation(
            "course run execution cannot finish while queue entries are still pending or running"
                .to_owned(),
        ))
    }

    fn ensure_running(&self, action: &str) -> Result<()> {
        match self.state {
            CourseRunExecutionState::Running => Ok(()),
            state => Err(CpassError::Validation(format!(
                "course run execution cannot {action} while state is {}",
                state.as_str()
            ))),
        }
    }

    fn transition_queue_entry(
        &mut self,
        queue_index: usize,
        next_state: CourseRunQueueEntryState,
    ) -> Result<()> {
        let entry = self
            .queue
            .iter_mut()
            .find(|entry| entry.queue_index == queue_index)
            .ok_or_else(|| {
                CpassError::Validation(format!(
                    "queue_index {queue_index} not found in course run execution result"
                ))
            })?;

        if !entry.state.can_transition_to(next_state) {
            return Err(CpassError::Validation(format!(
                "queue_index {queue_index} cannot transition from {} to {}",
                entry.state.as_str(),
                next_state.as_str()
            )));
        }

        entry.state = next_state;
        self.completed_entries = self
            .queue
            .iter()
            .filter(|entry| entry.state == CourseRunQueueEntryState::Completed)
            .count();
        self.blocked_entries = self
            .queue
            .iter()
            .filter(|entry| entry.state == CourseRunQueueEntryState::Blocked)
            .count();

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CourseRunQueueDriverOutcome {
    Completed,
    Blocked,
}

#[async_trait::async_trait]
pub trait CourseRunQueueEntryExecutor: Send + Sync {
    async fn execute_queue_entry(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome>;
}

#[derive(Default)]
pub struct FailClosedCourseRunExecutor {
    sink: Option<Arc<dyn RunEventSink>>,
}

impl FailClosedCourseRunExecutor {
    #[must_use]
    pub const fn new() -> Self {
        Self { sink: None }
    }

    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn emit(&self, event: RunEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }
}

#[async_trait::async_trait]
impl CourseRunQueueEntryExecutor for FailClosedCourseRunExecutor {
    async fn execute_queue_entry(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        self.emit(RunEvent::Warning {
            message: format!(
                "queue entry {} reached headless dispatch for executor {} ({}) but module-specific runtime support is not implemented yet; stopping fail-closed before any task endpoint is called",
                entry.queue_index,
                registration.key.as_str(),
                registration.module
            ),
        });
        Ok(CourseRunQueueDriverOutcome::Blocked)
    }
}

#[derive(Clone)]
pub struct HeadlessVideoCourseRunExecutor {
    client: ChaoxingClient,
    course: Course,
    account: AccountProfile,
    sink: Option<Arc<dyn RunEventSink>>,
}

impl HeadlessVideoCourseRunExecutor {
    #[must_use]
    pub fn new(client: ChaoxingClient, course: Course, account: AccountProfile) -> Self {
        Self {
            client,
            course,
            account,
            sink: None,
        }
    }

    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn emit(&self, event: RunEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }

    async fn fail_closed(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        let executor = match &self.sink {
            Some(sink) => FailClosedCourseRunExecutor::new().with_sink(sink.clone()),
            None => FailClosedCourseRunExecutor::new(),
        };
        executor.execute_queue_entry(entry, registration).await
    }
}

#[async_trait::async_trait]
impl CourseRunQueueEntryExecutor for HeadlessVideoCourseRunExecutor {
    async fn execute_queue_entry(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        if registration.key != TaskExecutorKey::Video || registration.module != "insertvideo" {
            return self.fail_closed(entry, registration).await;
        }

        let context = match entry.video_execution_context() {
            Ok(context) => context,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} could not build a video execution context: {error}; stopping fail-closed before retrying the play-report route",
                        entry.queue_index
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        let request = match context.build_play_report_request(
            &self.course,
            &self.account,
            context.duration_secs,
            chrono::Utc::now().timestamp_millis(),
        ) {
            Ok(request) => request,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} could not build a legacy video play-report request: {error}; stopping fail-closed before any task endpoint is retried",
                        entry.queue_index
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        let acknowledgement = match self.client.report_video_progress(&request).await {
            Ok(acknowledgement) => acknowledgement,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} video play-report request failed: {error}; stopping fail-closed after the first runtime acknowledgement attempt",
                        entry.queue_index
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        self.emit(RunEvent::CourseRunVideoProgressReported {
            course_id: self.course.course_id,
            queue_index: entry.queue_index,
            playing_time_secs: acknowledgement
                .playing_time_secs
                .unwrap_or(request.playing_time_secs),
            duration_secs: acknowledgement
                .duration_secs
                .unwrap_or(request.duration_secs),
            is_passed: acknowledgement.is_passed.unwrap_or(false),
        });

        if acknowledgement.is_passed == Some(true) {
            return Ok(CourseRunQueueDriverOutcome::Completed);
        }

        self.emit(RunEvent::Warning {
            message: format!(
                "queue entry {} video play-report acknowledgement did not mark the task as passed; stopping fail-closed before retrying or widening runtime support",
                entry.queue_index
            ),
        });
        Ok(CourseRunQueueDriverOutcome::Blocked)
    }
}

#[derive(Clone)]
pub struct HeadlessDocumentCourseRunExecutor {
    client: ChaoxingClient,
    course: Course,
    sink: Option<Arc<dyn RunEventSink>>,
}

impl HeadlessDocumentCourseRunExecutor {
    #[must_use]
    pub fn new(client: ChaoxingClient, course: Course) -> Self {
        Self {
            client,
            course,
            sink: None,
        }
    }

    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn emit(&self, event: RunEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }

    async fn fail_closed(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        let executor = match &self.sink {
            Some(sink) => FailClosedCourseRunExecutor::new().with_sink(sink.clone()),
            None => FailClosedCourseRunExecutor::new(),
        };
        executor.execute_queue_entry(entry, registration).await
    }
}

#[async_trait::async_trait]
impl CourseRunQueueEntryExecutor for HeadlessDocumentCourseRunExecutor {
    async fn execute_queue_entry(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        if registration.key != TaskExecutorKey::Document || registration.module != "insertdoc" {
            return self.fail_closed(entry, registration).await;
        }

        let context = match entry.document_execution_context() {
            Ok(context) => context,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} could not build a document execution context: {error}; stopping fail-closed before retrying the reading-report route",
                        entry.queue_index
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        let request = context
            .build_reading_report_request(&self.course, chrono::Utc::now().timestamp_millis());

        let acknowledgement = match self.client.report_document_progress(&request).await {
            Ok(acknowledgement) => acknowledgement,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} document reading-report request failed: {error}; stopping fail-closed after the first runtime acknowledgement attempt",
                        entry.queue_index
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        self.emit(RunEvent::CourseRunDocumentProgressReported {
            course_id: self.course.course_id,
            queue_index: entry.queue_index,
            success: acknowledgement.success,
        });

        if acknowledgement.success {
            return Ok(CourseRunQueueDriverOutcome::Completed);
        }

        self.emit(RunEvent::Warning {
            message: format!(
                "queue entry {} document reading-report acknowledgement did not confirm success; stopping fail-closed before retrying or widening runtime support",
                entry.queue_index
            ),
        });
        Ok(CourseRunQueueDriverOutcome::Blocked)
    }
}

#[derive(Clone)]
pub struct HeadlessLiveCourseRunExecutor {
    client: ChaoxingClient,
    course: Course,
    account: AccountProfile,
    sink: Option<Arc<dyn RunEventSink>>,
}

impl HeadlessLiveCourseRunExecutor {
    #[must_use]
    pub fn new(client: ChaoxingClient, course: Course, account: AccountProfile) -> Self {
        Self {
            client,
            course,
            account,
            sink: None,
        }
    }

    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn emit(&self, event: RunEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }

    async fn fail_closed(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        let executor = match &self.sink {
            Some(sink) => FailClosedCourseRunExecutor::new().with_sink(sink.clone()),
            None => FailClosedCourseRunExecutor::new(),
        };
        executor.execute_queue_entry(entry, registration).await
    }
}

#[async_trait::async_trait]
impl CourseRunQueueEntryExecutor for HeadlessLiveCourseRunExecutor {
    async fn execute_queue_entry(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        if registration.key != TaskExecutorKey::Live || registration.module != "insertlive" {
            return self.fail_closed(entry, registration).await;
        }

        let context = match entry.live_execution_context() {
            Ok(context) => context,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} could not build a live execution context: {error}; stopping fail-closed before the reviewed live progress-report route is called",
                        entry.queue_index
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        let request = context.build_progress_report_request(
            &self.course,
            &self.account,
            true,
            chrono::Utc::now().timestamp_millis(),
        );

        let acknowledgement = match self.client.report_live_progress(&request).await {
            Ok(acknowledgement) => acknowledgement,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} live progress-report request failed: {error}; stopping fail-closed after the first reviewed runtime acknowledgement attempt",
                        entry.queue_index
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        self.emit(RunEvent::CourseRunLiveProgressReported {
            course_id: self.course.course_id,
            queue_index: entry.queue_index,
            success: acknowledgement.success,
        });

        if !acknowledgement.success {
            self.emit(RunEvent::Warning {
                message: format!(
                    "queue entry {} live progress-report acknowledgement did not confirm success; stopping fail-closed before retrying or widening runtime support",
                    entry.queue_index
                ),
            });
            return Ok(CourseRunQueueDriverOutcome::Blocked);
        }

        self.emit(RunEvent::Warning {
            message: format!(
                "queue entry {} acknowledged the reviewed live progress-report route for {} and stopped fail-closed before attendance, completion, or any additional live runtime endpoints are called",
                entry.queue_index, context.live_id
            ),
        });
        Ok(CourseRunQueueDriverOutcome::Blocked)
    }
}

#[derive(Clone)]
pub struct HeadlessChapterWorkCourseRunExecutor {
    client: ChaoxingClient,
    course: Course,
    account: AccountProfile,
    searcher_pipeline: Option<Arc<SearcherPipeline>>,
    sink: Option<Arc<dyn RunEventSink>>,
}

impl HeadlessChapterWorkCourseRunExecutor {
    #[must_use]
    pub fn new(client: ChaoxingClient, course: Course, account: AccountProfile) -> Self {
        Self {
            client,
            course,
            account,
            searcher_pipeline: None,
            sink: None,
        }
    }

    #[must_use]
    pub fn with_searcher_pipeline(mut self, searcher_pipeline: Arc<SearcherPipeline>) -> Self {
        self.searcher_pipeline = Some(searcher_pipeline);
        self
    }

    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn emit(&self, event: RunEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }

    async fn select_candidates(
        &self,
        entry: &CourseRunQueueEntry,
        snapshot: &crate::models::ChapterWorkFormSnapshot,
    ) -> Result<Option<ChapterWorkCandidateSelectionBatch>> {
        let Some(searcher_pipeline) = &self.searcher_pipeline else {
            return Ok(None);
        };
        let query_batch = ChapterWorkQueryBatch::from(snapshot);
        let selection = searcher_pipeline
            .select_chapter_work_candidates(&query_batch)
            .await?;

        self.emit(RunEvent::CourseRunChapterWorkCandidateSelectionPrepared {
            course_id: self.course.course_id,
            queue_index: entry.queue_index,
            work_answer_id: selection.work_answer_id,
            total_questions: selection.total_question_num,
            selected_questions: selection.selected_count(),
        });
        Ok(Some(selection))
    }

    async fn fail_closed(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        let executor = match &self.sink {
            Some(sink) => FailClosedCourseRunExecutor::new().with_sink(sink.clone()),
            None => FailClosedCourseRunExecutor::new(),
        };
        executor.execute_queue_entry(entry, registration).await
    }
}

#[async_trait::async_trait]
impl CourseRunQueueEntryExecutor for HeadlessChapterWorkCourseRunExecutor {
    async fn execute_queue_entry(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        if registration.key != TaskExecutorKey::ChapterWork || registration.module != "work" {
            return self.fail_closed(entry, registration).await;
        }

        let context = match entry.chapter_work_execution_context() {
            Ok(context) => context,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} could not build a chapter-work execution context: {error}; stopping fail-closed before the runtime work page is fetched",
                        entry.queue_index
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        let request = context.build_runtime_request(&self.course, &self.account);
        let snapshot = match self.client.fetch_chapter_work_form(&request).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} chapter-work runtime snapshot fetch failed: {error}; stopping fail-closed before any answer save or submit endpoint is called",
                        entry.queue_index
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        self.emit(RunEvent::CourseRunChapterWorkSnapshotFetched {
            course_id: self.course.course_id,
            queue_index: entry.queue_index,
            snapshot: snapshot.clone(),
        });
        let candidate_selection = match self.select_candidates(entry, &snapshot).await {
            Ok(candidate_selection) => candidate_selection,
            Err(error) => {
                self.emit(RunEvent::Warning {
                    message: format!(
                        "queue entry {} chapter-work candidate selection failed for snapshot {}: {error}; stopping fail-closed before any answer save or submit endpoint is called",
                        entry.queue_index, snapshot.work_answer_id
                    ),
                });
                return Ok(CourseRunQueueDriverOutcome::Blocked);
            }
        };

        let warning_message = if let Some(selection) = candidate_selection {
            format!(
                "queue entry {} prepared chapter-work candidate selections for {}/{} questions from snapshot {} and stopped fail-closed before any answer save or submit endpoint is called",
                entry.queue_index,
                selection.selected_count(),
                selection.total_question_num,
                selection.work_answer_id
            )
        } else {
            format!(
                "queue entry {} fetched chapter-work runtime snapshot {} with {} questions and stopped fail-closed before any answer save or submit endpoint is called",
                entry.queue_index,
                snapshot.work_answer_id,
                snapshot.questions.len()
            )
        };
        self.emit(RunEvent::Warning {
            message: warning_message,
        });
        Ok(CourseRunQueueDriverOutcome::Blocked)
    }
}

pub struct CourseRunHeadlessDriver<'a> {
    registry: &'a TaskExecutorRegistry,
    sink: Option<Arc<dyn RunEventSink>>,
}

impl<'a> CourseRunHeadlessDriver<'a> {
    #[must_use]
    pub const fn new(registry: &'a TaskExecutorRegistry) -> Self {
        Self {
            registry,
            sink: None,
        }
    }

    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    pub async fn drive<E>(
        &self,
        plan: &CourseRunPlan,
        executor: &E,
    ) -> Result<CourseRunExecutionResult>
    where
        E: CourseRunQueueEntryExecutor + ?Sized,
    {
        let preflight = plan.build_execution_preflight(self.registry);
        let mut result =
            CourseRunExecutionResult::from_preflight(plan.course.course_id, &preflight);
        result.start()?;
        self.emit(RunEvent::CourseRunExecutionStarted {
            course_id: result.course_id,
            total_entries: result.total_entries,
        });

        for resolved in plan.iter_resolved_execution_queue(self.registry) {
            match &resolved.resolution {
                TaskExecutorResolution::Registered { registration } => {
                    self.transition_queue_entry(
                        &mut result,
                        resolved.entry.queue_index,
                        CourseRunQueueEntryState::Running,
                    )?;

                    let outcome = executor
                        .execute_queue_entry(resolved.entry, *registration)
                        .await?;
                    let state = match outcome {
                        CourseRunQueueDriverOutcome::Completed => {
                            CourseRunQueueEntryState::Completed
                        }
                        CourseRunQueueDriverOutcome::Blocked => CourseRunQueueEntryState::Blocked,
                    };
                    self.transition_queue_entry(&mut result, resolved.entry.queue_index, state)?;

                    if state == CourseRunQueueEntryState::Blocked {
                        break;
                    }
                }
                TaskExecutorResolution::UnsupportedModule { .. }
                | TaskExecutorResolution::InconsistentTaskKind { .. } => {
                    self.transition_queue_entry(
                        &mut result,
                        resolved.entry.queue_index,
                        CourseRunQueueEntryState::Blocked,
                    )?;
                    break;
                }
            }
        }

        result.finish()?;
        self.emit(RunEvent::CourseRunExecutionFinished {
            course_id: result.course_id,
            state: result.state,
            total_entries: result.total_entries,
            completed_entries: result.completed_entries,
            blocked_entries: result.blocked_entries,
        });
        Ok(result)
    }

    fn transition_queue_entry(
        &self,
        result: &mut CourseRunExecutionResult,
        queue_index: usize,
        state: CourseRunQueueEntryState,
    ) -> Result<()> {
        match state {
            CourseRunQueueEntryState::Pending => {
                return Err(CpassError::Validation(
                    "headless queue driver cannot transition entries back to pending".to_owned(),
                ));
            }
            CourseRunQueueEntryState::Running => result.mark_queue_entry_running(queue_index)?,
            CourseRunQueueEntryState::Completed => {
                result.mark_queue_entry_completed(queue_index)?
            }
            CourseRunQueueEntryState::Blocked => result.mark_queue_entry_blocked(queue_index)?,
        }

        self.emit(RunEvent::CourseRunQueueEntryStateChanged {
            course_id: result.course_id,
            queue_index,
            state,
        });
        Ok(())
    }

    fn emit(&self, event: RunEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }
}

impl CourseRunQueueEntryState {
    const fn can_transition_to(self, next_state: Self) -> bool {
        matches!(
            (self, next_state),
            (Self::Pending, Self::Running)
                | (Self::Pending, Self::Blocked)
                | (Self::Running, Self::Completed)
                | (Self::Running, Self::Blocked)
        )
    }
}

impl CourseRunPlan {
    #[must_use]
    pub fn from_scanned_chapters(course: Course, chapters: Vec<Chapter>) -> Self {
        let mut chapter_plans = chapters
            .into_iter()
            .map(|chapter| {
                let Chapter {
                    chapter_id,
                    jobs,
                    index,
                    name,
                    label,
                    layer,
                    status,
                    point_total,
                    point_finished,
                    mut task_points,
                } = chapter;
                task_points.sort_by(|left, right| {
                    (
                        left.card_index,
                        left.point_index,
                        left.module.as_str(),
                        left.resource_id.as_str(),
                    )
                        .cmp(&(
                            right.card_index,
                            right.point_index,
                            right.module.as_str(),
                            right.resource_id.as_str(),
                        ))
                });

                CourseRunChapterPlan {
                    chapter_id,
                    jobs,
                    chapter_index: index,
                    chapter_name: name,
                    chapter_label: label,
                    layer,
                    status,
                    point_total,
                    point_finished,
                    task_points: task_points
                        .into_iter()
                        .map(CourseRunTaskPoint::from)
                        .collect(),
                }
            })
            .collect::<Vec<_>>();

        chapter_plans.sort_by(|left, right| {
            (
                left.chapter_index,
                left.chapter_id,
                left.chapter_label.as_str(),
                left.chapter_name.as_str(),
            )
                .cmp(&(
                    right.chapter_index,
                    right.chapter_id,
                    right.chapter_label.as_str(),
                    right.chapter_name.as_str(),
                ))
        });

        let execution_queue = chapter_plans
            .iter()
            .flat_map(|chapter| {
                chapter
                    .task_points
                    .iter()
                    .map(move |task_point| (chapter, task_point))
            })
            .enumerate()
            .map(|(queue_index, (chapter, task_point))| CourseRunQueueEntry {
                queue_index,
                chapter_id: chapter.chapter_id,
                chapter_index: chapter.chapter_index,
                chapter_name: chapter.chapter_name.clone(),
                chapter_label: chapter.chapter_label.clone(),
                card_index: task_point.card_index,
                point_index: task_point.point_index,
                module: task_point.module.clone(),
                task_kind: task_point.task_kind.clone(),
                title: task_point.title.clone(),
                resource_id: task_point.resource_id.clone(),
                iframe_data: task_point.iframe_data.clone(),
                attachment_metadata: task_point.attachment_metadata.clone(),
                chapter_work_metadata: task_point.chapter_work_metadata.clone(),
            })
            .collect::<Vec<_>>();

        let total_task_points = execution_queue.len();

        Self {
            course,
            chapters: chapter_plans,
            execution_queue,
            total_task_points,
        }
    }

    pub fn iter_task_points(&self) -> impl Iterator<Item = &CourseRunTaskPoint> {
        self.chapters
            .iter()
            .flat_map(|chapter| chapter.task_points.iter())
    }

    pub fn iter_execution_queue(&self) -> impl Iterator<Item = &CourseRunQueueEntry> {
        self.execution_queue.iter()
    }

    pub fn iter_resolved_execution_queue<'a>(
        &'a self,
        registry: &'a TaskExecutorRegistry,
    ) -> impl Iterator<Item = TaskExecutorQueueResolution<'a>> + 'a {
        self.execution_queue
            .iter()
            .map(move |entry| TaskExecutorQueueResolution {
                entry,
                resolution: registry.resolve(entry),
            })
    }

    pub fn build_execution_preflight(
        &self,
        registry: &TaskExecutorRegistry,
    ) -> CourseRunExecutionPreflight {
        let queue = self
            .iter_resolved_execution_queue(registry)
            .map(|resolved| CourseRunExecutionPreflightEntry {
                queue_index: resolved.entry.queue_index,
                resolution: resolved.resolution.clone(),
            })
            .collect::<Vec<_>>();
        let total_entries = queue.len();
        let registered_entries = queue
            .iter()
            .filter(|entry| matches!(entry.resolution, TaskExecutorResolution::Registered { .. }))
            .count();
        let blocked_entries = total_entries.saturating_sub(registered_entries);

        CourseRunExecutionPreflight {
            all_entries_registered: blocked_entries == 0,
            total_entries,
            registered_entries,
            blocked_entries,
            queue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExamRunTarget {
    pub course_id: Option<u64>,
    pub course_index: Option<usize>,
    pub exam_id: Option<u64>,
    pub exam_index: Option<usize>,
}

impl ExamRunTarget {
    pub fn new(
        course_id: Option<u64>,
        course_index: Option<usize>,
        exam_id: Option<u64>,
        exam_index: Option<usize>,
    ) -> Result<Self> {
        CourseRunTarget::new(course_id, course_index)?;

        match (exam_id, exam_index) {
            (Some(_), Some(_)) => Err(CpassError::Validation(
                "only one of --exam-id or --exam-index may be set".to_owned(),
            )),
            (None, None) => Err(CpassError::Validation(
                "one of --exam-id or --exam-index is required".to_owned(),
            )),
            _ => Ok(Self {
                course_id,
                course_index,
                exam_id,
                exam_index,
            }),
        }
    }

    pub fn resolve_course(self, courses: Vec<Course>) -> Result<Course> {
        CourseRunTarget {
            course_id: self.course_id,
            course_index: self.course_index,
        }
        .resolve_course(courses)
    }

    pub fn resolve_exam(self, exams: Vec<CourseExam>) -> Result<CourseExam> {
        if let Some(exam_id) = self.exam_id {
            return exams
                .into_iter()
                .find(|exam| exam.exam_id == exam_id)
                .ok_or_else(|| CpassError::Validation(format!("exam_id {exam_id} not found")));
        }

        if let Some(exam_index) = self.exam_index {
            return exams.into_iter().nth(exam_index).ok_or_else(|| {
                CpassError::Validation(format!("exam index {exam_index} not found"))
            });
        }

        Err(CpassError::Validation(
            "one of --exam-id or --exam-index is required".to_owned(),
        ))
    }
}

pub type ExamRunQuestionKind = NormalizedQuestionKind;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExamRunQuestionPlan {
    pub question_index: usize,
    pub question_id: u64,
    pub question_type: u64,
    pub question_type_label: String,
    pub question_kind: ExamRunQuestionKind,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ExamQuestionOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blanks: Vec<String>,
}

impl From<ExamQuestionSummary> for ExamRunQuestionPlan {
    fn from(value: ExamQuestionSummary) -> Self {
        Self {
            question_index: value.question_index,
            question_id: value.question_id,
            question_type: value.question_type,
            question_type_label: value.question_type_label,
            question_kind: ExamRunQuestionKind::from_question_type(value.question_type),
            prompt: value.prompt,
            options: value.options,
            blanks: value.blanks,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExamRunPlan {
    pub course: Course,
    pub exam: CourseExam,
    pub preview: ExamPreviewQuery,
    pub questions: Vec<ExamRunQuestionPlan>,
    pub total_questions: usize,
}

impl ExamRunPlan {
    #[must_use]
    pub fn from_preview_snapshot(
        course: Course,
        exam: CourseExam,
        preview: ExamPreviewQuery,
        questions: Vec<ExamQuestionSummary>,
    ) -> Self {
        let mut questions = questions
            .into_iter()
            .map(ExamRunQuestionPlan::from)
            .collect::<Vec<_>>();

        questions.sort_by(|left, right| {
            (
                left.question_index,
                left.question_id,
                left.question_type,
                left.question_type_label.as_str(),
                left.prompt.as_str(),
            )
                .cmp(&(
                    right.question_index,
                    right.question_id,
                    right.question_type,
                    right.question_type_label.as_str(),
                    right.prompt.as_str(),
                ))
        });

        let total_questions = questions.len();

        Self {
            course,
            exam,
            preview,
            questions,
            total_questions,
        }
    }

    pub fn iter_questions(&self) -> impl Iterator<Item = &ExamRunQuestionPlan> {
        self.questions.iter()
    }
}

pub struct ExamRunner<'a> {
    client: &'a ChaoxingClient,
    sink: Option<Arc<dyn RunEventSink>>,
}

impl<'a> ExamRunner<'a> {
    #[must_use]
    pub fn new(client: &'a ChaoxingClient) -> Self {
        Self { client, sink: None }
    }

    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    pub async fn build_plan(
        &self,
        account: &AccountProfile,
        target: ExamRunTarget,
    ) -> Result<ExamRunPlan> {
        self.emit(RunEvent::ExamRunPlanningStarted {
            course_id: target.course_id,
            course_index: target.course_index,
            exam_id: target.exam_id,
            exam_index: target.exam_index,
        });
        let course = target.resolve_course(self.client.fetch_courses().await?)?;
        let exams = self.client.fetch_exams(&course).await?;
        let mut exam = target.resolve_exam(exams)?;
        let meta = self.client.fetch_exam_meta(&course, account, &exam).await?;
        let preview = ExamPreviewQuery::from_exam_meta(&meta).ok_or_else(|| {
            CpassError::Validation(
                "exam run planning requires exam_answer_id from read-only exam cover metadata"
                    .to_owned(),
            )
        })?;
        exam.meta = Some(meta);
        let questions = self
            .client
            .fetch_exam_preview_questions(&course, &exam, &preview)
            .await?;

        let plan = ExamRunPlan::from_preview_snapshot(course, exam, preview, questions);
        self.emit(RunEvent::ExamRunPlanningFinished {
            course_id: plan.course.course_id,
            exam_id: plan.exam.exam_id,
            questions: plan.total_questions,
        });
        Ok(plan)
    }

    fn emit(&self, event: RunEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::{
        CourseRunExecutionPreflight, CourseRunExecutionPreflightEntry, CourseRunExecutionResult,
        CourseRunHeadlessDriver, CourseRunPlan, CourseRunQueueDriverOutcome, CourseRunQueueEntry,
        CourseRunQueueEntryExecutor, CourseRunTarget, CourseRunTaskKind, CourseRunner, ExamRunPlan,
        ExamRunQuestionKind, ExamRunTarget, ExamRunner, FailClosedCourseRunExecutor,
    };
    use crate::chaoxing::ChaoxingClient;
    use crate::event::{CourseRunExecutionState, CourseRunQueueEntryState, RunEvent, RunEventSink};
    use crate::models::{
        AccountProfile, Chapter, ChapterWorkMetadata, Course, CourseExam, ExamMeta,
        ExamPreviewQuery, ExamQuestionOption, ExamQuestionSummary, TaskAttachment,
        TaskPointAttachmentMetadata, TaskPointSummary, VideoAttachmentStatus,
    };
    use crate::question_kind::NormalizedQuestionKind;
    use crate::searcher::{AnswerCandidate, AnswerQuery, SearcherPipeline, SearcherProvider};
    use crate::task_executor::{
        TaskExecutorKey, TaskExecutorRegistration, TaskExecutorRegistry, TaskExecutorResolution,
    };
    use crate::transport::FixtureChaoxingTransport;
    use crate::{
        HeadlessChapterWorkCourseRunExecutor, HeadlessDocumentCourseRunExecutor,
        HeadlessLiveCourseRunExecutor, HeadlessVideoCourseRunExecutor,
    };

    #[derive(Default, Clone)]
    struct TestRunEventSink {
        events: Arc<Mutex<Vec<RunEvent>>>,
    }

    impl TestRunEventSink {
        fn snapshot(&self) -> Vec<RunEvent> {
            self.events.lock().expect("event buffer poisoned").clone()
        }
    }

    impl RunEventSink for TestRunEventSink {
        fn emit(&self, event: RunEvent) {
            self.events
                .lock()
                .expect("event buffer poisoned")
                .push(event);
        }
    }

    struct TestSearcherProvider {
        provider: &'static str,
        answers: Vec<&'static str>,
    }

    #[async_trait::async_trait]
    impl SearcherProvider for TestSearcherProvider {
        async fn search(&self, query: &AnswerQuery) -> crate::Result<Vec<AnswerCandidate>> {
            Ok(self
                .answers
                .iter()
                .map(|answer| AnswerCandidate {
                    provider: format!("{}:{}", self.provider, query.question_id),
                    confidence: None,
                    answer: (*answer).to_owned(),
                })
                .collect())
        }
    }

    #[derive(Clone)]
    struct TestQueueExecutor {
        calls: Arc<Mutex<Vec<(usize, TaskExecutorKey)>>>,
        blocked_at: Option<usize>,
    }

    impl Default for TestQueueExecutor {
        fn default() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                blocked_at: None,
            }
        }
    }

    impl TestQueueExecutor {
        fn with_blocked_at(queue_index: usize) -> Self {
            Self {
                blocked_at: Some(queue_index),
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<(usize, TaskExecutorKey)> {
            self.calls.lock().expect("call buffer poisoned").clone()
        }
    }

    #[async_trait::async_trait]
    impl CourseRunQueueEntryExecutor for TestQueueExecutor {
        async fn execute_queue_entry(
            &self,
            entry: &CourseRunQueueEntry,
            registration: crate::task_executor::TaskExecutorRegistration,
        ) -> crate::Result<CourseRunQueueDriverOutcome> {
            self.calls
                .lock()
                .expect("call buffer poisoned")
                .push((entry.queue_index, registration.key));

            Ok(if self.blocked_at == Some(entry.queue_index) {
                CourseRunQueueDriverOutcome::Blocked
            } else {
                CourseRunQueueDriverOutcome::Completed
            })
        }
    }

    fn sample_course() -> Course {
        Course {
            course_id: 1001,
            class_id: 2001,
            cpi: 3001,
            key: 4001,
            name: "Rust Automation".to_owned(),
            teacher_name: "Tester".to_owned(),
            state: "ongoing".to_owned(),
        }
    }

    fn sample_chapter(
        chapter_id: u64,
        chapter_index: u64,
        chapter_label: &str,
        chapter_name: &str,
        task_points: Vec<TaskPointSummary>,
    ) -> Chapter {
        Chapter {
            chapter_id,
            jobs: task_points.len() as u64,
            index: chapter_index,
            name: chapter_name.to_owned(),
            label: chapter_label.to_owned(),
            layer: 1,
            status: "open".to_owned(),
            point_total: task_points.len() as u64,
            point_finished: 0,
            task_points,
        }
    }

    fn sample_task_point(
        card_index: usize,
        point_index: usize,
        module: &str,
        title: &str,
        resource_id: &str,
    ) -> TaskPointSummary {
        TaskPointSummary {
            card_index,
            point_index,
            module: module.to_owned(),
            title: title.to_owned(),
            resource_id: resource_id.to_owned(),
            iframe_data: None,
            attachment_metadata: None,
            chapter_work_metadata: None,
        }
    }

    fn sample_video_attachment_metadata() -> TaskPointAttachmentMetadata {
        TaskPointAttachmentMetadata {
            fid: Some(999),
            attachment: Some(TaskAttachment {
                attachment_type: Some("video".to_owned()),
                object_id: Some("video-001".to_owned()),
                title: Some("Intro video".to_owned()),
                file_type: None,
                work_id: None,
                stream_name: None,
                vdo_id: None,
                live_id: None,
                job_id: Some("job-001".to_owned()),
                jtoken: Some("token-001".to_owned()),
                enc: None,
                other_info: Some("nodeId_11".to_owned()),
                playback_rate: Some("0.9".to_owned()),
                is_passed: Some(false),
                is_job: true,
            }),
            video_status: Some(VideoAttachmentStatus {
                status: "active".to_owned(),
                filename: "intro.mp4".to_owned(),
                duration_secs: 602,
                dtoken: Some("dtoken-001".to_owned()),
            }),
        }
    }

    fn sample_document_attachment_metadata() -> TaskPointAttachmentMetadata {
        TaskPointAttachmentMetadata {
            fid: None,
            attachment: Some(TaskAttachment {
                attachment_type: Some("document".to_owned()),
                object_id: Some("doc-001".to_owned()),
                title: Some("词汇材料".to_owned()),
                file_type: Some("pdf".to_owned()),
                work_id: None,
                stream_name: None,
                vdo_id: None,
                live_id: None,
                job_id: Some("job-doc-001".to_owned()),
                jtoken: Some("jtoken-doc-001".to_owned()),
                enc: None,
                other_info: None,
                playback_rate: None,
                is_passed: Some(false),
                is_job: true,
            }),
            video_status: None,
        }
    }

    fn sample_live_attachment_metadata() -> TaskPointAttachmentMetadata {
        TaskPointAttachmentMetadata {
            fid: None,
            attachment: Some(TaskAttachment {
                attachment_type: Some("live".to_owned()),
                object_id: None,
                title: Some("直播任务点".to_owned()),
                file_type: None,
                work_id: None,
                stream_name: Some("zhibo_12345".to_owned()),
                vdo_id: Some("vdo-live-001".to_owned()),
                live_id: Some("live-course-001".to_owned()),
                job_id: Some("job-live-001".to_owned()),
                jtoken: None,
                enc: None,
                other_info: None,
                playback_rate: None,
                is_passed: Some(false),
                is_job: true,
            }),
            video_status: None,
        }
    }

    fn sample_chapter_work_metadata() -> ChapterWorkMetadata {
        ChapterWorkMetadata {
            work_id: "work-001".to_owned(),
            job_id: Some("job-work-001".to_owned()),
            school_id: Some("school-001".to_owned()),
        }
    }

    fn sample_registered_execution_plan() -> CourseRunPlan {
        CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![
                sample_chapter(
                    22,
                    2,
                    "2",
                    "Follow-up",
                    vec![
                        sample_task_point(1, 1, "work", "Quiz", "work-001"),
                        sample_task_point(0, 0, "insertdoc", "Guide", "doc-001"),
                    ],
                ),
                sample_chapter(
                    11,
                    1,
                    "1",
                    "Warmup",
                    vec![sample_task_point(
                        0,
                        0,
                        "insertvideo",
                        "Intro video",
                        "video-001",
                    )],
                ),
            ],
        )
    }

    fn sample_registered_execution_preflight() -> CourseRunExecutionPreflight {
        sample_registered_execution_plan().build_execution_preflight(&TaskExecutorRegistry::new())
    }

    fn sample_fail_closed_execution_plan() -> CourseRunPlan {
        CourseRunPlan {
            course: sample_course(),
            chapters: vec![],
            execution_queue: vec![
                CourseRunQueueEntry {
                    queue_index: 0,
                    chapter_id: 11,
                    chapter_index: 1,
                    chapter_name: "Warmup".to_owned(),
                    chapter_label: "1".to_owned(),
                    card_index: 0,
                    point_index: 0,
                    module: "insertvideo".to_owned(),
                    task_kind: CourseRunTaskKind::Video,
                    title: "Intro video".to_owned(),
                    resource_id: "video-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                },
                CourseRunQueueEntry {
                    queue_index: 1,
                    chapter_id: 11,
                    chapter_index: 1,
                    chapter_name: "Warmup".to_owned(),
                    chapter_label: "1".to_owned(),
                    card_index: 1,
                    point_index: 0,
                    module: "insertvideo".to_owned(),
                    task_kind: CourseRunTaskKind::Document,
                    title: "Broken task".to_owned(),
                    resource_id: "broken-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                },
                CourseRunQueueEntry {
                    queue_index: 2,
                    chapter_id: 12,
                    chapter_index: 2,
                    chapter_name: "Follow-up".to_owned(),
                    chapter_label: "2".to_owned(),
                    card_index: 0,
                    point_index: 0,
                    module: "insertaudio".to_owned(),
                    task_kind: CourseRunTaskKind::Unknown("insertaudio".to_owned()),
                    title: "Unsupported task".to_owned(),
                    resource_id: "audio-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                },
            ],
            total_task_points: 3,
        }
    }

    fn sample_blocked_execution_preflight() -> CourseRunExecutionPreflight {
        sample_fail_closed_execution_plan().build_execution_preflight(&TaskExecutorRegistry::new())
    }

    fn sample_exam() -> CourseExam {
        CourseExam {
            exam_id: 555001,
            course_id: 1001,
            class_id: 2001,
            cpi: 3001,
            enc_task: "enc-task-001".to_owned(),
            name: "Chapter Exam".to_owned(),
            status: "ongoing".to_owned(),
            expire_time: Some("2026-04-10 18:00".to_owned()),
            meta: Some(ExamMeta {
                entry_state: "previewable".to_owned(),
                title: Some("Chapter Exam".to_owned()),
                exam_answer_id: Some(777001),
                monitor_enc: Some("monitor-enc".to_owned()),
                need_code: false,
                need_face: false,
                need_captcha: false,
                captcha_id: None,
                blocked_message: None,
            }),
        }
    }

    fn sample_account() -> AccountProfile {
        AccountProfile {
            puid: 114514,
            name: "Legacy User".to_owned(),
            phone: "13800138000".to_owned(),
            school: "示例大学".to_owned(),
            sex: Some("male".to_owned()),
            student_id: Some("20240001".to_owned()),
        }
    }

    fn sample_preview_query() -> ExamPreviewQuery {
        ExamPreviewQuery {
            exam_answer_id: 777001,
            enc: "preview-enc".to_owned(),
            remain_time_param: Some(1800),
            relation_answer_last_update_time: Some(1_712_345_678),
        }
    }

    fn sample_question(
        question_index: usize,
        question_id: u64,
        question_type: u64,
        question_type_label: &str,
        prompt: &str,
        options: Vec<ExamQuestionOption>,
        blanks: Vec<&str>,
    ) -> ExamQuestionSummary {
        ExamQuestionSummary {
            question_index,
            question_id,
            question_type,
            question_type_label: question_type_label.to_owned(),
            question_kind: NormalizedQuestionKind::from_question_type(question_type)
                .as_str()
                .to_owned(),
            prompt: prompt.to_owned(),
            options,
            blanks: blanks.into_iter().map(str::to_owned).collect(),
        }
    }

    #[test]
    fn builds_a_deterministic_course_run_plan() {
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![
                sample_chapter(
                    22,
                    2,
                    "2",
                    "Follow-up",
                    vec![
                        sample_task_point(1, 1, "work", "Quiz", "work-001"),
                        sample_task_point(0, 0, "insertdoc", "Guide", "doc-001"),
                    ],
                ),
                sample_chapter(
                    11,
                    1,
                    "1",
                    "Warmup",
                    vec![sample_task_point(
                        0,
                        0,
                        "insertvideo",
                        "Intro video",
                        "video-001",
                    )],
                ),
            ],
        );

        assert_eq!(plan.total_task_points, 3);
        assert_eq!(plan.chapters.len(), 2);
        assert_eq!(plan.chapters[0].chapter_id, 11);
        assert_eq!(plan.chapters[1].chapter_id, 22);
        assert_eq!(plan.chapters[1].task_points.len(), 2);
        assert_eq!(plan.chapters[1].task_points[0].module, "insertdoc");
        assert_eq!(
            plan.chapters[1].task_points[0].task_kind,
            CourseRunTaskKind::Document
        );
        assert_eq!(plan.chapters[1].task_points[1].module, "work");
        assert_eq!(
            plan.chapters[1].task_points[1].task_kind,
            CourseRunTaskKind::ChapterWork
        );

        let ordered_modules = plan
            .iter_task_points()
            .map(|task_point| task_point.module.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ordered_modules, vec!["insertvideo", "insertdoc", "work"]);
    }

    #[test]
    fn preserves_unknown_modules_for_future_executor_support() {
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(
                33,
                3,
                "3",
                "Future",
                vec![sample_task_point(
                    0,
                    0,
                    "insertaudio",
                    "Audio practice",
                    "audio-001",
                )],
            )],
        );

        assert_eq!(plan.total_task_points, 1);
        assert_eq!(
            plan.chapters[0].task_points[0].task_kind,
            CourseRunTaskKind::Unknown("insertaudio".to_owned())
        );
    }

    #[test]
    fn preserves_raw_iframe_data_for_future_module_support() {
        let mut task_point = sample_task_point(0, 0, "insertlive", "Live session", "live-001");
        task_point.iframe_data = Some(
            "{\"liveid\":\"live-001\",\"jobid\":\"job-live-001\",\"title\":\"直播任务\"}"
                .to_owned(),
        );

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(33, 3, "3", "Live", vec![task_point])],
        );

        assert_eq!(
            plan.chapters[0].task_points[0].iframe_data.as_deref(),
            Some("{\"liveid\":\"live-001\",\"jobid\":\"job-live-001\",\"title\":\"直播任务\"}")
        );
        assert_eq!(
            plan.execution_queue[0].iframe_data.as_deref(),
            Some("{\"liveid\":\"live-001\",\"jobid\":\"job-live-001\",\"title\":\"直播任务\"}")
        );
    }

    #[test]
    fn preserves_safe_attachment_metadata_for_future_executors() {
        let mut task_point = sample_task_point(0, 0, "insertvideo", "Intro video", "video-001");
        task_point.attachment_metadata = Some(sample_video_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(11, 1, "1", "Warmup", vec![task_point])],
        );

        let attachment_metadata = plan.chapters[0].task_points[0]
            .attachment_metadata
            .as_ref()
            .expect("attachment metadata to be preserved");
        assert_eq!(attachment_metadata.fid, Some(999));
        assert_eq!(
            attachment_metadata
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.attachment_type.as_deref()),
            Some("video")
        );
        assert_eq!(
            attachment_metadata
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.playback_rate.as_deref()),
            Some("0.9")
        );
        assert_eq!(
            attachment_metadata
                .video_status
                .as_ref()
                .map(|status| status.duration_secs),
            Some(602)
        );

        let queue_entry = plan.execution_queue[0]
            .attachment_metadata
            .as_ref()
            .expect("queue attachment metadata to be preserved");
        assert_eq!(queue_entry.fid, Some(999));
        assert_eq!(
            queue_entry
                .video_status
                .as_ref()
                .map(|status| status.duration_secs),
            Some(602)
        );
    }

    #[test]
    fn builds_video_execution_context_from_preserved_queue_metadata() {
        let mut task_point = sample_task_point(0, 0, "insertvideo", "Intro video", "video-001");
        task_point.attachment_metadata = Some(sample_video_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(11, 1, "1", "Warmup", vec![task_point])],
        );

        let context = plan.execution_queue[0]
            .video_execution_context()
            .expect("video execution context");

        assert_eq!(context.queue_index, 0);
        assert_eq!(context.chapter_id, 11);
        assert_eq!(context.card_index, 0);
        assert_eq!(context.point_index, 0);
        assert_eq!(context.object_id, "video-001");
        assert_eq!(context.title, "Intro video");
        assert_eq!(context.job_id, "job-001");
        assert_eq!(context.other_info, "nodeId_11");
        assert_eq!(context.playback_rate, "0.9");
        assert_eq!(context.fid, 999);
        assert_eq!(context.duration_secs, 602);
        assert_eq!(context.dtoken, "dtoken-001");
    }

    #[test]
    fn builds_live_execution_context_from_preserved_queue_metadata() {
        let mut task_point =
            sample_task_point(0, 0, "insertlive", "Live session", "live-course-001");
        task_point.iframe_data = Some(
            "{\"liveId\":\"live-course-001\",\"vdoid\":\"vdo-live-001\",\"streamName\":\"zhibo_12345\"}"
                .to_owned(),
        );
        task_point.attachment_metadata = Some(sample_live_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Live", vec![task_point])],
        );

        let context = plan.execution_queue[0]
            .live_execution_context()
            .expect("live execution context");

        assert_eq!(context.queue_index, 0);
        assert_eq!(context.chapter_id, 13);
        assert_eq!(context.card_index, 0);
        assert_eq!(context.point_index, 0);
        assert_eq!(context.title, "直播任务点");
        assert_eq!(context.live_id, "live-course-001");
        assert_eq!(context.job_id, "job-live-001");
        assert_eq!(context.stream_name, "zhibo_12345");
        assert_eq!(context.vdo_id, "vdo-live-001");
    }

    #[test]
    fn rejects_live_execution_context_without_iframe_data() {
        let mut task_point =
            sample_task_point(0, 0, "insertlive", "Live session", "live-course-001");
        task_point.attachment_metadata = Some(sample_live_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Live", vec![task_point])],
        );

        let err = plan.execution_queue[0]
            .live_execution_context()
            .expect_err("missing iframe_data to fail");

        assert!(err.to_string().contains("missing live iframe_data"));
    }

    #[test]
    fn rejects_live_execution_context_with_invalid_iframe_json() {
        let mut task_point =
            sample_task_point(0, 0, "insertlive", "Live session", "live-course-001");
        task_point.iframe_data = Some("{not-json}".to_owned());
        task_point.attachment_metadata = Some(sample_live_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Live", vec![task_point])],
        );

        let err = plan.execution_queue[0]
            .live_execution_context()
            .expect_err("invalid iframe_data to fail");

        assert!(err.to_string().contains("invalid live iframe_data"));
    }

    #[test]
    fn rejects_live_execution_context_when_attachment_metadata_mismatches_iframe() {
        let mut task_point =
            sample_task_point(0, 0, "insertlive", "Live session", "live-course-001");
        task_point.iframe_data = Some(
            "{\"liveId\":\"live-course-001\",\"vdoid\":\"vdo-live-001\",\"streamName\":\"zhibo_12345\"}"
                .to_owned(),
        );
        let mut metadata = sample_live_attachment_metadata();
        metadata
            .attachment
            .as_mut()
            .expect("live attachment")
            .stream_name = Some("zhibo_other".to_owned());
        task_point.attachment_metadata = Some(metadata);

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Live", vec![task_point])],
        );

        let err = plan.execution_queue[0]
            .live_execution_context()
            .expect_err("mismatched stream_name to fail");

        let error_message = err.to_string();
        assert!(error_message.contains("live stream_name in attachment metadata"));
        assert!(error_message.contains("zhibo_other"));
        assert!(error_message.contains("zhibo_12345"));
    }

    #[test]
    fn builds_live_progress_report_request_from_execution_context() {
        let mut task_point =
            sample_task_point(0, 0, "insertlive", "Live session", "live-course-001");
        task_point.iframe_data = Some(
            "{\"liveId\":\"live-course-001\",\"vdoid\":\"vdo-live-001\",\"streamName\":\"zhibo_12345\"}"
                .to_owned(),
        );
        task_point.attachment_metadata = Some(sample_live_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Live", vec![task_point])],
        );

        let request = plan.execution_queue[0]
            .live_execution_context()
            .expect("live execution context")
            .build_progress_report_request(
                &plan.course,
                &sample_account(),
                false,
                1_717_000_000_123,
            );

        assert_eq!(request.stream_name, "zhibo_12345");
        assert_eq!(request.vdo_id, "vdo-live-001");
        assert_eq!(request.user_id, 114514);
        assert!(!request.is_start);
        assert_eq!(request.course_id, 1001);
        assert_eq!(request.report_timestamp_millis, 1_717_000_000_123);
    }

    #[test]
    fn builds_document_execution_context_from_preserved_queue_metadata() {
        let mut task_point = sample_task_point(0, 0, "insertdoc", "Guide", "doc-001");
        task_point.attachment_metadata = Some(sample_document_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(12, 2, "2", "Reading", vec![task_point])],
        );

        let context = plan.execution_queue[0]
            .document_execution_context()
            .expect("document execution context");

        assert_eq!(context.queue_index, 0);
        assert_eq!(context.chapter_id, 12);
        assert_eq!(context.card_index, 0);
        assert_eq!(context.point_index, 0);
        assert_eq!(context.object_id, "doc-001");
        assert_eq!(context.title, "词汇材料");
        assert_eq!(context.job_id, "job-doc-001");
        assert_eq!(context.jtoken, "jtoken-doc-001");
    }

    #[test]
    fn preserves_chapter_work_metadata_in_plan_and_queue() {
        let mut task_point = sample_task_point(0, 0, "work", "Quiz", "work-001");
        task_point.chapter_work_metadata = Some(sample_chapter_work_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Practice", vec![task_point])],
        );

        assert_eq!(
            plan.chapters[0].task_points[0]
                .chapter_work_metadata
                .as_ref()
                .map(|metadata| metadata.work_id.as_str()),
            Some("work-001")
        );
        assert_eq!(
            plan.chapters[0].task_points[0]
                .chapter_work_metadata
                .as_ref()
                .and_then(|metadata| metadata.job_id.as_deref()),
            Some("job-work-001")
        );
        assert_eq!(
            plan.execution_queue[0]
                .chapter_work_metadata
                .as_ref()
                .and_then(|metadata| metadata.school_id.as_deref()),
            Some("school-001")
        );
    }

    #[test]
    fn builds_chapter_work_execution_context_from_preserved_queue_metadata() {
        let mut task_point = sample_task_point(0, 0, "work", "Quiz", "work-001");
        task_point.chapter_work_metadata = Some(sample_chapter_work_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Practice", vec![task_point])],
        );

        let context = plan.execution_queue[0]
            .chapter_work_execution_context()
            .expect("chapter work execution context");

        assert_eq!(context.queue_index, 0);
        assert_eq!(context.chapter_id, 13);
        assert_eq!(context.card_index, 0);
        assert_eq!(context.point_index, 0);
        assert_eq!(context.title, "Quiz");
        assert_eq!(context.work_id, "work-001");
        assert_eq!(context.job_id, "job-work-001");
        assert_eq!(context.school_id.as_deref(), Some("school-001"));
    }

    #[test]
    fn builds_chapter_work_runtime_request_from_execution_context() {
        let mut task_point = sample_task_point(1, 0, "work", "Quiz", "work-001");
        task_point.chapter_work_metadata = Some(sample_chapter_work_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(11, 1, "1", "Warmup", vec![task_point])],
        );

        let request = plan.execution_queue[0]
            .chapter_work_execution_context()
            .expect("chapter work execution context")
            .build_runtime_request(&plan.course, &sample_account());

        assert_eq!(request.course_id, 1001);
        assert_eq!(request.class_id, 2001);
        assert_eq!(request.knowledge_id, 11);
        assert_eq!(request.user_id, 114514);
        assert_eq!(request.cpi, 3001);
        assert_eq!(request.card_index, 1);
        assert_eq!(request.work_id, "work-001");
        assert_eq!(request.job_id, "job-work-001");
        assert_eq!(request.school_id.as_deref(), Some("school-001"));
        assert_eq!(request.relation_work_id(), "school-001-work-001");
    }

    #[test]
    fn rejects_chapter_work_execution_context_without_job_id() {
        let mut task_point = sample_task_point(0, 0, "work", "Quiz", "work-001");
        let mut metadata = sample_chapter_work_metadata();
        metadata.job_id = None;
        task_point.chapter_work_metadata = Some(metadata);

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Practice", vec![task_point])],
        );

        let err = plan.execution_queue[0]
            .chapter_work_execution_context()
            .expect_err("missing chapter work job_id to fail");

        assert!(err.to_string().contains("missing chapter work job_id"));
    }

    #[test]
    fn rejects_chapter_work_execution_context_with_mismatched_work_id() {
        let mut task_point = sample_task_point(0, 0, "work", "Quiz", "work-001");
        let mut metadata = sample_chapter_work_metadata();
        metadata.work_id = "work-999".to_owned();
        task_point.chapter_work_metadata = Some(metadata);

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Practice", vec![task_point])],
        );

        let err = plan.execution_queue[0]
            .chapter_work_execution_context()
            .expect_err("mismatched work_id to fail");

        assert!(
            err.to_string()
                .contains("work_id work-999 does not match resource_id work-001")
        );
    }

    #[test]
    fn rejects_document_execution_context_without_jtoken() {
        let mut task_point = sample_task_point(0, 0, "insertdoc", "Guide", "doc-001");
        let mut metadata = sample_document_attachment_metadata();
        metadata
            .attachment
            .as_mut()
            .expect("document attachment")
            .jtoken = None;
        task_point.attachment_metadata = Some(metadata);

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(12, 2, "2", "Reading", vec![task_point])],
        );

        let err = plan.execution_queue[0]
            .document_execution_context()
            .expect_err("missing jtoken to fail");

        assert!(err.to_string().contains("missing document jtoken"));
    }

    #[test]
    fn builds_document_reading_report_request_from_execution_context() {
        let mut task_point = sample_task_point(0, 0, "insertdoc", "Guide", "doc-001");
        task_point.attachment_metadata = Some(sample_document_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(12, 2, "2", "Reading", vec![task_point])],
        );

        let request = plan.execution_queue[0]
            .document_execution_context()
            .expect("document execution context")
            .build_reading_report_request(&plan.course, 1_717_000_000_123);

        assert_eq!(request.course_id, 1001);
        assert_eq!(request.class_id, 2001);
        assert_eq!(request.knowledge_id, 12);
        assert_eq!(request.job_id, "job-doc-001");
        assert_eq!(request.jtoken, "jtoken-doc-001");
        assert_eq!(request.report_timestamp_millis, 1_717_000_000_123);
    }

    #[test]
    fn builds_video_play_report_request_from_execution_context() {
        let mut task_point = sample_task_point(0, 0, "insertvideo", "Intro video", "video-001");
        task_point.attachment_metadata = Some(sample_video_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(11, 1, "1", "Warmup", vec![task_point])],
        );

        let request = plan.execution_queue[0]
            .video_execution_context()
            .expect("video execution context")
            .build_play_report_request(&plan.course, &sample_account(), 301, 1_717_000_000_123)
            .expect("video play report request");

        assert_eq!(request.cpi, 3001);
        assert_eq!(request.class_id, 2001);
        assert_eq!(request.user_id, 114514);
        assert_eq!(request.dtoken, "dtoken-001");
        assert_eq!(request.object_id, "video-001");
        assert_eq!(request.job_id, "job-001");
        assert_eq!(request.other_info, "nodeId_11");
        assert_eq!(request.playing_time_secs, 301);
        assert_eq!(request.duration_secs, 602);
        assert_eq!(request.clip_time, "0_602");
        assert_eq!(request.playback_rate, "0.9");
        assert_eq!(request.report_timestamp_millis, 1_717_000_000_123);
        assert_eq!(request.enc, "4360136dffe14a02ffff28e09e869946");
    }

    #[test]
    fn rejects_video_play_report_request_beyond_duration() {
        let mut task_point = sample_task_point(0, 0, "insertvideo", "Intro video", "video-001");
        task_point.attachment_metadata = Some(sample_video_attachment_metadata());

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(11, 1, "1", "Warmup", vec![task_point])],
        );

        let err = plan.execution_queue[0]
            .video_execution_context()
            .expect("video execution context")
            .build_play_report_request(&plan.course, &sample_account(), 603, 1_717_000_000_123)
            .expect_err("playing time beyond duration to fail");

        assert!(
            err.to_string()
                .contains("video playing_time_secs 603 exceeds duration 602")
        );
    }

    #[test]
    fn defaults_video_execution_context_playback_rate_to_legacy_value() {
        let mut task_point = sample_task_point(0, 0, "insertvideo", "Intro video", "video-001");
        let mut metadata = sample_video_attachment_metadata();
        metadata
            .attachment
            .as_mut()
            .expect("attachment metadata")
            .playback_rate = None;
        task_point.attachment_metadata = Some(metadata);

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(11, 1, "1", "Warmup", vec![task_point])],
        );

        assert_eq!(
            plan.execution_queue[0]
                .video_execution_context()
                .expect("video execution context")
                .playback_rate,
            "0.9"
        );
    }

    #[test]
    fn rejects_video_execution_context_with_invalid_playback_rate() {
        let mut task_point = sample_task_point(0, 0, "insertvideo", "Intro video", "video-001");
        let mut metadata = sample_video_attachment_metadata();
        metadata
            .attachment
            .as_mut()
            .expect("attachment metadata")
            .playback_rate = Some("fast".to_owned());
        task_point.attachment_metadata = Some(metadata);

        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(11, 1, "1", "Warmup", vec![task_point])],
        );

        let err = plan.execution_queue[0]
            .video_execution_context()
            .expect_err("invalid playback rate to fail");
        assert!(err.to_string().contains("invalid video playback_rate"));
    }

    #[test]
    fn builds_a_flattened_execution_queue_for_future_executors() {
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![
                sample_chapter(
                    22,
                    2,
                    "2",
                    "Follow-up",
                    vec![
                        sample_task_point(1, 1, "work", "Quiz", "work-001"),
                        sample_task_point(0, 0, "insertdoc", "Guide", "doc-001"),
                    ],
                ),
                sample_chapter(
                    11,
                    1,
                    "1",
                    "Warmup",
                    vec![sample_task_point(
                        0,
                        0,
                        "insertvideo",
                        "Intro video",
                        "video-001",
                    )],
                ),
            ],
        );

        let queue = plan.iter_execution_queue().cloned().collect::<Vec<_>>();
        assert_eq!(queue.len(), 3);
        assert_eq!(
            queue,
            vec![
                CourseRunQueueEntry {
                    queue_index: 0,
                    chapter_id: 11,
                    chapter_index: 1,
                    chapter_name: "Warmup".to_owned(),
                    chapter_label: "1".to_owned(),
                    card_index: 0,
                    point_index: 0,
                    module: "insertvideo".to_owned(),
                    task_kind: CourseRunTaskKind::Video,
                    title: "Intro video".to_owned(),
                    resource_id: "video-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                },
                CourseRunQueueEntry {
                    queue_index: 1,
                    chapter_id: 22,
                    chapter_index: 2,
                    chapter_name: "Follow-up".to_owned(),
                    chapter_label: "2".to_owned(),
                    card_index: 0,
                    point_index: 0,
                    module: "insertdoc".to_owned(),
                    task_kind: CourseRunTaskKind::Document,
                    title: "Guide".to_owned(),
                    resource_id: "doc-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                },
                CourseRunQueueEntry {
                    queue_index: 2,
                    chapter_id: 22,
                    chapter_index: 2,
                    chapter_name: "Follow-up".to_owned(),
                    chapter_label: "2".to_owned(),
                    card_index: 1,
                    point_index: 1,
                    module: "work".to_owned(),
                    task_kind: CourseRunTaskKind::ChapterWork,
                    title: "Quiz".to_owned(),
                    resource_id: "work-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                },
            ]
        );
    }

    #[test]
    fn resolves_execution_queue_in_stable_plan_order() {
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![
                sample_chapter(
                    22,
                    2,
                    "2",
                    "Follow-up",
                    vec![
                        sample_task_point(1, 1, "work", "Quiz", "work-001"),
                        sample_task_point(0, 0, "insertdoc", "Guide", "doc-001"),
                    ],
                ),
                sample_chapter(
                    11,
                    1,
                    "1",
                    "Warmup",
                    vec![sample_task_point(
                        0,
                        0,
                        "insertvideo",
                        "Intro video",
                        "video-001",
                    )],
                ),
            ],
        );
        let registry = TaskExecutorRegistry::new();

        let resolutions = plan
            .iter_resolved_execution_queue(&registry)
            .map(|resolved| {
                (
                    resolved.entry.queue_index,
                    resolved.entry.module.clone(),
                    resolved.resolution,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            resolutions,
            vec![
                (
                    0,
                    "insertvideo".to_owned(),
                    TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::Video,
                            module: "insertvideo",
                        },
                    },
                ),
                (
                    1,
                    "insertdoc".to_owned(),
                    TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::Document,
                            module: "insertdoc",
                        },
                    },
                ),
                (
                    2,
                    "work".to_owned(),
                    TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::ChapterWork,
                            module: "work",
                        },
                    },
                ),
            ]
        );
    }

    #[test]
    fn summarizes_registered_execution_preflight_in_queue_order() {
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![
                sample_chapter(
                    22,
                    2,
                    "2",
                    "Follow-up",
                    vec![
                        sample_task_point(1, 1, "work", "Quiz", "work-001"),
                        sample_task_point(0, 0, "insertdoc", "Guide", "doc-001"),
                    ],
                ),
                sample_chapter(
                    11,
                    1,
                    "1",
                    "Warmup",
                    vec![sample_task_point(
                        0,
                        0,
                        "insertvideo",
                        "Intro video",
                        "video-001",
                    )],
                ),
            ],
        );
        let registry = TaskExecutorRegistry::new();

        let preflight = plan.build_execution_preflight(&registry);

        assert!(preflight.all_entries_registered);
        assert_eq!(preflight.total_entries, 3);
        assert_eq!(preflight.registered_entries, 3);
        assert_eq!(preflight.blocked_entries, 0);
        assert_eq!(
            preflight.queue,
            vec![
                CourseRunExecutionPreflightEntry {
                    queue_index: 0,
                    resolution: TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::Video,
                            module: "insertvideo",
                        },
                    },
                },
                CourseRunExecutionPreflightEntry {
                    queue_index: 1,
                    resolution: TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::Document,
                            module: "insertdoc",
                        },
                    },
                },
                CourseRunExecutionPreflightEntry {
                    queue_index: 2,
                    resolution: TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::ChapterWork,
                            module: "work",
                        },
                    },
                },
            ]
        );
    }

    #[test]
    fn summarizes_fail_closed_execution_preflight_for_blocked_entries() {
        let plan = CourseRunPlan {
            course: sample_course(),
            chapters: vec![],
            execution_queue: vec![
                CourseRunQueueEntry {
                    queue_index: 0,
                    chapter_id: 11,
                    chapter_index: 1,
                    chapter_name: "Warmup".to_owned(),
                    chapter_label: "1".to_owned(),
                    card_index: 0,
                    point_index: 0,
                    module: "insertvideo".to_owned(),
                    task_kind: CourseRunTaskKind::Video,
                    title: "Intro video".to_owned(),
                    resource_id: "video-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                },
                CourseRunQueueEntry {
                    queue_index: 1,
                    chapter_id: 11,
                    chapter_index: 1,
                    chapter_name: "Warmup".to_owned(),
                    chapter_label: "1".to_owned(),
                    card_index: 1,
                    point_index: 0,
                    module: "insertvideo".to_owned(),
                    task_kind: CourseRunTaskKind::Document,
                    title: "Broken task".to_owned(),
                    resource_id: "broken-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                },
                CourseRunQueueEntry {
                    queue_index: 2,
                    chapter_id: 12,
                    chapter_index: 2,
                    chapter_name: "Follow-up".to_owned(),
                    chapter_label: "2".to_owned(),
                    card_index: 0,
                    point_index: 0,
                    module: "insertaudio".to_owned(),
                    task_kind: CourseRunTaskKind::Unknown("insertaudio".to_owned()),
                    title: "Unsupported task".to_owned(),
                    resource_id: "audio-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                },
            ],
            total_task_points: 3,
        };
        let registry = TaskExecutorRegistry::new();

        let preflight = plan.build_execution_preflight(&registry);

        assert!(!preflight.all_entries_registered);
        assert_eq!(preflight.total_entries, 3);
        assert_eq!(preflight.registered_entries, 1);
        assert_eq!(preflight.blocked_entries, 2);
        assert_eq!(
            preflight.queue,
            vec![
                CourseRunExecutionPreflightEntry {
                    queue_index: 0,
                    resolution: TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::Video,
                            module: "insertvideo",
                        },
                    },
                },
                CourseRunExecutionPreflightEntry {
                    queue_index: 1,
                    resolution: TaskExecutorResolution::InconsistentTaskKind {
                        module: "insertvideo".to_owned(),
                        task_kind: CourseRunTaskKind::Document,
                        expected_key: TaskExecutorKey::Video,
                    },
                },
                CourseRunExecutionPreflightEntry {
                    queue_index: 2,
                    resolution: TaskExecutorResolution::UnsupportedModule {
                        module: "insertaudio".to_owned(),
                        task_kind: CourseRunTaskKind::Unknown("insertaudio".to_owned()),
                    },
                },
            ]
        );
    }

    #[test]
    fn builds_pending_runtime_execution_results_from_preflight() {
        let preflight = sample_blocked_execution_preflight();

        let result = CourseRunExecutionResult::from_preflight(1001, &preflight);

        assert_eq!(result.course_id, 1001);
        assert_eq!(result.state, CourseRunExecutionState::Pending);
        assert_eq!(result.total_entries, 3);
        assert_eq!(result.completed_entries, 0);
        assert_eq!(result.blocked_entries, 0);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| (entry.queue_index, entry.state))
                .collect::<Vec<_>>(),
            vec![
                (0, CourseRunQueueEntryState::Pending),
                (1, CourseRunQueueEntryState::Pending),
                (2, CourseRunQueueEntryState::Pending),
            ]
        );
        assert_eq!(
            result.queue[1].resolution,
            TaskExecutorResolution::InconsistentTaskKind {
                module: "insertvideo".to_owned(),
                task_kind: CourseRunTaskKind::Document,
                expected_key: TaskExecutorKey::Video,
            }
        );
        assert_eq!(
            result.queue[2].resolution,
            TaskExecutorResolution::UnsupportedModule {
                module: "insertaudio".to_owned(),
                task_kind: CourseRunTaskKind::Unknown("insertaudio".to_owned()),
            }
        );
    }

    #[test]
    fn tracks_runtime_queue_transitions_until_completion() {
        let preflight = sample_registered_execution_preflight();
        let mut result = CourseRunExecutionResult::from_preflight(1001, &preflight);

        result.start().expect("start execution");
        result
            .mark_queue_entry_running(0)
            .expect("queue entry 0 to start");
        result
            .mark_queue_entry_completed(0)
            .expect("queue entry 0 to finish");
        result
            .mark_queue_entry_running(1)
            .expect("queue entry 1 to start");
        result
            .mark_queue_entry_completed(1)
            .expect("queue entry 1 to finish");
        result
            .mark_queue_entry_running(2)
            .expect("queue entry 2 to start");
        result
            .mark_queue_entry_completed(2)
            .expect("queue entry 2 to finish");
        result.finish().expect("finish execution");

        assert_eq!(result.state, CourseRunExecutionState::Completed);
        assert_eq!(result.completed_entries, 3);
        assert_eq!(result.blocked_entries, 0);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| entry.state)
                .collect::<Vec<_>>(),
            vec![
                CourseRunQueueEntryState::Completed,
                CourseRunQueueEntryState::Completed,
                CourseRunQueueEntryState::Completed,
            ]
        );
    }

    #[test]
    fn rejects_invalid_runtime_transitions_before_entries_start_running() {
        let preflight = sample_registered_execution_preflight();
        let mut result = CourseRunExecutionResult::from_preflight(1001, &preflight);

        let error = result
            .mark_queue_entry_running(0)
            .expect_err("execution must be started before queue transitions");
        assert_eq!(
            error.to_string(),
            "validation error: course run execution cannot advance queue entry state while state is pending"
        );

        result.start().expect("start execution");

        let error = result
            .mark_queue_entry_completed(0)
            .expect_err("queue entry cannot skip running state");
        assert_eq!(
            error.to_string(),
            "validation error: queue_index 0 cannot transition from pending to completed"
        );

        let error = result
            .mark_queue_entry_running(99)
            .expect_err("missing queue entry to fail");
        assert_eq!(
            error.to_string(),
            "validation error: queue_index 99 not found in course run execution result"
        );
    }

    #[test]
    fn finishes_blocked_when_any_queue_entry_stops_fail_closed() {
        let preflight = sample_blocked_execution_preflight();
        let mut result = CourseRunExecutionResult::from_preflight(1001, &preflight);

        result.start().expect("start execution");
        result
            .mark_queue_entry_running(0)
            .expect("first queue entry to start");
        result
            .mark_queue_entry_completed(0)
            .expect("first queue entry to finish");
        result
            .mark_queue_entry_blocked(1)
            .expect("blocked queue entry to transition");
        result.finish().expect("finish blocked execution");

        assert_eq!(result.state, CourseRunExecutionState::Blocked);
        assert_eq!(result.completed_entries, 1);
        assert_eq!(result.blocked_entries, 1);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| (entry.queue_index, entry.state))
                .collect::<Vec<_>>(),
            vec![
                (0, CourseRunQueueEntryState::Completed),
                (1, CourseRunQueueEntryState::Blocked),
                (2, CourseRunQueueEntryState::Pending),
            ]
        );
    }

    #[tokio::test]
    async fn drives_registered_queue_entries_in_stable_order() {
        let registry = TaskExecutorRegistry::new();
        let sink = Arc::new(TestRunEventSink::default());
        let driver = CourseRunHeadlessDriver::new(&registry).with_sink(sink.clone());
        let executor = TestQueueExecutor::default();
        let plan = sample_registered_execution_plan();

        let result = driver
            .drive(&plan, &executor)
            .await
            .expect("registered queue to complete");

        assert_eq!(
            executor.calls(),
            vec![
                (0, TaskExecutorKey::Video),
                (1, TaskExecutorKey::Document),
                (2, TaskExecutorKey::ChapterWork),
            ]
        );
        assert_eq!(result.state, CourseRunExecutionState::Completed);
        assert_eq!(result.completed_entries, 3);
        assert_eq!(result.blocked_entries, 0);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| (entry.queue_index, entry.state))
                .collect::<Vec<_>>(),
            vec![
                (0, CourseRunQueueEntryState::Completed),
                (1, CourseRunQueueEntryState::Completed),
                (2, CourseRunQueueEntryState::Completed),
            ]
        );
        assert_eq!(
            sink.snapshot(),
            vec![
                RunEvent::CourseRunExecutionStarted {
                    course_id: 1001,
                    total_entries: 3,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Completed,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 1,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 1,
                    state: CourseRunQueueEntryState::Completed,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 2,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 2,
                    state: CourseRunQueueEntryState::Completed,
                },
                RunEvent::CourseRunExecutionFinished {
                    course_id: 1001,
                    state: CourseRunExecutionState::Completed,
                    total_entries: 3,
                    completed_entries: 3,
                    blocked_entries: 0,
                },
            ]
        );
    }

    #[tokio::test]
    async fn stops_fail_closed_when_planner_output_is_inconsistent() {
        let registry = TaskExecutorRegistry::new();
        let sink = Arc::new(TestRunEventSink::default());
        let driver = CourseRunHeadlessDriver::new(&registry).with_sink(sink.clone());
        let executor = TestQueueExecutor::default();
        let plan = sample_fail_closed_execution_plan();

        let result = driver
            .drive(&plan, &executor)
            .await
            .expect("inconsistent queue to stop fail closed");

        assert_eq!(executor.calls(), vec![(0, TaskExecutorKey::Video)]);
        assert_eq!(result.state, CourseRunExecutionState::Blocked);
        assert_eq!(result.completed_entries, 1);
        assert_eq!(result.blocked_entries, 1);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| (entry.queue_index, entry.state))
                .collect::<Vec<_>>(),
            vec![
                (0, CourseRunQueueEntryState::Completed),
                (1, CourseRunQueueEntryState::Blocked),
                (2, CourseRunQueueEntryState::Pending),
            ]
        );
        assert_eq!(
            sink.snapshot(),
            vec![
                RunEvent::CourseRunExecutionStarted {
                    course_id: 1001,
                    total_entries: 3,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Completed,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 1,
                    state: CourseRunQueueEntryState::Blocked,
                },
                RunEvent::CourseRunExecutionFinished {
                    course_id: 1001,
                    state: CourseRunExecutionState::Blocked,
                    total_entries: 3,
                    completed_entries: 1,
                    blocked_entries: 1,
                },
            ]
        );
    }

    #[tokio::test]
    async fn stops_after_executor_reports_a_blocked_registered_entry() {
        let registry = TaskExecutorRegistry::new();
        let sink = Arc::new(TestRunEventSink::default());
        let driver = CourseRunHeadlessDriver::new(&registry).with_sink(sink.clone());
        let executor = TestQueueExecutor::with_blocked_at(1);
        let plan = sample_registered_execution_plan();

        let result = driver
            .drive(&plan, &executor)
            .await
            .expect("executor-level block to stop queue");

        assert_eq!(
            executor.calls(),
            vec![(0, TaskExecutorKey::Video), (1, TaskExecutorKey::Document),]
        );
        assert_eq!(result.state, CourseRunExecutionState::Blocked);
        assert_eq!(result.completed_entries, 1);
        assert_eq!(result.blocked_entries, 1);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| (entry.queue_index, entry.state))
                .collect::<Vec<_>>(),
            vec![
                (0, CourseRunQueueEntryState::Completed),
                (1, CourseRunQueueEntryState::Blocked),
                (2, CourseRunQueueEntryState::Pending),
            ]
        );
        assert_eq!(
            sink.snapshot(),
            vec![
                RunEvent::CourseRunExecutionStarted {
                    course_id: 1001,
                    total_entries: 3,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Completed,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 1,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 1,
                    state: CourseRunQueueEntryState::Blocked,
                },
                RunEvent::CourseRunExecutionFinished {
                    course_id: 1001,
                    state: CourseRunExecutionState::Blocked,
                    total_entries: 3,
                    completed_entries: 1,
                    blocked_entries: 1,
                },
            ]
        );
    }

    #[tokio::test]
    async fn fail_closed_executor_warns_and_blocks_before_task_endpoints() {
        let sink = Arc::new(TestRunEventSink::default());
        let executor = FailClosedCourseRunExecutor::new().with_sink(sink.clone());
        let entry = sample_registered_execution_plan().execution_queue[0].clone();

        let outcome = executor
            .execute_queue_entry(
                &entry,
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Video,
                    module: "insertvideo",
                },
            )
            .await
            .expect("executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::Warning {
                message: "queue entry 0 reached headless dispatch for executor video (insertvideo) but module-specific runtime support is not implemented yet; stopping fail-closed before any task endpoint is called".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn headless_video_executor_completes_fixture_backed_video_entries() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let plan = runner
            .build_plan(CourseRunTarget::new(Some(1001), None).expect("course target"))
            .await
            .expect("fixture-backed course plan");
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessVideoCourseRunExecutor::new(
            client.clone(),
            plan.course.clone(),
            sample_account(),
        )
        .with_sink(sink.clone());

        let outcome = executor
            .execute_queue_entry(
                &plan.execution_queue[0],
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Video,
                    module: "insertvideo",
                },
            )
            .await
            .expect("video executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Completed);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::CourseRunVideoProgressReported {
                course_id: 1001,
                queue_index: 0,
                playing_time_secs: 602,
                duration_secs: 602,
                is_passed: true,
            }]
        );
    }

    #[tokio::test]
    async fn headless_video_executor_keeps_non_video_entries_fail_closed() {
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessVideoCourseRunExecutor::new(
            ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy"),
            ))),
            sample_course(),
            sample_account(),
        )
        .with_sink(sink.clone());
        let entry = sample_registered_execution_plan().execution_queue[1].clone();

        let outcome = executor
            .execute_queue_entry(
                &entry,
                TaskExecutorRegistration {
                    key: TaskExecutorKey::ChapterWork,
                    module: "work",
                },
            )
            .await
            .expect("fallback executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::Warning {
                message: "queue entry 1 reached headless dispatch for executor chapter_work (work) but module-specific runtime support is not implemented yet; stopping fail-closed before any task endpoint is called".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn headless_document_executor_completes_fixture_backed_document_entries() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let plan = runner
            .build_plan(CourseRunTarget::new(Some(1001), None).expect("course target"))
            .await
            .expect("fixture-backed course plan");
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessDocumentCourseRunExecutor::new(client.clone(), plan.course.clone())
            .with_sink(sink.clone());

        let outcome = executor
            .execute_queue_entry(
                &plan.execution_queue[2],
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Document,
                    module: "insertdoc",
                },
            )
            .await
            .expect("document executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Completed);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::CourseRunDocumentProgressReported {
                course_id: 1001,
                queue_index: 2,
                success: true,
            }]
        );
    }

    #[tokio::test]
    async fn headless_document_executor_blocks_on_rejected_fixture_acknowledgements() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessDocumentCourseRunExecutor::new(client.clone(), sample_course())
            .with_sink(sink.clone());
        let mut task_point = sample_task_point(0, 0, "insertdoc", "Guide", "doc-001");
        let mut metadata = sample_document_attachment_metadata();
        metadata
            .attachment
            .as_mut()
            .expect("document attachment")
            .jtoken = Some("jtoken-doc-001-error".to_owned());
        task_point.attachment_metadata = Some(metadata);
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(12, 2, "2", "Reading", vec![task_point])],
        );

        let outcome = executor
            .execute_queue_entry(
                &plan.execution_queue[0],
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Document,
                    module: "insertdoc",
                },
            )
            .await
            .expect("document executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::Warning {
                message: "queue entry 0 document reading-report request failed: unexpected response: document reading-report rejected: document progress rejected; stopping fail-closed after the first runtime acknowledgement attempt".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn headless_document_executor_keeps_non_document_entries_fail_closed() {
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessDocumentCourseRunExecutor::new(
            ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy"),
            ))),
            sample_course(),
        )
        .with_sink(sink.clone());
        let entry = sample_registered_execution_plan().execution_queue[0].clone();

        let outcome = executor
            .execute_queue_entry(
                &entry,
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Video,
                    module: "insertvideo",
                },
            )
            .await
            .expect("fallback executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::Warning {
                message: "queue entry 0 reached headless dispatch for executor video (insertvideo) but module-specific runtime support is not implemented yet; stopping fail-closed before any task endpoint is called".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn headless_live_executor_acknowledges_fixture_backed_live_entries_and_blocks() {
        let fixture_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy_live");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let plan = runner
            .build_plan(CourseRunTarget::new(Some(1001), None).expect("course target"))
            .await
            .expect("fixture-backed live course plan");
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessLiveCourseRunExecutor::new(
            client.clone(),
            plan.course.clone(),
            sample_account(),
        )
        .with_sink(sink.clone());

        let outcome = executor
            .execute_queue_entry(
                &plan.execution_queue[0],
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Live,
                    module: "insertlive",
                },
            )
            .await
            .expect("live executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        assert_eq!(
            sink.snapshot(),
            vec![
                RunEvent::CourseRunLiveProgressReported {
                    course_id: 1001,
                    queue_index: 0,
                    success: true,
                },
                RunEvent::Warning {
                    message: "queue entry 0 acknowledged the reviewed live progress-report route for live-course-001 and stopped fail-closed before attendance, completion, or any additional live runtime endpoints are called".to_owned(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn headless_live_executor_blocks_on_rejected_fixture_acknowledgements() {
        let fixture_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy_live");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let sink = Arc::new(TestRunEventSink::default());
        let executor =
            HeadlessLiveCourseRunExecutor::new(client.clone(), sample_course(), sample_account())
                .with_sink(sink.clone());
        let mut task_point =
            sample_task_point(0, 0, "insertlive", "Live session", "live-course-001");
        task_point.iframe_data =
            Some("{\"liveId\":\"live-course-001\",\"vdoid\":\"vdo-live-001-error\",\"streamName\":\"zhibo_12345\"}".to_owned());
        let mut metadata = sample_live_attachment_metadata();
        metadata
            .attachment
            .as_mut()
            .expect("live attachment")
            .vdo_id = Some("vdo-live-001-error".to_owned());
        task_point.attachment_metadata = Some(metadata);
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(13, 3, "3", "Live", vec![task_point])],
        );

        let outcome = executor
            .execute_queue_entry(
                &plan.execution_queue[0],
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Live,
                    module: "insertlive",
                },
            )
            .await
            .expect("live executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::Warning {
                message: "queue entry 0 live progress-report request failed: unexpected response: live progress-report rejected: live progress rejected; stopping fail-closed after the first reviewed runtime acknowledgement attempt".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn headless_live_executor_keeps_non_live_entries_fail_closed() {
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessLiveCourseRunExecutor::new(
            ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy_live"),
            ))),
            sample_course(),
            sample_account(),
        )
        .with_sink(sink.clone());
        let entry = sample_registered_execution_plan().execution_queue[0].clone();

        let outcome = executor
            .execute_queue_entry(
                &entry,
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Video,
                    module: "insertvideo",
                },
            )
            .await
            .expect("fallback executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::Warning {
                message: "queue entry 0 reached headless dispatch for executor video (insertvideo) but module-specific runtime support is not implemented yet; stopping fail-closed before any task endpoint is called".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn driver_blocks_after_acknowledging_live_progress_with_live_executor() {
        let fixture_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy_live");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let registry = TaskExecutorRegistry::new();
        let plan = runner
            .build_plan(CourseRunTarget::new(Some(1001), None).expect("course target"))
            .await
            .expect("fixture-backed live course plan");
        let sink = Arc::new(TestRunEventSink::default());
        let driver = CourseRunHeadlessDriver::new(&registry).with_sink(sink.clone());
        let executor = HeadlessLiveCourseRunExecutor::new(
            client.clone(),
            plan.course.clone(),
            sample_account(),
        )
        .with_sink(sink.clone());

        let result = driver.drive(&plan, &executor).await.expect("driver result");

        assert_eq!(result.state, CourseRunExecutionState::Blocked);
        assert_eq!(result.completed_entries, 0);
        assert_eq!(result.blocked_entries, 1);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| (entry.queue_index, entry.state))
                .collect::<Vec<_>>(),
            vec![(0, CourseRunQueueEntryState::Blocked)]
        );
        assert_eq!(
            sink.snapshot(),
            vec![
                RunEvent::CourseRunExecutionStarted {
                    course_id: 1001,
                    total_entries: 1,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::CourseRunLiveProgressReported {
                    course_id: 1001,
                    queue_index: 0,
                    success: true,
                },
                RunEvent::Warning {
                    message: "queue entry 0 acknowledged the reviewed live progress-report route for live-course-001 and stopped fail-closed before attendance, completion, or any additional live runtime endpoints are called".to_owned(),
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Blocked,
                },
                RunEvent::CourseRunExecutionFinished {
                    course_id: 1001,
                    state: CourseRunExecutionState::Blocked,
                    total_entries: 1,
                    completed_entries: 0,
                    blocked_entries: 1,
                },
            ]
        );
    }

    #[tokio::test]
    async fn headless_chapter_work_executor_fetches_fixture_backed_work_snapshot_and_blocks() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let plan = runner
            .build_plan(CourseRunTarget::new(Some(1001), None).expect("course target"))
            .await
            .expect("fixture-backed course plan");
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessChapterWorkCourseRunExecutor::new(
            client.clone(),
            plan.course.clone(),
            sample_account(),
        )
        .with_sink(sink.clone());

        let outcome = executor
            .execute_queue_entry(
                &plan.execution_queue[1],
                TaskExecutorRegistration {
                    key: TaskExecutorKey::ChapterWork,
                    module: "work",
                },
            )
            .await
            .expect("chapter work executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        let events = sink.snapshot();
        assert_eq!(events.len(), 2);
        match &events[0] {
            RunEvent::CourseRunChapterWorkSnapshotFetched {
                course_id,
                queue_index,
                snapshot,
            } => {
                assert_eq!(*course_id, 1001);
                assert_eq!(*queue_index, 1);
                assert_eq!(snapshot.title, "绪论测验");
                assert_eq!(snapshot.work_answer_id, 99001);
                assert_eq!(snapshot.total_question_num, 3);
                assert_eq!(snapshot.work_relation_id, 88001);
                assert_eq!(snapshot.full_score, "100");
                assert_eq!(snapshot.enc_work, "enc-work-submit-001");
                assert_eq!(snapshot.questions.len(), 3);
            }
            other => panic!("unexpected first event: {other:?}"),
        }
        assert_eq!(
            events[1],
            RunEvent::Warning {
                message: "queue entry 1 fetched chapter-work runtime snapshot 99001 with 3 questions and stopped fail-closed before any answer save or submit endpoint is called".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn headless_chapter_work_executor_prepares_candidate_selection_when_pipeline_is_injected()
    {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let plan = runner
            .build_plan(CourseRunTarget::new(Some(1001), None).expect("course target"))
            .await
            .expect("fixture-backed course plan");
        let mut searcher_pipeline = SearcherPipeline::new();
        searcher_pipeline.add_provider(TestSearcherProvider {
            provider: "json",
            answers: vec!["B"],
        });
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessChapterWorkCourseRunExecutor::new(
            client.clone(),
            plan.course.clone(),
            sample_account(),
        )
        .with_searcher_pipeline(Arc::new(searcher_pipeline))
        .with_sink(sink.clone());

        let outcome = executor
            .execute_queue_entry(
                &plan.execution_queue[1],
                TaskExecutorRegistration {
                    key: TaskExecutorKey::ChapterWork,
                    module: "work",
                },
            )
            .await
            .expect("chapter work executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        let events = sink.snapshot();
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events[0],
            RunEvent::CourseRunChapterWorkSnapshotFetched {
                course_id: 1001,
                queue_index: 1,
                ..
            }
        ));
        assert_eq!(
            events[1],
            RunEvent::CourseRunChapterWorkCandidateSelectionPrepared {
                course_id: 1001,
                queue_index: 1,
                work_answer_id: 99001,
                total_questions: 3,
                selected_questions: 1,
            }
        );
        assert_eq!(
            events[2],
            RunEvent::Warning {
                message: "queue entry 1 prepared chapter-work candidate selections for 1/3 questions from snapshot 99001 and stopped fail-closed before any answer save or submit endpoint is called".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn headless_chapter_work_executor_blocks_on_runtime_snapshot_fetch_failure() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessChapterWorkCourseRunExecutor::new(
            client.clone(),
            sample_course(),
            sample_account(),
        )
        .with_sink(sink.clone());
        let task_point = TaskPointSummary {
            card_index: 1,
            point_index: 0,
            module: "work".to_owned(),
            title: "Broken quiz".to_owned(),
            resource_id: "work-999".to_owned(),
            iframe_data: None,
            attachment_metadata: None,
            chapter_work_metadata: Some(ChapterWorkMetadata {
                work_id: "work-999".to_owned(),
                job_id: Some("job-001".to_owned()),
                school_id: None,
            }),
        };
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(11, 1, "1", "Warmup", vec![task_point])],
        );

        let outcome = executor
            .execute_queue_entry(
                &plan.execution_queue[0],
                TaskExecutorRegistration {
                    key: TaskExecutorKey::ChapterWork,
                    module: "work",
                },
            )
            .await
            .expect("chapter work executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::Warning {
                message: "queue entry 0 chapter-work runtime snapshot fetch failed: unexpected response: missing chapter work attachment for work_id work-999; stopping fail-closed before any answer save or submit endpoint is called".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn headless_chapter_work_executor_keeps_non_work_entries_fail_closed() {
        let sink = Arc::new(TestRunEventSink::default());
        let executor = HeadlessChapterWorkCourseRunExecutor::new(
            ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy"),
            ))),
            sample_course(),
            sample_account(),
        )
        .with_sink(sink.clone());
        let entry = sample_registered_execution_plan().execution_queue[0].clone();

        let outcome = executor
            .execute_queue_entry(
                &entry,
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Video,
                    module: "insertvideo",
                },
            )
            .await
            .expect("fallback executor result");

        assert_eq!(outcome, CourseRunQueueDriverOutcome::Blocked);
        assert_eq!(
            sink.snapshot(),
            vec![RunEvent::Warning {
                message: "queue entry 0 reached headless dispatch for executor video (insertvideo) but module-specific runtime support is not implemented yet; stopping fail-closed before any task endpoint is called".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn driver_blocks_after_fetching_chapter_work_snapshot_with_chapter_work_executor() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let registry = TaskExecutorRegistry::new();
        let sink = Arc::new(TestRunEventSink::default());
        let driver = CourseRunHeadlessDriver::new(&registry).with_sink(sink.clone());
        let executor = HeadlessChapterWorkCourseRunExecutor::new(
            client.clone(),
            sample_course(),
            sample_account(),
        )
        .with_sink(sink.clone());
        let task_point = TaskPointSummary {
            card_index: 1,
            point_index: 0,
            module: "work".to_owned(),
            title: "绪论测验".to_owned(),
            resource_id: "work-001".to_owned(),
            iframe_data: None,
            attachment_metadata: None,
            chapter_work_metadata: Some(ChapterWorkMetadata {
                work_id: "work-001".to_owned(),
                job_id: Some("job-001".to_owned()),
                school_id: None,
            }),
        };
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(11, 1, "1", "Warmup", vec![task_point])],
        );

        let result = driver.drive(&plan, &executor).await.expect("driver result");

        assert_eq!(result.state, CourseRunExecutionState::Blocked);
        assert_eq!(result.completed_entries, 0);
        assert_eq!(result.blocked_entries, 1);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| (entry.queue_index, entry.state))
                .collect::<Vec<_>>(),
            vec![(0, CourseRunQueueEntryState::Blocked)]
        );

        let events = sink.snapshot();
        assert_eq!(events.len(), 6);
        assert_eq!(
            events[0],
            RunEvent::CourseRunExecutionStarted {
                course_id: 1001,
                total_entries: 1,
            }
        );
        assert_eq!(
            events[1],
            RunEvent::CourseRunQueueEntryStateChanged {
                course_id: 1001,
                queue_index: 0,
                state: CourseRunQueueEntryState::Running,
            }
        );
        assert!(matches!(
            events[2],
            RunEvent::CourseRunChapterWorkSnapshotFetched {
                course_id: 1001,
                queue_index: 0,
                ..
            }
        ));
        assert_eq!(
            events[3],
            RunEvent::Warning {
                message: "queue entry 0 fetched chapter-work runtime snapshot 99001 with 3 questions and stopped fail-closed before any answer save or submit endpoint is called".to_owned(),
            }
        );
        assert_eq!(
            events[4],
            RunEvent::CourseRunQueueEntryStateChanged {
                course_id: 1001,
                queue_index: 0,
                state: CourseRunQueueEntryState::Blocked,
            }
        );
        assert_eq!(
            events[5],
            RunEvent::CourseRunExecutionFinished {
                course_id: 1001,
                state: CourseRunExecutionState::Blocked,
                total_entries: 1,
                completed_entries: 0,
                blocked_entries: 1,
            }
        );
    }

    #[tokio::test]
    async fn driver_completes_video_entries_and_keeps_following_modules_fail_closed() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let registry = TaskExecutorRegistry::new();
        let plan = runner
            .build_plan(CourseRunTarget::new(Some(1001), None).expect("course target"))
            .await
            .expect("fixture-backed course plan");
        let sink = Arc::new(TestRunEventSink::default());
        let driver = CourseRunHeadlessDriver::new(&registry).with_sink(sink.clone());
        let executor = HeadlessVideoCourseRunExecutor::new(
            client.clone(),
            plan.course.clone(),
            sample_account(),
        )
        .with_sink(sink.clone());

        let result = driver.drive(&plan, &executor).await.expect("driver result");

        assert_eq!(result.state, CourseRunExecutionState::Blocked);
        assert_eq!(result.completed_entries, 1);
        assert_eq!(result.blocked_entries, 1);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| (entry.queue_index, entry.state))
                .collect::<Vec<_>>(),
            vec![
                (0, CourseRunQueueEntryState::Completed),
                (1, CourseRunQueueEntryState::Blocked),
                (2, CourseRunQueueEntryState::Pending),
            ]
        );
        assert_eq!(
            sink.snapshot(),
            vec![
                RunEvent::CourseRunExecutionStarted {
                    course_id: 1001,
                    total_entries: 3,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::CourseRunVideoProgressReported {
                    course_id: 1001,
                    queue_index: 0,
                    playing_time_secs: 602,
                    duration_secs: 602,
                    is_passed: true,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Completed,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 1,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::Warning {
                    message: "queue entry 1 reached headless dispatch for executor chapter_work (work) but module-specific runtime support is not implemented yet; stopping fail-closed before any task endpoint is called".to_owned(),
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 1,
                    state: CourseRunQueueEntryState::Blocked,
                },
                RunEvent::CourseRunExecutionFinished {
                    course_id: 1001,
                    state: CourseRunExecutionState::Blocked,
                    total_entries: 3,
                    completed_entries: 1,
                    blocked_entries: 1,
                },
            ]
        );
    }

    #[tokio::test]
    async fn driver_completes_document_entries_with_document_executor() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let registry = TaskExecutorRegistry::new();
        let sink = Arc::new(TestRunEventSink::default());
        let driver = CourseRunHeadlessDriver::new(&registry).with_sink(sink.clone());
        let executor = HeadlessDocumentCourseRunExecutor::new(client.clone(), sample_course())
            .with_sink(sink.clone());
        let mut task_point = sample_task_point(0, 0, "insertdoc", "Guide", "doc-001");
        task_point.attachment_metadata = Some(sample_document_attachment_metadata());
        let plan = CourseRunPlan::from_scanned_chapters(
            sample_course(),
            vec![sample_chapter(12, 2, "2", "Reading", vec![task_point])],
        );

        let result = driver.drive(&plan, &executor).await.expect("driver result");

        assert_eq!(result.state, CourseRunExecutionState::Completed);
        assert_eq!(result.completed_entries, 1);
        assert_eq!(result.blocked_entries, 0);
        assert_eq!(
            result
                .queue
                .iter()
                .map(|entry| (entry.queue_index, entry.state))
                .collect::<Vec<_>>(),
            vec![(0, CourseRunQueueEntryState::Completed),]
        );
        assert_eq!(
            sink.snapshot(),
            vec![
                RunEvent::CourseRunExecutionStarted {
                    course_id: 1001,
                    total_entries: 1,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Running,
                },
                RunEvent::CourseRunDocumentProgressReported {
                    course_id: 1001,
                    queue_index: 0,
                    success: true,
                },
                RunEvent::CourseRunQueueEntryStateChanged {
                    course_id: 1001,
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Completed,
                },
                RunEvent::CourseRunExecutionFinished {
                    course_id: 1001,
                    state: CourseRunExecutionState::Completed,
                    total_entries: 1,
                    completed_entries: 1,
                    blocked_entries: 0,
                },
            ]
        );
    }

    #[test]
    fn validates_course_run_target_selection() {
        let error = CourseRunTarget::new(None, None).expect_err("target selection to fail");
        assert_eq!(
            error.to_string(),
            "validation error: one of --course-id or --course-index is required"
        );

        let error = CourseRunTarget::new(Some(1001), Some(0))
            .expect_err("conflicting target selection to fail");
        assert_eq!(
            error.to_string(),
            "validation error: only one of --course-id or --course-index may be set"
        );
    }

    #[test]
    fn validates_exam_run_target_selection() {
        let error =
            ExamRunTarget::new(None, None, Some(555001), None).expect_err("course selection");
        assert_eq!(
            error.to_string(),
            "validation error: one of --course-id or --course-index is required"
        );

        let error = ExamRunTarget::new(Some(1001), None, None, None).expect_err("exam selection");
        assert_eq!(
            error.to_string(),
            "validation error: one of --exam-id or --exam-index is required"
        );

        let error = ExamRunTarget::new(Some(1001), None, Some(555001), Some(0))
            .expect_err("conflicting exam selection");
        assert_eq!(
            error.to_string(),
            "validation error: only one of --exam-id or --exam-index may be set"
        );
    }

    #[test]
    fn builds_a_deterministic_exam_run_plan() {
        let plan = ExamRunPlan::from_preview_snapshot(
            sample_course(),
            sample_exam(),
            sample_preview_query(),
            vec![
                sample_question(
                    2,
                    9003,
                    3,
                    "判断题",
                    "Rust trait object is sized.",
                    vec![],
                    vec![],
                ),
                sample_question(
                    0,
                    9001,
                    0,
                    "单选题",
                    "Rust ownership rules are enforced at compile time.",
                    vec![
                        ExamQuestionOption {
                            key: "A".to_owned(),
                            value: "Yes".to_owned(),
                            rich_content: None,
                        },
                        ExamQuestionOption {
                            key: "B".to_owned(),
                            value: "No".to_owned(),
                            rich_content: None,
                        },
                    ],
                    vec![],
                ),
                sample_question(
                    1,
                    9002,
                    2,
                    "填空题",
                    "The borrow checker prevents data ____.",
                    vec![],
                    vec!["race"],
                ),
            ],
        );

        assert_eq!(plan.course.course_id, 1001);
        assert_eq!(plan.exam.exam_id, 555001);
        assert_eq!(plan.preview.exam_answer_id, 777001);
        assert_eq!(plan.total_questions, 3);
        assert_eq!(
            plan.iter_questions()
                .map(|question| question.question_id)
                .collect::<Vec<_>>(),
            vec![9001, 9002, 9003]
        );
        assert_eq!(
            plan.questions[0].question_kind,
            ExamRunQuestionKind::SingleChoice
        );
        assert_eq!(
            plan.questions[1].question_kind,
            ExamRunQuestionKind::FillBlank
        );
        assert_eq!(
            plan.questions[2].question_kind,
            ExamRunQuestionKind::TrueFalse
        );
        assert_eq!(plan.questions[0].options.len(), 2);
        assert_eq!(plan.questions[1].blanks, vec!["race".to_owned()]);
    }

    #[test]
    fn preserves_unknown_exam_question_types_for_future_executor_support() {
        let plan = ExamRunPlan::from_preview_snapshot(
            sample_course(),
            sample_exam(),
            sample_preview_query(),
            vec![sample_question(
                0,
                9010,
                99,
                "未知题型",
                "Read the passage and answer the question.",
                vec![],
                vec![],
            )],
        );

        assert_eq!(plan.total_questions, 1);
        assert_eq!(
            plan.questions[0].question_kind,
            ExamRunQuestionKind::Unknown(99)
        );
        assert_eq!(plan.questions[0].question_type_label, "未知题型");
        assert_eq!(
            plan.exam.meta.as_ref().and_then(|meta| meta.exam_answer_id),
            Some(777001)
        );
    }

    #[test]
    fn classifies_extended_legacy_exam_question_types() {
        let cases = [
            (4, ExamRunQuestionKind::ShortAnswer),
            (5, ExamRunQuestionKind::TermExplanation),
            (6, ExamRunQuestionKind::Essay),
            (7, ExamRunQuestionKind::Calculation),
            (8, ExamRunQuestionKind::Other),
            (9, ExamRunQuestionKind::JournalEntry),
            (10, ExamRunQuestionKind::Material),
            (11, ExamRunQuestionKind::Matching),
            (13, ExamRunQuestionKind::Ordering),
            (14, ExamRunQuestionKind::Cloze),
            (15, ExamRunQuestionKind::ReadingComprehension),
            (18, ExamRunQuestionKind::Spoken),
            (19, ExamRunQuestionKind::Listening),
            (20, ExamRunQuestionKind::SharedOption),
            (21, ExamRunQuestionKind::Assessment),
        ];

        for (question_type, expected_kind) in cases {
            assert_eq!(
                ExamRunQuestionKind::from_question_type(question_type),
                expected_kind
            );
        }

        assert_eq!(
            ExamRunQuestionKind::from_question_type(99),
            ExamRunQuestionKind::Unknown(99)
        );
    }

    #[tokio::test]
    async fn builds_a_fixture_backed_course_run_plan() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let target = CourseRunTarget::new(Some(1001), None).expect("target");

        let plan = runner
            .build_plan(target)
            .await
            .expect("fixture-backed run plan");

        assert_eq!(plan.course.course_id, 1001);
        assert_eq!(plan.course.name, "现代汉语");
        assert_eq!(plan.chapters.len(), 2);
        assert_eq!(plan.total_task_points, 3);
        assert_eq!(plan.execution_queue.len(), 3);
        assert_eq!(plan.execution_queue[0].queue_index, 0);
        assert_eq!(plan.execution_queue[0].chapter_id, 11);
        assert_eq!(plan.chapters[0].chapter_id, 11);
        assert_eq!(
            plan.chapters[0].task_points[0].iframe_data.as_deref(),
            Some("{\"objectid\":\"video-001\"}")
        );
        assert_eq!(
            plan.execution_queue[1].iframe_data.as_deref(),
            Some("{\"workid\":\"work-001\",\"_jobid\":\"job-001\"}")
        );
        assert_eq!(
            plan.chapters[0].task_points[0].task_kind,
            CourseRunTaskKind::Video
        );
        assert_eq!(
            plan.chapters[0].task_points[0]
                .attachment_metadata
                .as_ref()
                .and_then(|attachment| attachment.fid),
            Some(9876)
        );
        assert_eq!(
            plan.chapters[0].task_points[0]
                .attachment_metadata
                .as_ref()
                .and_then(|attachment| attachment.video_status.as_ref())
                .map(|status| status.duration_secs),
            Some(602)
        );
        assert_eq!(
            plan.chapters[0].task_points[1].task_kind,
            CourseRunTaskKind::ChapterWork
        );
        assert_eq!(
            plan.chapters[1].task_points[0].task_kind,
            CourseRunTaskKind::Document
        );
        assert_eq!(
            plan.chapters[1].task_points[0]
                .attachment_metadata
                .as_ref()
                .and_then(|attachment| attachment.attachment.as_ref())
                .and_then(|attachment| attachment.file_type.as_deref()),
            Some("pdf")
        );
        assert_eq!(
            plan.execution_queue[2]
                .attachment_metadata
                .as_ref()
                .and_then(|attachment| attachment.attachment.as_ref())
                .and_then(|attachment| attachment.file_type.as_deref()),
            Some("pdf")
        );
    }

    #[tokio::test]
    async fn builds_a_fixture_backed_live_course_run_plan() {
        let fixture_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy_live");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let target = CourseRunTarget::new(Some(1001), None).expect("target");

        let plan = runner
            .build_plan(target)
            .await
            .expect("fixture-backed live run plan");

        assert_eq!(plan.course.course_id, 1001);
        assert_eq!(plan.chapters.len(), 1);
        assert_eq!(plan.total_task_points, 1);
        assert_eq!(plan.execution_queue.len(), 1);
        assert_eq!(plan.chapters[0].chapter_id, 13);
        assert_eq!(plan.chapters[0].task_points[0].module, "insertlive");
        assert_eq!(
            plan.chapters[0].task_points[0].task_kind,
            CourseRunTaskKind::Live
        );
        assert_eq!(
            plan.chapters[0].task_points[0].iframe_data.as_deref(),
            Some(
                "{\"liveId\":\"live-course-001\",\"vdoid\":\"vdo-live-001\",\"streamName\":\"zhibo_12345\"}"
            )
        );
        assert_eq!(
            plan.chapters[0].task_points[0]
                .attachment_metadata
                .as_ref()
                .and_then(|attachment| attachment.attachment.as_ref())
                .and_then(|attachment| attachment.live_id.as_deref()),
            Some("live-course-001")
        );
        assert_eq!(
            plan.execution_queue[0]
                .attachment_metadata
                .as_ref()
                .and_then(|attachment| attachment.attachment.as_ref())
                .and_then(|attachment| attachment.stream_name.as_deref()),
            Some("zhibo_12345")
        );
    }

    #[tokio::test]
    async fn resolves_fixture_backed_live_queue_entries_as_registered() {
        let fixture_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy_live");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let registry = TaskExecutorRegistry::new();
        let target = CourseRunTarget::new(Some(1001), None).expect("target");

        let plan = runner
            .build_plan(target)
            .await
            .expect("fixture-backed live run plan");
        let preflight = plan.build_execution_preflight(&registry);

        assert_eq!(preflight.total_entries, 1);
        assert!(preflight.all_entries_registered);
        assert_eq!(preflight.registered_entries, 1);
        assert_eq!(preflight.blocked_entries, 0);
        assert_eq!(
            preflight.queue[0].resolution,
            TaskExecutorResolution::Registered {
                registration: crate::task_executor::TaskExecutorRegistration {
                    key: TaskExecutorKey::Live,
                    module: "insertlive",
                },
            }
        );
    }

    #[tokio::test]
    async fn resolves_fixture_backed_execution_queue_in_registry_order() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let registry = TaskExecutorRegistry::new();
        let target = CourseRunTarget::new(Some(1001), None).expect("target");

        let plan = runner
            .build_plan(target)
            .await
            .expect("fixture-backed run plan");

        let resolutions = plan
            .iter_resolved_execution_queue(&registry)
            .map(|resolved| {
                (
                    resolved.entry.queue_index,
                    resolved.entry.chapter_id,
                    resolved.entry.module.clone(),
                    resolved.resolution,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            resolutions,
            vec![
                (
                    0,
                    11,
                    "insertvideo".to_owned(),
                    TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::Video,
                            module: "insertvideo",
                        },
                    },
                ),
                (
                    1,
                    11,
                    "work".to_owned(),
                    TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::ChapterWork,
                            module: "work",
                        },
                    },
                ),
                (
                    2,
                    12,
                    "insertdoc".to_owned(),
                    TaskExecutorResolution::Registered {
                        registration: crate::task_executor::TaskExecutorRegistration {
                            key: TaskExecutorKey::Document,
                            module: "insertdoc",
                        },
                    },
                ),
            ]
        );
    }

    #[tokio::test]
    async fn builds_fixture_backed_execution_preflight_summary() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = CourseRunner::new(&client);
        let registry = TaskExecutorRegistry::new();
        let target = CourseRunTarget::new(Some(1001), None).expect("target");

        let plan = runner
            .build_plan(target)
            .await
            .expect("fixture-backed run plan");
        let preflight = plan.build_execution_preflight(&registry);

        assert!(preflight.all_entries_registered);
        assert_eq!(preflight.total_entries, 3);
        assert_eq!(preflight.registered_entries, 3);
        assert_eq!(preflight.blocked_entries, 0);
        assert_eq!(preflight.queue.len(), 3);
        assert_eq!(preflight.queue[0].queue_index, 0);
        assert_eq!(
            preflight.queue[0].resolution,
            TaskExecutorResolution::Registered {
                registration: crate::task_executor::TaskExecutorRegistration {
                    key: TaskExecutorKey::Video,
                    module: "insertvideo",
                },
            }
        );
        assert_eq!(preflight.queue[1].queue_index, 1);
        assert_eq!(
            preflight.queue[1].resolution,
            TaskExecutorResolution::Registered {
                registration: crate::task_executor::TaskExecutorRegistration {
                    key: TaskExecutorKey::ChapterWork,
                    module: "work",
                },
            }
        );
        assert_eq!(preflight.queue[2].queue_index, 2);
        assert_eq!(
            preflight.queue[2].resolution,
            TaskExecutorResolution::Registered {
                registration: crate::task_executor::TaskExecutorRegistration {
                    key: TaskExecutorKey::Document,
                    module: "insertdoc",
                },
            }
        );
    }

    #[tokio::test]
    async fn emits_planning_lifecycle_events_for_fixture_backed_course_run_plan() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let sink = Arc::new(TestRunEventSink::default());
        let runner = CourseRunner::new(&client).with_sink(sink.clone());
        let target = CourseRunTarget::new(Some(1001), None).expect("target");

        let plan = runner
            .build_plan(target)
            .await
            .expect("fixture-backed run plan");

        assert_eq!(plan.course.course_id, 1001);
        assert_eq!(
            sink.snapshot(),
            vec![
                RunEvent::CourseRunPlanningStarted {
                    course_id: Some(1001),
                    course_index: None,
                },
                RunEvent::CourseRunPlanningFinished {
                    course_id: 1001,
                    chapters: 2,
                    task_points: 3,
                },
            ]
        );
    }

    #[tokio::test]
    async fn builds_a_fixture_backed_exam_run_plan() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = ExamRunner::new(&client);
        let target = ExamRunTarget::new(Some(1001), None, Some(555001), None).expect("target");

        let plan = runner
            .build_plan(&sample_account(), target)
            .await
            .expect("fixture-backed exam run plan");

        assert_eq!(plan.course.course_id, 1001);
        assert_eq!(plan.exam.exam_id, 555001);
        assert_eq!(
            plan.exam
                .meta
                .as_ref()
                .map(|meta| meta.entry_state.as_str()),
            Some("ready")
        );
        assert_eq!(plan.preview.exam_answer_id, 900001);
        assert_eq!(plan.total_questions, 4);
        assert_eq!(plan.questions[0].question_id, 700001);
        assert_eq!(
            plan.questions[0].question_kind,
            ExamRunQuestionKind::SingleChoice
        );
        assert_eq!(
            plan.questions[1].question_kind,
            ExamRunQuestionKind::MultipleChoice
        );
        assert_eq!(plan.questions[2].blanks.len(), 2);
        assert_eq!(
            plan.questions[3].question_kind,
            ExamRunQuestionKind::TrueFalse
        );
    }

    #[tokio::test]
    async fn emits_planning_lifecycle_events_for_fixture_backed_exam_run_plan() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let sink = Arc::new(TestRunEventSink::default());
        let runner = ExamRunner::new(&client).with_sink(sink.clone());
        let target = ExamRunTarget::new(Some(1001), None, Some(555001), None).expect("target");

        let plan = runner
            .build_plan(&sample_account(), target)
            .await
            .expect("fixture-backed exam run plan");

        assert_eq!(plan.course.course_id, 1001);
        assert_eq!(plan.exam.exam_id, 555001);
        assert_eq!(
            sink.snapshot(),
            vec![
                RunEvent::ExamRunPlanningStarted {
                    course_id: Some(1001),
                    course_index: None,
                    exam_id: Some(555001),
                    exam_index: None,
                },
                RunEvent::ExamRunPlanningFinished {
                    course_id: 1001,
                    exam_id: 555001,
                    questions: 4,
                },
            ]
        );
    }

    #[tokio::test]
    async fn fails_closed_when_exam_preview_metadata_is_unavailable() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let runner = ExamRunner::new(&client);
        let target = ExamRunTarget::new(Some(1001), None, Some(555002), None).expect("target");

        let error = runner
            .build_plan(&sample_account(), target)
            .await
            .expect_err("completed exam without preview metadata to fail closed");

        assert_eq!(
            error.to_string(),
            "validation error: exam run planning requires exam_answer_id from read-only exam cover metadata"
        );
    }
}
