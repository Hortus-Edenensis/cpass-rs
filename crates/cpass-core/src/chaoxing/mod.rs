use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use aes::Aes128;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cbc::Encryptor;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockEncryptMut, KeyIvInit};
use reqwest::cookie::Jar;
use reqwest::header::{HeaderValue, USER_AGENT};
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::config::TransportConfig;
use crate::error::{CpassError, Result};
use crate::models::{
    AccountProfile, Chapter, ChapterWorkAttachmentMetadata, ChapterWorkFormSnapshot,
    ChapterWorkMetadata, ChapterWorkQuestionSummary, ChapterWorkRuntimeRequest, CookieSnapshot,
    Course, CourseExam, DocumentReadingReportAck, DocumentReadingReportRequest, ExamMeta,
    ExamPreviewQuery, ExamQuestionOption, ExamQuestionSummary, LiveProgressReportAck,
    LiveProgressReportRequest, QuestionRichContent, TaskAttachment, TaskAttachmentSnapshot,
    TaskPointAttachmentMetadata, TaskPointSummary, VideoAttachmentStatus, VideoPlayReportAck,
    VideoPlayReportRequest,
};
use crate::question_kind::NormalizedQuestionKind;
use crate::transport::{ChaoxingTransport, ReqwestChaoxingTransport, TransportRequest};

const API_LOGIN_WEB: &str = "https://passport2.chaoxing.com/fanyalogin";
const API_QRCREATE: &str = "https://passport2.chaoxing.com/createqr";
const API_QRLOGIN: &str = "https://passport2.chaoxing.com/getauthstatus";
const PAGE_LOGIN: &str = "https://passport2.chaoxing.com/login";
const URL_QRLOGIN: &str = "https://passport2.chaoxing.com/toauthlogin";
const API_CLASS_LIST: &str = "https://mooc1-api.chaoxing.com/mycourse/backclazzdata";
const API_SSO_LOGIN: &str = "https://sso.chaoxing.com/apis/login/userLogin4Uname.do";
const API_CHAPTER_LIST: &str = "https://mooc1-api.chaoxing.com/gas/clazz";
const API_CHAPTER_STATUS: &str = "https://mooc1-api.chaoxing.com/job/myjobsnodesmap";
const API_CHAPTER_CARDS: &str = "https://mooc1-api.chaoxing.com/gas/knowledge";
const PAGE_CHAPTER_CARD_ATTACHMENT: &str = "https://mooc1-api.chaoxing.com/knowledge/cards";
const PAGE_CHAPTER_WORK: &str = "https://mooc1-api.chaoxing.com/android/mworkspecial";
const API_ATTACHMENT_STATUS: &str = "https://mooc1-api.chaoxing.com/ananas/status";
const API_VIDEO_PLAYREPORT: &str = "https://mooc1-api.chaoxing.com/multimedia/log/a";
const API_DOCUMENT_READINGREPORT: &str = "https://mooc1.chaoxing.com/ananas/job/document";
const API_LIVE_PROGRESS_REPORT: &str = "https://zhibo.chaoxing.com/saveTimePc";
const PAGE_EXAM_COVER: &str = "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/task-exam";
const PAGE_EXAM_LIST: &str = "https://mooc1-api.chaoxing.com/exam/phone/task-list";
const PAGE_EXAM_PREVIEW: &str = "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/preview";
const WEB_LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrLoginBootstrap {
    pub uuid: String,
    pub enc: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrLoginStatus {
    Pending,
    Scanned { nickname: String, uid: String },
    Success,
    Expired,
    Failed { message: String },
}

#[derive(Clone)]
pub struct QrLoginFlow {
    client: ChaoxingClient,
    jar: Option<Arc<Jar>>,
    uuid: String,
    enc: String,
}

pub enum QrLoginPollOutcome {
    Pending,
    Scanned {
        nickname: String,
        uid: String,
    },
    Success {
        client: ChaoxingClient,
        account: AccountProfile,
        snapshot: CookieSnapshot,
    },
    Expired,
    Failed {
        message: String,
    },
}

#[derive(Clone)]
pub struct ChaoxingClient {
    transport: Arc<dyn ChaoxingTransport>,
}

impl ChaoxingClient {
    pub fn new(transport: Arc<dyn ChaoxingTransport>) -> Self {
        Self { transport }
    }

    pub async fn login_password(
        phone: &str,
        password: &str,
        transport_config: TransportConfig,
    ) -> Result<(Self, AccountProfile, CookieSnapshot)> {
        let (transport, jar) = ReqwestChaoxingTransport::with_cookie_jar(transport_config)?;
        let client = Self::new(Arc::new(transport));

        let mut form = BTreeMap::new();
        form.insert("fid".to_owned(), "-1".to_owned());
        form.insert("uname".to_owned(), encrypt_login_value(phone)?);
        form.insert("password".to_owned(), encrypt_login_value(password)?);
        form.insert("t".to_owned(), "true".to_owned());
        form.insert("forbidotherlogin".to_owned(), "0".to_owned());
        form.insert("validate".to_owned(), String::new());

        let response = client
            .transport
            .execute(TransportRequest::post_form(API_LOGIN_WEB, form))
            .await?;
        let value: Value = response.json()?;
        let status = value
            .get("status")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !status {
            let message = value
                .get("msg2")
                .or_else(|| value.get("msg"))
                .and_then(Value::as_str)
                .unwrap_or("unknown login failure")
                .to_owned();
            return Err(CpassError::LoginFailed(message));
        }

        let account = client.fetch_account_info().await?;
        let snapshot = ReqwestChaoxingTransport::snapshot_from_jar(jar.as_ref());
        Ok((client, account, snapshot))
    }

    pub async fn begin_qr_login(transport_config: TransportConfig) -> Result<QrLoginFlow> {
        let (transport, jar) = ReqwestChaoxingTransport::with_cookie_jar(transport_config)?;
        QrLoginFlow::begin_with_transport(Arc::new(transport), Some(jar)).await
    }

    pub async fn fetch_account_info(&self) -> Result<AccountProfile> {
        let response = self
            .transport
            .execute(TransportRequest::get(API_SSO_LOGIN))
            .await?;
        let value: Value = response.json()?;
        if value.get("result").and_then(Value::as_i64) == Some(0) {
            return Err(CpassError::LoginFailed(
                "session is invalid; run cpass login again".to_owned(),
            ));
        }

        let msg = value.get("msg").and_then(Value::as_object).ok_or_else(|| {
            CpassError::UnexpectedResponse("missing account info payload".to_owned())
        })?;

        let sex = match msg.get("sex").and_then(Value::as_i64) {
            Some(1) => Some("male".to_owned()),
            Some(0) => Some("female".to_owned()),
            Some(_) => Some("unknown".to_owned()),
            None => None,
        };

        Ok(AccountProfile {
            puid: msg
                .get("puid")
                .and_then(Value::as_u64)
                .ok_or_else(|| CpassError::UnexpectedResponse("missing puid".to_owned()))?,
            name: msg
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            phone: msg
                .get("phone")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            school: msg
                .get("schoolname")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            sex,
            student_id: msg.get("uname").and_then(Value::as_str).map(str::to_owned),
        })
    }

    pub async fn fetch_courses(&self) -> Result<Vec<Course>> {
        let response = self
            .transport
            .execute(TransportRequest::get(API_CLASS_LIST))
            .await?;
        let value: Value = response.json()?;
        parse_courses(&value)
    }

    pub async fn fetch_chapters(&self, course: &Course) -> Result<Vec<Chapter>> {
        let mut request = TransportRequest::get(API_CHAPTER_LIST);
        request.query = vec![
            ("id".to_owned(), course.key.to_string()),
            ("personid".to_owned(), course.cpi.to_string()),
            (
                "fields".to_owned(),
                "id,bbsid,classscore,isstart,allowdownload,chatid,name,state,isfiled,visiblescore,begindate,coursesetting.fields(id,courseid,hiddencoursecover,coursefacecheck),course.fields(id,name,infocontent,objectid,app,bulletformat,mappingcourseid,imageurl,teacherfactor,jobcount,knowledge.fields(id,name,indexOrder,parentnodeid,status,layer,label,jobcount,begintime,endtime,attachment.fields(id,type,objectid,extension).type(video)))".to_owned(),
            ),
            ("view".to_owned(), "json".to_owned()),
        ];
        let response = self.transport.execute(request).await?;
        let value: Value = response.json()?;
        parse_chapters(&value)
    }

    pub async fn fetch_task_scan(&self, course: &Course) -> Result<Vec<Chapter>> {
        let mut chapters = self.fetch_chapters(course).await?;
        let mut form = BTreeMap::new();
        form.insert("view".to_owned(), "json".to_owned());
        form.insert(
            "nodes".to_owned(),
            chapters
                .iter()
                .map(|chapter| chapter.chapter_id.to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
        form.insert("clazzid".to_owned(), course.class_id.to_string());
        form.insert(
            "time".to_owned(),
            chrono::Utc::now().timestamp_millis().to_string(),
        );
        form.insert("userid".to_owned(), course.cpi.to_string());
        form.insert("cpi".to_owned(), course.cpi.to_string());
        form.insert("courseid".to_owned(), course.course_id.to_string());

        let response = self
            .transport
            .execute(TransportRequest::post_form(API_CHAPTER_STATUS, form))
            .await?;
        let value: Value = response.json()?;
        merge_chapter_progress(&mut chapters, &value);
        for chapter in &mut chapters {
            chapter.task_points = self.fetch_task_points(course, chapter.chapter_id).await?;
        }
        Ok(chapters)
    }

    pub async fn fetch_task_scan_with_attachment_metadata(
        &self,
        course: &Course,
    ) -> Result<Vec<Chapter>> {
        let mut chapters = self.fetch_task_scan(course).await?;
        let mut card_snapshot_cache = BTreeMap::new();
        let mut video_status_cache = BTreeMap::new();

        for chapter in &mut chapters {
            for task_point in &mut chapter.task_points {
                task_point.attachment_metadata = self
                    .fetch_task_point_attachment_metadata(
                        course,
                        chapter.chapter_id,
                        task_point,
                        &mut card_snapshot_cache,
                        &mut video_status_cache,
                    )
                    .await?;
            }
        }

        Ok(chapters)
    }

    pub async fn fetch_task_points(
        &self,
        course: &Course,
        chapter_id: u64,
    ) -> Result<Vec<TaskPointSummary>> {
        let mut request = TransportRequest::get(API_CHAPTER_CARDS);
        request.query = vec![
            ("id".to_owned(), chapter_id.to_string()),
            ("courseid".to_owned(), course.course_id.to_string()),
            (
                "fields".to_owned(),
                "id,parentnodeid,indexorder,label,layer,name,begintime,createtime,lastmodifytime,status,jobUnfinishedCount,clickcount,openlock,card.fields(id,knowledgeid,title,knowledgeTitile,description,cardorder).contentcard(all)".to_owned(),
            ),
            ("view".to_owned(), "json".to_owned()),
            (
                "token".to_owned(),
                "4faa8662c59590c6f43ae9fe5b002b42".to_owned(),
            ),
        ];
        let response = self.transport.execute(request).await?;
        let value: Value = response.json()?;
        parse_task_points(&value)
    }

    pub async fn fetch_task_card_attachments(
        &self,
        course: &Course,
        chapter_id: u64,
        card_index: usize,
    ) -> Result<TaskAttachmentSnapshot> {
        let mut request = TransportRequest::get(PAGE_CHAPTER_CARD_ATTACHMENT);
        request.query = vec![
            ("clazzid".to_owned(), course.class_id.to_string()),
            ("courseid".to_owned(), course.course_id.to_string()),
            ("knowledgeid".to_owned(), chapter_id.to_string()),
            ("num".to_owned(), card_index.to_string()),
            ("isPhone".to_owned(), "1".to_owned()),
            ("control".to_owned(), "true".to_owned()),
            ("cpi".to_owned(), course.cpi.to_string()),
        ];
        let response = self.transport.execute(request).await?;
        parse_task_card_attachments(&response.body)
    }

    pub async fn fetch_chapter_work_attachment_metadata(
        &self,
        request: &ChapterWorkRuntimeRequest,
    ) -> Result<ChapterWorkAttachmentMetadata> {
        let mut attachment_request = TransportRequest::get(PAGE_CHAPTER_CARD_ATTACHMENT);
        attachment_request.query = vec![
            ("clazzid".to_owned(), request.class_id.to_string()),
            ("courseid".to_owned(), request.course_id.to_string()),
            ("knowledgeid".to_owned(), request.knowledge_id.to_string()),
            ("num".to_owned(), request.card_index.to_string()),
            ("isPhone".to_owned(), "1".to_owned()),
            ("control".to_owned(), "true".to_owned()),
            ("cpi".to_owned(), request.cpi.to_string()),
        ];
        let response = self.transport.execute(attachment_request).await?;
        let snapshot = parse_task_card_attachments(&response.body)?;
        parse_chapter_work_attachment_metadata(&snapshot, &request.work_id, &request.job_id)
    }

    pub async fn fetch_chapter_work_form(
        &self,
        request: &ChapterWorkRuntimeRequest,
    ) -> Result<ChapterWorkFormSnapshot> {
        let attachment = self.fetch_chapter_work_attachment_metadata(request).await?;
        let response = self
            .transport
            .execute(build_chapter_work_form_transport_request(
                request,
                &attachment,
            ))
            .await?;
        parse_chapter_work_form(&response.body)
    }

    pub async fn fetch_video_attachment_status(
        &self,
        fid: u64,
        object_id: &str,
    ) -> Result<VideoAttachmentStatus> {
        let mut request = TransportRequest::get(format!("{API_ATTACHMENT_STATUS}/{object_id}"));
        request.query = vec![
            ("k".to_owned(), fid.to_string()),
            ("flag".to_owned(), "normal".to_owned()),
            (
                "_dc".to_owned(),
                chrono::Utc::now().timestamp_millis().to_string(),
            ),
        ];
        let response = self.transport.execute(request).await?;
        let value: Value = response.json()?;
        parse_video_attachment_status(&value)
    }

    pub async fn report_video_progress(
        &self,
        report: &VideoPlayReportRequest,
    ) -> Result<VideoPlayReportAck> {
        let response = self
            .transport
            .execute(build_video_play_report_transport_request(report))
            .await?;
        let value: Value = response.json()?;
        parse_video_play_report_ack(&value)
    }

    pub async fn report_document_progress(
        &self,
        report: &DocumentReadingReportRequest,
    ) -> Result<DocumentReadingReportAck> {
        let response = self
            .transport
            .execute(build_document_reading_report_transport_request(report))
            .await?;
        let value: Value = response.json()?;
        parse_document_reading_report_ack(&value)
    }

    pub async fn report_live_progress(
        &self,
        report: &LiveProgressReportRequest,
    ) -> Result<LiveProgressReportAck> {
        let response = self
            .transport
            .execute(build_live_progress_report_transport_request(report))
            .await?;
        parse_live_progress_report_ack(&response.body)
    }

    pub async fn fetch_exams(&self, course: &Course) -> Result<Vec<CourseExam>> {
        let mut request = TransportRequest::get(PAGE_EXAM_LIST);
        request.query = vec![
            ("courseId".to_owned(), course.course_id.to_string()),
            ("classId".to_owned(), course.class_id.to_string()),
            ("cpi".to_owned(), course.cpi.to_string()),
        ];
        let response = self.transport.execute(request).await?;
        parse_exams(&response.body, course)
    }

    pub async fn fetch_exam_meta(
        &self,
        course: &Course,
        account: &AccountProfile,
        exam: &CourseExam,
    ) -> Result<ExamMeta> {
        let mut request = TransportRequest::get(PAGE_EXAM_COVER);
        request.query = vec![
            ("redo".to_owned(), "1".to_owned()),
            ("taskrefId".to_owned(), exam.exam_id.to_string()),
            ("courseId".to_owned(), course.course_id.to_string()),
            ("classId".to_owned(), course.class_id.to_string()),
            ("userId".to_owned(), account.puid.to_string()),
            ("role".to_owned(), String::new()),
            ("source".to_owned(), "0".to_owned()),
            ("enc_task".to_owned(), exam.enc_task.clone()),
            ("cpi".to_owned(), course.cpi.to_string()),
            ("vx".to_owned(), "0".to_owned()),
            ("examsignal".to_owned(), "1".to_owned()),
        ];
        let response = self.transport.execute(request).await?;
        parse_exam_meta(&response.body, &response.final_url)
    }

    pub async fn fetch_exam_preview_questions(
        &self,
        course: &Course,
        exam: &CourseExam,
        preview: &ExamPreviewQuery,
    ) -> Result<Vec<ExamQuestionSummary>> {
        if preview.exam_answer_id == 0 {
            return Err(CpassError::Validation(
                "exam preview requires exam_answer_id".to_owned(),
            ));
        }

        let mut request = TransportRequest::get(PAGE_EXAM_PREVIEW);
        request.query = vec![
            ("courseId".to_owned(), course.course_id.to_string()),
            ("classId".to_owned(), course.class_id.to_string()),
            ("source".to_owned(), "0".to_owned()),
            ("imei".to_owned(), "cpass-rs-preview".to_owned()),
            ("start".to_owned(), "0".to_owned()),
            ("cpi".to_owned(), course.cpi.to_string()),
            ("examRelationId".to_owned(), exam.exam_id.to_string()),
            (
                "examRelationAnswerId".to_owned(),
                preview.exam_answer_id.to_string(),
            ),
            ("monitorStatus".to_owned(), "0".to_owned()),
            ("monitorOp".to_owned(), "-1".to_owned()),
            (
                "remainTimeParam".to_owned(),
                preview.remain_time_param.unwrap_or(0).to_string(),
            ),
            (
                "relationAnswerLastUpdateTime".to_owned(),
                preview
                    .relation_answer_last_update_time
                    .unwrap_or(0)
                    .to_string(),
            ),
            ("enc".to_owned(), preview.enc.clone()),
        ];
        let response = self.transport.execute(request).await?;
        parse_exam_preview_questions(&response.body)
    }

    async fn fetch_task_point_attachment_metadata(
        &self,
        course: &Course,
        chapter_id: u64,
        task_point: &TaskPointSummary,
        card_snapshot_cache: &mut BTreeMap<(u64, usize), TaskAttachmentSnapshot>,
        video_status_cache: &mut BTreeMap<String, VideoAttachmentStatus>,
    ) -> Result<Option<TaskPointAttachmentMetadata>> {
        if !matches!(
            task_point.module.as_str(),
            "insertvideo" | "insertdoc" | "insertlive"
        ) {
            return Ok(None);
        }

        let cache_key = (chapter_id, task_point.card_index);
        let snapshot = match card_snapshot_cache.get(&cache_key) {
            Some(snapshot) => snapshot.clone(),
            None => {
                let snapshot = self
                    .fetch_task_card_attachments(course, chapter_id, task_point.card_index)
                    .await?;
                card_snapshot_cache.insert(cache_key, snapshot.clone());
                snapshot
            }
        };

        let Some(attachment) = snapshot
            .attachments
            .iter()
            .find(|attachment| attachment_matches_task_point(attachment, task_point))
            .cloned()
        else {
            return Ok(None);
        };

        let video_status = if task_point.module == "insertvideo" {
            let object_id = attachment.object_id.as_deref().ok_or_else(|| {
                CpassError::UnexpectedResponse("missing video attachment object id".to_owned())
            })?;
            match video_status_cache.get(object_id) {
                Some(status) => Some(status.clone()),
                None => {
                    let fid = snapshot.fid.ok_or_else(|| {
                        CpassError::UnexpectedResponse("missing video attachment fid".to_owned())
                    })?;
                    let status = self.fetch_video_attachment_status(fid, object_id).await?;
                    video_status_cache.insert(object_id.to_owned(), status.clone());
                    Some(status)
                }
            }
        } else {
            None
        };

        Ok(Some(TaskPointAttachmentMetadata {
            fid: snapshot.fid,
            attachment: Some(attachment),
            video_status,
        }))
    }
}

impl QrLoginFlow {
    pub async fn begin_with_transport(
        transport: Arc<dyn ChaoxingTransport>,
        jar: Option<Arc<Jar>>,
    ) -> Result<Self> {
        let client = ChaoxingClient::new(transport);
        let mut login_request = TransportRequest::get(PAGE_LOGIN);
        login_request
            .headers
            .insert(USER_AGENT, HeaderValue::from_static(WEB_LOGIN_USER_AGENT));
        let login_page = client.transport.execute(login_request).await?;
        let bootstrap = parse_qr_login_bootstrap(&login_page.body)?;

        let mut activate_request = TransportRequest::get(API_QRCREATE);
        activate_request.query = vec![
            ("uuid".to_owned(), bootstrap.uuid.clone()),
            ("fid".to_owned(), "-1".to_owned()),
        ];
        activate_request
            .headers
            .insert(USER_AGENT, HeaderValue::from_static(WEB_LOGIN_USER_AGENT));
        let _ = client.transport.execute(activate_request).await?;

        Ok(Self {
            client,
            jar,
            uuid: bootstrap.uuid,
            enc: bootstrap.enc,
        })
    }

    #[must_use]
    pub fn qr_url(&self) -> String {
        Url::parse_with_params(
            URL_QRLOGIN,
            [
                ("uuid", self.uuid.as_str()),
                ("enc", self.enc.as_str()),
                ("xxtrefer", ""),
                ("clientid", ""),
                ("mobiletip", ""),
            ],
        )
        .expect("static qr login url to be valid")
        .to_string()
    }

    pub async fn poll(&self) -> Result<QrLoginPollOutcome> {
        let mut request = TransportRequest::post_form(
            API_QRLOGIN,
            BTreeMap::from([
                ("enc".to_owned(), self.enc.clone()),
                ("uuid".to_owned(), self.uuid.clone()),
            ]),
        );
        request
            .headers
            .insert(USER_AGENT, HeaderValue::from_static(WEB_LOGIN_USER_AGENT));
        let response = self.client.transport.execute(request).await?;
        let value: Value = response.json()?;
        match parse_qr_login_status(&value)? {
            QrLoginStatus::Pending => Ok(QrLoginPollOutcome::Pending),
            QrLoginStatus::Scanned { nickname, uid } => {
                Ok(QrLoginPollOutcome::Scanned { nickname, uid })
            }
            QrLoginStatus::Success => {
                let account = self.client.fetch_account_info().await?;
                let snapshot = self
                    .jar
                    .as_deref()
                    .map(ReqwestChaoxingTransport::snapshot_from_jar)
                    .unwrap_or_else(CookieSnapshot::empty);
                Ok(QrLoginPollOutcome::Success {
                    client: self.client.clone(),
                    account,
                    snapshot,
                })
            }
            QrLoginStatus::Expired => Ok(QrLoginPollOutcome::Expired),
            QrLoginStatus::Failed { message } => Ok(QrLoginPollOutcome::Failed { message }),
        }
    }
}

fn attachment_matches_task_point(
    attachment: &TaskAttachment,
    task_point: &TaskPointSummary,
) -> bool {
    let resource_id = task_point.resource_id.as_str();
    [
        attachment.object_id.as_deref(),
        attachment.work_id.as_deref(),
        attachment.live_id.as_deref(),
        attachment.vdo_id.as_deref(),
        attachment.stream_name.as_deref(),
        attachment.job_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|candidate| candidate == resource_id)
}

fn encrypt_login_value(value: &str) -> Result<String> {
    type Aes128CbcEnc = Encryptor<Aes128>;
    let key = b"u2oh6Vu^HWe4_AES";
    let mut buffer = value.as_bytes().to_vec();
    let message_len = buffer.len();
    let pad_room = 16 - (message_len % 16);
    buffer.resize(message_len + pad_room, 0);
    let encrypted = Aes128CbcEnc::new(key.into(), key.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, message_len)
        .map_err(|err| {
            CpassError::UnexpectedResponse(format!("failed to encrypt login value: {err}"))
        })?;
    Ok(STANDARD.encode(encrypted))
}

pub fn parse_qr_login_bootstrap(html: &str) -> Result<QrLoginBootstrap> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("input").expect("valid qr login input selector");
    let mut uuid = None;
    let mut enc = None;

    for input in document.select(&selector) {
        match input.value().attr("id") {
            Some("uuid") => uuid = input.value().attr("value").map(str::to_owned),
            Some("enc") => enc = input.value().attr("value").map(str::to_owned),
            _ => {}
        }
    }

    Ok(QrLoginBootstrap {
        uuid: uuid.ok_or_else(|| {
            CpassError::UnexpectedResponse("qr login bootstrap page is missing uuid".to_owned())
        })?,
        enc: enc.ok_or_else(|| {
            CpassError::UnexpectedResponse("qr login bootstrap page is missing enc".to_owned())
        })?,
    })
}

pub fn parse_qr_login_status(value: &Value) -> Result<QrLoginStatus> {
    if value
        .get("status")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(QrLoginStatus::Success);
    }

    match value.get("type").and_then(Value::as_str) {
        Some("4") => Ok(QrLoginStatus::Scanned {
            nickname: value
                .get("nickname")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            uid: value
                .get("uid")
                .and_then(|uid| match uid {
                    Value::String(text) => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "unknown".to_owned()),
        }),
        Some("2") => Ok(QrLoginStatus::Expired),
        Some("1") => Ok(QrLoginStatus::Failed {
            message: value
                .get("msg")
                .or_else(|| value.get("msg2"))
                .and_then(Value::as_str)
                .unwrap_or("二维码验证错误")
                .to_owned(),
        }),
        Some(_) | None => Ok(QrLoginStatus::Pending),
    }
}

fn build_video_play_report_transport_request(report: &VideoPlayReportRequest) -> TransportRequest {
    let query = vec![
        ("otherInfo".to_owned(), report.other_info.clone()),
        (
            "playingTime".to_owned(),
            report.playing_time_secs.to_string(),
        ),
        ("duration".to_owned(), report.duration_secs.to_string()),
        ("jobid".to_owned(), report.job_id.clone()),
        ("clipTime".to_owned(), report.clip_time.clone()),
        ("clazzId".to_owned(), report.class_id.to_string()),
        ("objectId".to_owned(), report.object_id.clone()),
        ("userid".to_owned(), report.user_id.to_string()),
        ("isdrag".to_owned(), "0".to_owned()),
        ("enc".to_owned(), report.enc.clone()),
        ("rt".to_owned(), report.playback_rate.clone()),
        ("dtype".to_owned(), "Video".to_owned()),
        ("view".to_owned(), "pc".to_owned()),
        ("_t".to_owned(), report.report_timestamp_millis.to_string()),
    ];
    let raw_query = legacy_urlencode_query(&query);
    TransportRequest::get(format!(
        "{API_VIDEO_PLAYREPORT}/{}/{}",
        report.cpi, report.dtoken
    ))
    .with_raw_query(raw_query)
}

fn build_document_reading_report_transport_request(
    report: &DocumentReadingReportRequest,
) -> TransportRequest {
    let mut request = TransportRequest::get(API_DOCUMENT_READINGREPORT);
    request.query = vec![
        ("jobid".to_owned(), report.job_id.clone()),
        ("knowledgeid".to_owned(), report.knowledge_id.to_string()),
        ("courseid".to_owned(), report.course_id.to_string()),
        ("clazzid".to_owned(), report.class_id.to_string()),
        ("jtoken".to_owned(), report.jtoken.clone()),
        ("_dc".to_owned(), report.report_timestamp_millis.to_string()),
    ];
    request
}

fn build_live_progress_report_transport_request(
    report: &LiveProgressReportRequest,
) -> TransportRequest {
    let mut request = TransportRequest::get(API_LIVE_PROGRESS_REPORT);
    request.query = vec![
        ("streamName".to_owned(), report.stream_name.clone()),
        ("vdoid".to_owned(), report.vdo_id.clone()),
        ("userId".to_owned(), report.user_id.to_string()),
        (
            "isStart".to_owned(),
            if report.is_start { "1" } else { "0" }.to_owned(),
        ),
        ("t".to_owned(), report.report_timestamp_millis.to_string()),
        ("courseId".to_owned(), report.course_id.to_string()),
    ];
    request
}

fn build_chapter_work_form_transport_request(
    request: &ChapterWorkRuntimeRequest,
    attachment: &ChapterWorkAttachmentMetadata,
) -> TransportRequest {
    let mut transport_request = TransportRequest::get(PAGE_CHAPTER_WORK);
    transport_request.query = vec![
        ("courseid".to_owned(), request.course_id.to_string()),
        ("workid".to_owned(), request.relation_work_id()),
        ("jobid".to_owned(), request.job_id.clone()),
        ("needRedirect".to_owned(), "true".to_owned()),
        ("knowledgeid".to_owned(), request.knowledge_id.to_string()),
        ("userid".to_owned(), request.user_id.to_string()),
        ("ut".to_owned(), "s".to_owned()),
        ("clazzId".to_owned(), request.class_id.to_string()),
        ("cpi".to_owned(), request.cpi.to_string()),
        ("ktoken".to_owned(), attachment.ktoken.clone()),
        ("enc".to_owned(), attachment.enc.clone()),
    ];
    transport_request
}

fn legacy_urlencode_query(query: &[(String, String)]) -> String {
    query
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                legacy_urlencode_component(key),
                legacy_urlencode_component(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn legacy_urlencode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'&' | b'=' => {
                encoded.push(char::from(byte))
            }
            b' ' => encoded.push('+'),
            _ => {
                use std::fmt::Write as _;
                write!(&mut encoded, "%{byte:02X}").expect("write to string");
            }
        }
    }
    encoded
}

pub fn parse_courses(value: &Value) -> Result<Vec<Course>> {
    if value.get("result").and_then(Value::as_i64) != Some(1) {
        return Err(CpassError::UnexpectedResponse(
            "course list response did not include result=1".to_owned(),
        ));
    }

    let channels = value
        .get("channelList")
        .and_then(Value::as_array)
        .ok_or_else(|| CpassError::UnexpectedResponse("missing channelList".to_owned()))?;

    let mut courses = Vec::new();
    for channel in channels {
        let content = match channel.get("content").and_then(Value::as_object) {
            Some(content) => content,
            None => continue,
        };
        let course = match content
            .get("course")
            .and_then(Value::as_object)
            .and_then(|course| course.get("data"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_object)
        {
            Some(course) => course,
            None => continue,
        };
        courses.push(Course {
            course_id: course
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| CpassError::UnexpectedResponse("missing course id".to_owned()))?,
            class_id: content
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| CpassError::UnexpectedResponse("missing class id".to_owned()))?,
            cpi: channel
                .get("cpi")
                .and_then(Value::as_u64)
                .ok_or_else(|| CpassError::UnexpectedResponse("missing cpi".to_owned()))?,
            key: channel
                .get("key")
                .and_then(Value::as_u64)
                .ok_or_else(|| CpassError::UnexpectedResponse("missing course key".to_owned()))?,
            name: course
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .trim()
                .to_owned(),
            teacher_name: course
                .get("teacherfactor")
                .and_then(Value::as_str)
                .unwrap_or("未知")
                .to_owned(),
            state: content
                .get("state")
                .and_then(Value::as_i64)
                .map(|state| match state {
                    0 => "进行中".to_owned(),
                    1 => "已结课".to_owned(),
                    other => format!("未知({other})"),
                })
                .unwrap_or_else(|| "未知".to_owned()),
        });
    }
    Ok(courses)
}

pub fn parse_chapters(value: &Value) -> Result<Vec<Chapter>> {
    let chapters = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("course"))
        .and_then(|course| course.get("data"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|course| course.get("knowledge"))
        .and_then(|knowledge| knowledge.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| CpassError::UnexpectedResponse("missing chapter payload".to_owned()))?;

    let mut parsed = Vec::new();
    for chapter in chapters {
        parsed.push(Chapter {
            chapter_id: chapter
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| CpassError::UnexpectedResponse("missing chapter id".to_owned()))?,
            jobs: chapter.get("jobcount").and_then(Value::as_u64).unwrap_or(0),
            index: chapter
                .get("indexorder")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            name: chapter
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .trim()
                .to_owned(),
            label: chapter
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("0")
                .to_owned(),
            layer: chapter.get("layer").and_then(Value::as_u64).unwrap_or(0),
            status: chapter
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            point_total: 0,
            point_finished: 0,
            task_points: Vec::new(),
        });
    }
    parsed.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(parsed)
}

pub fn merge_chapter_progress(chapters: &mut [Chapter], value: &Value) {
    for chapter in chapters {
        let Some(point_data) = value.get(chapter.chapter_id.to_string()) else {
            continue;
        };
        let total = point_data
            .get("totalcount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let unfinished = point_data
            .get("unfinishcount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let finished = point_data
            .get("finishcount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        chapter.point_total = if unfinished != 0 && total == 0 {
            unfinished
        } else {
            total
        };
        chapter.point_finished = finished;
    }
}

pub fn parse_exams(html: &str, course: &Course) -> Result<Vec<CourseExam>> {
    let document = Html::parse_document(html);
    let nav_selector = Selector::parse("ul.nav li")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let name_selector = Selector::parse("p")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let status_selector = Selector::parse("span")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let expire_selector = Selector::parse("span.fr")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;

    let mut exams = Vec::new();
    for node in document.select(&nav_selector) {
        let Some(data) = node.value().attr("data") else {
            continue;
        };
        let url =
            Url::parse(data).or_else(|_| Url::parse(&format!("https://unused.local/{data}")))?;
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        let name = node
            .select(&name_selector)
            .next()
            .map(|node| node.text().collect::<String>().trim().to_owned())
            .unwrap_or_else(|| "未命名考试".to_owned());
        let status = node
            .select(&status_selector)
            .next()
            .map(|node| node.text().collect::<String>().trim().to_owned())
            .unwrap_or_else(|| "未知".to_owned());
        let expire_time = node
            .select(&expire_selector)
            .next()
            .map(|node| node.text().collect::<String>().trim().to_owned())
            .filter(|value| !value.is_empty());

        let exam_id = query
            .get("taskrefId")
            .ok_or_else(|| CpassError::UnexpectedResponse("missing taskrefId".to_owned()))?
            .parse()
            .map_err(|_| CpassError::UnexpectedResponse("invalid taskrefId".to_owned()))?;
        let enc_task = query
            .get("enc_task")
            .ok_or_else(|| CpassError::UnexpectedResponse("missing enc_task".to_owned()))?
            .to_string();

        exams.push(CourseExam {
            exam_id,
            course_id: course.course_id,
            class_id: course.class_id,
            cpi: course.cpi,
            enc_task,
            name,
            status,
            expire_time,
            meta: None,
        });
    }

    Ok(exams)
}

pub fn parse_exam_meta(html: &str, final_url: &str) -> Result<ExamMeta> {
    let final_url = Url::parse(final_url)?;
    if final_url.path().ends_with("/exam-ans/exam/phone/look") {
        return Ok(ExamMeta {
            entry_state: "completed".to_owned(),
            title: None,
            exam_answer_id: None,
            monitor_enc: None,
            need_code: false,
            need_face: false,
            need_captcha: false,
            captcha_id: None,
            blocked_message: None,
        });
    }

    let document = Html::parse_document(html);
    let title_selector = Selector::parse("span.overHidden2")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let exam_answer_selector = Selector::parse("input#testUserRelationId")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let monitor_selector = Selector::parse("input#monitorEnc")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let face_selector = Selector::parse("input#faceRecognitionCompare")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let captcha_selector = Selector::parse("input#captchaCheck")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let captcha_id_selector = Selector::parse("input#captchaCaptchaId")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let script_selector = Selector::parse("body script")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let error_selector = Selector::parse("h2.color6.fs36.textCenter.marBom60.line64")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;

    if let Some(node) = document.select(&error_selector).next() {
        let message = node.text().collect::<String>().trim().to_owned();
        return Ok(ExamMeta {
            entry_state: "blocked".to_owned(),
            title: None,
            exam_answer_id: None,
            monitor_enc: None,
            need_code: false,
            need_face: false,
            need_captcha: false,
            captcha_id: None,
            blocked_message: Some(message),
        });
    }

    let title = document
        .select(&title_selector)
        .next()
        .map(|node| node.text().collect::<String>().trim().to_owned())
        .filter(|value| !value.is_empty());
    let exam_answer_id = document
        .select(&exam_answer_selector)
        .next()
        .and_then(|node| node.value().attr("value"))
        .and_then(|value| value.parse().ok());
    let monitor_enc = document
        .select(&monitor_selector)
        .next()
        .and_then(|node| node.value().attr("value"))
        .map(str::to_owned);
    let need_face = document
        .select(&face_selector)
        .next()
        .and_then(|node| node.value().attr("value"))
        .map(|value| value != "0")
        .unwrap_or(false);
    let need_captcha = document
        .select(&captcha_selector)
        .next()
        .and_then(|node| node.value().attr("value"))
        .map(|value| value != "0")
        .unwrap_or(false);
    let captcha_id = document
        .select(&captcha_id_selector)
        .next()
        .and_then(|node| node.value().attr("value"))
        .map(str::to_owned)
        .filter(|value| !value.is_empty());
    let need_code = document
        .select(&script_selector)
        .flat_map(|node| node.text())
        .any(|code| code.contains("var needcode = 1;") || code.contains("var needcode=1;"));

    Ok(ExamMeta {
        entry_state: "ready".to_owned(),
        title,
        exam_answer_id,
        monitor_enc,
        need_code,
        need_face,
        need_captcha,
        captcha_id,
        blocked_message: None,
    })
}

pub fn parse_exam_preview_questions(html: &str) -> Result<Vec<ExamQuestionSummary>> {
    let document = Html::parse_document(html);
    let question_selector = Selector::parse("div.questionWrap.singleQuesId.ans-cc-exam")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let question_id_selector = Selector::parse("input[name='questionId']")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let title_selector = Selector::parse("div.tit")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let title_header_selector = Selector::parse("h3")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let option_selector = Selector::parse("div.answerList.radioList")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let option_value_selector = Selector::parse("cc")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let image_selector = Selector::parse("img")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let blank_selector = Selector::parse("div.completionList.objectAuswerList span.grayTit")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;

    let mut questions = Vec::new();
    let mut question_ids = BTreeSet::new();
    for (question_index, question_node) in document.select(&question_selector).enumerate() {
        let question_id = parse_question_id(
            question_node
                .select(&question_id_selector)
                .next()
                .and_then(|node| node.value().attr("value")),
        )?;
        if !question_ids.insert(question_id) {
            return Err(CpassError::UnexpectedResponse(format!(
                "duplicate preview question id: {question_id}"
            )));
        }
        let question_type_selector = Selector::parse(&format!("input[name='type{question_id}']"))
            .map_err(|err| {
            CpassError::UnexpectedResponse(format!("invalid selector: {err}"))
        })?;
        let question_type = question_node
            .select(&question_type_selector)
            .next()
            .and_then(|node| node.value().attr("value"))
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| {
                CpassError::UnexpectedResponse("missing preview question type".to_owned())
            })?;
        let title_node = question_node
            .select(&title_selector)
            .next()
            .ok_or_else(|| {
                CpassError::UnexpectedResponse("missing preview question title".to_owned())
            })?;
        let header_text = title_node
            .select(&title_header_selector)
            .next()
            .map(normalized_node_text);
        let question_type_label = header_text
            .as_deref()
            .map(strip_question_type_suffix)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default_question_type_label(question_type))
            .to_owned();
        let prompt = parse_question_prompt(title_node, true)?;
        let options = question_node
            .select(&option_selector)
            .map(|option_node| {
                let key = option_node
                    .value()
                    .attr("name")
                    .unwrap_or("unknown")
                    .to_owned();
                let value = option_node
                    .select(&option_value_selector)
                    .next()
                    .map(question_node_text)
                    .unwrap_or_else(|| question_node_text(option_node));
                let value = strip_option_key_prefix(&value, &key);
                let rich_content = option_node
                    .select(&option_value_selector)
                    .next()
                    .and_then(|node| parse_option_rich_content(node, &image_selector));
                ExamQuestionOption {
                    key,
                    value,
                    rich_content,
                }
            })
            .collect::<Vec<_>>();
        validate_question_options(&options)?;
        let blanks = question_node
            .select(&blank_selector)
            .map(normalized_node_text)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();

        questions.push(ExamQuestionSummary {
            question_index,
            question_id,
            question_type,
            question_type_label,
            question_kind: NormalizedQuestionKind::from_question_type(question_type)
                .as_str()
                .to_owned(),
            prompt,
            options,
            blanks,
        });
    }

    Ok(questions)
}

pub fn parse_task_points(value: &Value) -> Result<Vec<TaskPointSummary>> {
    let cards = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("card"))
        .and_then(|card| card.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CpassError::UnexpectedResponse("missing chapter cards payload".to_owned())
        })?;

    let iframe_selector = Selector::parse("iframe")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;

    let mut task_points = Vec::new();
    for (card_index, card) in cards.iter().enumerate() {
        let title = card
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("未命名任务点")
            .trim()
            .to_owned();

        let Some(description) = card.get("description").and_then(Value::as_str) else {
            continue;
        };
        let fragment = Html::parse_fragment(description);
        for (point_index, point) in fragment.select(&iframe_selector).enumerate() {
            let Some(module) = point.value().attr("module") else {
                continue;
            };
            let iframe_data = point
                .value()
                .attr("data")
                .map(str::trim)
                .filter(|data| !data.is_empty())
                .map(str::to_owned);
            let payload: Value = iframe_data
                .as_deref()
                .map(|data| serde_json::from_str(data).unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
            let chapter_work_metadata = if module == "work" {
                payload
                    .get("workid")
                    .and_then(Value::as_str)
                    .map(|work_id| ChapterWorkMetadata {
                        work_id: work_id.to_owned(),
                        job_id: payload
                            .get("_jobid")
                            .or_else(|| payload.get("jobid"))
                            .and_then(Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                            .map(str::to_owned),
                        school_id: payload
                            .get("schoolid")
                            .and_then(Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                            .map(str::to_owned),
                    })
            } else {
                None
            };
            let resource_id = payload
                .get("objectid")
                .or_else(|| payload.get("workid"))
                .or_else(|| payload.get("liveId"))
                .or_else(|| payload.get("liveid"))
                .or_else(|| payload.get("vdoid"))
                .or_else(|| payload.get("vdoId"))
                .or_else(|| payload.get("streamName"))
                .or_else(|| payload.get("_jobid"))
                .or_else(|| payload.get("jobid"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            task_points.push(TaskPointSummary {
                card_index,
                point_index,
                module: module.to_owned(),
                title: title.clone(),
                resource_id,
                iframe_data,
                attachment_metadata: None,
                chapter_work_metadata,
            });
        }
    }

    Ok(task_points)
}

#[derive(Debug, Default, Deserialize)]
struct RawTaskCardAttachmentBundle {
    #[serde(default)]
    defaults: RawTaskCardAttachmentDefaults,
    #[serde(default)]
    attachments: Vec<RawTaskCardAttachment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTaskCardAttachmentDefaults {
    #[serde(default)]
    fid: Option<u64>,
    #[serde(default)]
    ktoken: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTaskCardAttachment {
    #[serde(default, rename = "type")]
    attachment_type: Option<String>,
    #[serde(default)]
    job: bool,
    #[serde(default)]
    jobid: Option<String>,
    #[serde(default)]
    jtoken: Option<String>,
    #[serde(default)]
    enc: Option<String>,
    #[serde(default, rename = "otherInfo")]
    other_info: Option<String>,
    #[serde(default, rename = "isPassed")]
    is_passed: Option<bool>,
    #[serde(default)]
    property: RawTaskCardAttachmentProperty,
}

#[derive(Debug, Default, Deserialize)]
struct RawTaskCardAttachmentProperty {
    #[serde(default)]
    objectid: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    filetype: Option<String>,
    #[serde(default)]
    workid: Option<String>,
    #[serde(default, rename = "streamName")]
    stream_name: Option<String>,
    #[serde(default, rename = "vdoid", alias = "vdoId")]
    vdo_id: Option<String>,
    #[serde(default, rename = "liveId", alias = "liveid")]
    live_id: Option<String>,
    #[serde(default)]
    rt: Option<String>,
}

pub fn parse_task_card_attachments(html: &str) -> Result<TaskAttachmentSnapshot> {
    let document = Html::parse_document(html);
    let script_selector = Selector::parse("script")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;

    for script in document.select(&script_selector) {
        let source = script.text().collect::<String>();
        let Some(json) = extract_js_assignment(&source, "window.AttachmentSetting") else {
            continue;
        };
        let bundle: RawTaskCardAttachmentBundle = serde_json::from_str(json).map_err(|err| {
            CpassError::UnexpectedResponse(format!(
                "invalid chapter card attachment payload: {err}"
            ))
        })?;
        let attachments = bundle
            .attachments
            .into_iter()
            .map(|attachment| {
                let object_id = attachment.property.objectid;
                let title = attachment.property.name;
                let file_type = attachment.property.filetype;
                TaskAttachment {
                    attachment_type: attachment.attachment_type,
                    object_id,
                    title,
                    file_type,
                    work_id: attachment.property.workid,
                    stream_name: attachment.property.stream_name,
                    vdo_id: attachment.property.vdo_id,
                    live_id: attachment.property.live_id,
                    job_id: attachment.jobid,
                    jtoken: attachment.jtoken,
                    enc: attachment.enc,
                    other_info: attachment.other_info,
                    playback_rate: attachment.property.rt,
                    is_passed: attachment.is_passed,
                    is_job: attachment.job,
                }
            })
            .collect();
        return Ok(TaskAttachmentSnapshot {
            fid: bundle.defaults.fid,
            ktoken: bundle.defaults.ktoken,
            attachments,
        });
    }

    Err(CpassError::UnexpectedResponse(
        "missing chapter card attachment payload".to_owned(),
    ))
}

pub fn parse_chapter_work_attachment_metadata(
    snapshot: &TaskAttachmentSnapshot,
    work_id: &str,
    job_id: &str,
) -> Result<ChapterWorkAttachmentMetadata> {
    let ktoken = snapshot
        .ktoken
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            CpassError::UnexpectedResponse("missing chapter work attachment ktoken".to_owned())
        })?
        .to_owned();

    let attachment = snapshot
        .attachments
        .iter()
        .find(|attachment| attachment.work_id.as_deref() == Some(work_id))
        .ok_or_else(|| {
            CpassError::UnexpectedResponse(format!(
                "missing chapter work attachment for work_id {work_id}"
            ))
        })?;

    if let Some(attachment_job_id) = attachment.job_id.as_deref()
        && attachment_job_id != job_id
    {
        return Err(CpassError::UnexpectedResponse(format!(
            "chapter work attachment job_id mismatch: expected {job_id}, got {attachment_job_id}"
        )));
    }

    let enc = attachment
        .enc
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            CpassError::UnexpectedResponse("missing chapter work attachment enc".to_owned())
        })?
        .to_owned();

    Ok(ChapterWorkAttachmentMetadata {
        work_id: work_id.to_owned(),
        job_id: job_id.to_owned(),
        ktoken,
        enc,
    })
}

pub fn parse_chapter_work_form(html: &str) -> Result<ChapterWorkFormSnapshot> {
    let document = Html::parse_document(html);
    let blank_tips_selector = Selector::parse("p.blankTips")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    if let Some(message) = document
        .select(&blank_tips_selector)
        .map(normalized_node_text)
        .find(|message| !message.is_empty())
    {
        return Err(CpassError::UnexpectedResponse(format!(
            "chapter work page rejected: {message}"
        )));
    }

    let title_selector = Selector::parse("h3.py-Title, h3.chapter-title")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let form_selector = Selector::parse("form#form1")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let question_selector = Selector::parse("div.Py-mian1")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;

    let title = document
        .select(&title_selector)
        .next()
        .map(normalized_node_text)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CpassError::UnexpectedResponse("missing chapter work title".to_owned()))?;
    let form = document
        .select(&form_selector)
        .next()
        .ok_or_else(|| CpassError::UnexpectedResponse("missing chapter work form".to_owned()))?;

    let work_answer_id = parse_form_u64_value(form, "workAnswerId")?;
    let total_question_num = parse_form_usize_value(form, "totalQuestionNum")?;
    let work_relation_id = parse_form_u64_value(form, "workRelationId")?;
    let full_score = parse_form_string_value(form, "fullScore")?;
    let enc_work = parse_form_string_value(form, "enc_work")?;
    let questions = document
        .select(&question_selector)
        .enumerate()
        .map(|(question_index, question_node)| {
            parse_chapter_work_question(question_index, question_node)
        })
        .collect::<Result<Vec<_>>>()?;

    if questions.len() != total_question_num {
        return Err(CpassError::UnexpectedResponse(format!(
            "chapter work question count mismatch: declared {total_question_num}, parsed {}",
            questions.len()
        )));
    }
    let mut question_ids = BTreeSet::new();
    for question in &questions {
        if !question_ids.insert(question.question_id) {
            return Err(CpassError::UnexpectedResponse(format!(
                "duplicate chapter work question id: {}",
                question.question_id
            )));
        }
    }

    Ok(ChapterWorkFormSnapshot {
        title,
        work_answer_id,
        total_question_num,
        work_relation_id,
        full_score,
        enc_work,
        questions,
    })
}

fn parse_chapter_work_question(
    question_index: usize,
    question_node: scraper::ElementRef<'_>,
) -> Result<ChapterWorkQuestionSummary> {
    let answer_type_selector = Selector::parse("input[id^='answertype']")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let title_selector = Selector::parse("div.Py-m1-title")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let option_selector = Selector::parse("li.more-choose-item")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let option_key_selector = Selector::parse("em.choose-opt")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let option_value_selector = Selector::parse("div.choose-desc")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let image_selector = Selector::parse("img")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let blank_selector = Selector::parse("ul.blankList2 > li")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let blank_label_selector = Selector::parse("span")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;

    let answer_type = question_node
        .select(&answer_type_selector)
        .next()
        .ok_or_else(|| {
            CpassError::UnexpectedResponse("missing chapter work question type".to_owned())
        })?;
    let question_id = parse_question_id(
        answer_type
            .value()
            .attr("id")
            .and_then(|value| value.strip_prefix("answertype")),
    )?;
    let question_type = answer_type
        .value()
        .attr("value")
        .ok_or_else(|| {
            CpassError::UnexpectedResponse("missing chapter work question type value".to_owned())
        })?
        .parse::<u64>()
        .map_err(|err| {
            CpassError::UnexpectedResponse(format!("invalid chapter work question type: {err}"))
        })?;
    let title_node = question_node
        .select(&title_selector)
        .next()
        .ok_or_else(|| {
            CpassError::UnexpectedResponse("missing chapter work question title".to_owned())
        })?;
    let prompt = parse_question_prompt(title_node, false)?;
    let question_type_label = default_question_type_label(question_type).to_owned();

    let options = if matches!(question_type, 0 | 1) {
        question_node
            .select(&option_selector)
            .map(|option_node| {
                let key_node = option_node.select(&option_key_selector).next();
                let key = key_node
                    .and_then(|node| node.value().attr("id-param").map(str::to_owned))
                    .unwrap_or_else(|| {
                        key_node
                            .map(normalized_node_text)
                            .filter(|value| !value.is_empty())
                            .unwrap_or_else(|| "unknown".to_owned())
                    });
                let value = option_node
                    .select(&option_value_selector)
                    .next()
                    .map(question_node_text)
                    .unwrap_or_else(|| question_node_text(option_node));
                let value = strip_option_key_prefix(&value, &key);
                let rich_content = option_node
                    .select(&option_value_selector)
                    .next()
                    .and_then(|node| parse_option_rich_content(node, &image_selector));
                ExamQuestionOption {
                    key,
                    value,
                    rich_content,
                }
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    validate_question_options(&options)?;
    let blanks = if question_type == 2 {
        question_node
            .select(&blank_selector)
            .map(|blank_node| {
                blank_node
                    .select(&blank_label_selector)
                    .next()
                    .map(normalized_node_text)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| normalized_node_text(blank_node))
            })
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    Ok(ChapterWorkQuestionSummary {
        question_index,
        question_id,
        question_type,
        question_type_label,
        question_kind: NormalizedQuestionKind::from_question_type(question_type)
            .as_str()
            .to_owned(),
        prompt,
        options,
        blanks,
    })
}

pub fn parse_video_attachment_status(value: &Value) -> Result<VideoAttachmentStatus> {
    Ok(VideoAttachmentStatus {
        status: value
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| CpassError::UnexpectedResponse("missing video status".to_owned()))?
            .to_owned(),
        filename: value
            .get("filename")
            .or_else(|| value.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| CpassError::UnexpectedResponse("missing video filename".to_owned()))?
            .to_owned(),
        duration_secs: value
            .get("duration")
            .and_then(Value::as_u64)
            .ok_or_else(|| CpassError::UnexpectedResponse("missing video duration".to_owned()))?,
        dtoken: value
            .get("dtoken")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

pub fn parse_video_play_report_ack(value: &Value) -> Result<VideoPlayReportAck> {
    if let Some(error) = value
        .get("error")
        .and_then(Value::as_str)
        .filter(|error| !error.trim().is_empty())
    {
        return Err(CpassError::UnexpectedResponse(format!(
            "video play-report rejected: {error}"
        )));
    }

    Ok(VideoPlayReportAck {
        is_passed: value.get("isPassed").and_then(Value::as_bool),
        playing_time_secs: value.get("playingTime").and_then(parse_value_u64),
        duration_secs: value.get("duration").and_then(parse_value_u64),
        clip_time: value
            .get("clipTime")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

pub fn parse_document_reading_report_ack(value: &Value) -> Result<DocumentReadingReportAck> {
    if let Some(error) = value
        .get("error")
        .and_then(Value::as_str)
        .filter(|error| !error.trim().is_empty())
    {
        return Err(CpassError::UnexpectedResponse(format!(
            "document reading-report rejected: {error}"
        )));
    }

    Ok(DocumentReadingReportAck {
        success: value.get("status").and_then(Value::as_bool).unwrap_or(true),
    })
}

pub fn parse_live_progress_report_ack(body: &str) -> Result<LiveProgressReportAck> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(CpassError::UnexpectedResponse(
            "live progress-report returned an empty body".to_owned(),
        ));
    }

    if trimmed.eq_ignore_ascii_case("success") {
        return Ok(LiveProgressReportAck { success: true });
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let success = value
            .get("status")
            .and_then(Value::as_bool)
            .or_else(|| value.get("success").and_then(Value::as_bool))
            .or_else(|| {
                value
                    .as_str()
                    .map(|status| status.eq_ignore_ascii_case("success"))
            })
            .unwrap_or(false);

        if success {
            return Ok(LiveProgressReportAck { success: true });
        }

        let message = value
            .get("msg")
            .or_else(|| value.get("message"))
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
            .unwrap_or(trimmed);
        return Err(CpassError::UnexpectedResponse(format!(
            "live progress-report rejected: {message}"
        )));
    }

    if trimmed.to_ascii_lowercase().contains("success") {
        return Ok(LiveProgressReportAck { success: true });
    }

    Err(CpassError::UnexpectedResponse(format!(
        "live progress-report rejected: {trimmed}"
    )))
}

fn parse_form_string_value(form: scraper::ElementRef<'_>, id: &str) -> Result<String> {
    let selector = Selector::parse(&format!("input#{id}"))
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    form.select(&selector)
        .next()
        .and_then(|node| node.value().attr("value"))
        .map(str::to_owned)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CpassError::UnexpectedResponse(format!("missing chapter work {id}")))
}

fn parse_form_u64_value(form: scraper::ElementRef<'_>, id: &str) -> Result<u64> {
    let value = parse_form_string_value(form, id)?;
    value
        .parse::<u64>()
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid chapter work {id}: {err}")))
}

fn parse_form_usize_value(form: scraper::ElementRef<'_>, id: &str) -> Result<usize> {
    let value = parse_form_string_value(form, id)?;
    value
        .parse::<usize>()
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid chapter work {id}: {err}")))
}

fn parse_value_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
}

fn normalized_node_text(node: scraper::ElementRef<'_>) -> String {
    normalize_text(&node.text().collect::<Vec<_>>().join(""))
}

fn extract_js_assignment<'a>(script: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name} =");
    let start = script.find(&marker)?;
    let tail = script[start + marker.len()..].trim_start();
    let end = tail.find(';').unwrap_or(tail.len());
    Some(tail[..end].trim())
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_question_type_suffix(value: &str) -> &str {
    value.split('（').next().unwrap_or(value).trim()
}

fn parse_question_id(raw: Option<&str>) -> Result<u64> {
    raw.filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| CpassError::UnexpectedResponse("invalid or missing question id".to_owned()))
}

fn validate_question_options(options: &[ExamQuestionOption]) -> Result<()> {
    let mut keys = BTreeSet::new();
    for option in options {
        if option.key.len() != 1
            || !option.key.bytes().all(|byte| byte.is_ascii_uppercase())
            || !keys.insert(&option.key)
        {
            return Err(CpassError::UnexpectedResponse(format!(
                "invalid or duplicate question option key: {}",
                option.key
            )));
        }
    }
    Ok(())
}

fn render_question_text(
    node: scraper::ElementRef<'_>,
    skip_heading: bool,
    skip_label: Option<scraper::ElementRef<'_>>,
    output: &mut String,
) {
    if Some(node) == skip_label {
        output.push(' ');
        return;
    }
    let name = node.value().name();
    // Formula structure distinguishes answers that would otherwise collapse to identical text.
    if name == "math" {
        output.push_str(&node.html());
        return;
    }
    if name == "script"
        && node.value().attr("type").is_some_and(|kind| {
            kind.split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("math/tex"))
        })
    {
        output.push_str(r"\(");
        output.extend(node.text());
        output.push_str(r"\)");
        return;
    }
    if matches!(name, "script" | "style" | "input" | "button") || (skip_heading && name == "h3") {
        return;
    }
    let block = matches!(name, "p" | "div" | "li" | "br");
    if block {
        output.push('\n');
    }
    match name {
        "sup" => output.push_str("^{"),
        "sub" => output.push_str("_{"),
        _ => {}
    }
    for child in node.children() {
        if let Some(element) = scraper::ElementRef::wrap(child) {
            render_question_text(element, skip_heading, skip_label, output);
        } else if let Some(text) = child.value().as_text() {
            // Source indentation is whitespace, not a paragraph boundary.
            for ch in text.chars() {
                output.push(if ch.is_whitespace() { ' ' } else { ch });
            }
        }
    }
    if matches!(name, "sup" | "sub") {
        output.push('}');
    }
    if block {
        output.push('\n');
    }
}

fn question_node_text(node: scraper::ElementRef<'_>) -> String {
    let mut output = String::new();
    render_question_text(node, false, None, &mut output);
    normalize_question_lines(&output)
}

fn normalize_question_lines(value: &str) -> String {
    value
        .lines()
        .map(normalize_text)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_question_prompt(title_node: scraper::ElementRef<'_>, preview: bool) -> Result<String> {
    let labels = Selector::parse("span, strong, b")
        .map_err(|err| CpassError::UnexpectedResponse(format!("invalid selector: {err}")))?;
    let full_text = title_node.text().collect::<String>();
    let skip_label = (!preview)
        .then(|| {
            title_node.select(&labels).find(|node| {
                let raw = node.text().collect::<String>();
                let label = normalize_text(&raw);
                let label = label.trim_matches(['（', '）', '(', ')']);
                let label = label.split(['，', ',']).next().unwrap_or(label).trim();
                (0..=21).any(|kind| default_question_type_label(kind) == label)
                    && full_text.split_once(&raw).is_some_and(|(prefix, _)| {
                        strip_question_number_prefix(&normalize_text(prefix)).is_empty()
                    })
            })
        })
        .flatten();
    let mut output = String::new();
    render_question_text(title_node, preview, skip_label, &mut output);
    let prompt = strip_question_number_prefix(&normalize_question_lines(&output));
    if prompt.is_empty() {
        return Err(CpassError::UnexpectedResponse(
            "missing question prompt".to_owned(),
        ));
    }
    Ok(prompt)
}

fn strip_question_number_prefix(value: &str) -> String {
    let trimmed = value.trim();
    let mut seen_digit = false;
    let mut split_at = None;

    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() {
            seen_digit = true;
            continue;
        }
        if seen_digit && matches!(ch, '.' | '．' | '、') {
            if ch == '.'
                && trimmed[index + ch.len_utf8()..].starts_with(|next: char| next.is_ascii_digit())
            {
                break;
            }
            split_at = Some(index + ch.len_utf8());
            break;
        }
        if ch.is_whitespace() {
            continue;
        }
        break;
    }

    split_at
        .map(|index| trimmed[index..].trim_start().to_owned())
        .unwrap_or_else(|| trimmed.to_owned())
}

fn default_question_type_label(question_type: u64) -> &'static str {
    match question_type {
        0 => "单选题",
        1 => "多选题",
        2 => "填空题",
        3 => "判断题",
        4 => "简答题",
        5 => "名词解释",
        6 => "论述题",
        7 => "计算题",
        8 => "其它",
        9 => "分录题",
        10 => "资料题",
        11 => "连线题",
        13 => "排序题",
        14 => "完型填空",
        15 => "阅读理解",
        18 => "口语题",
        19 => "听力题",
        20 => "共用选项题",
        21 => "测评题",
        _ => "未知题型",
    }
}

fn strip_option_key_prefix(value: &str, key: &str) -> String {
    [format!("{key}."), format!("{key}．"), format!("{key}、")]
        .into_iter()
        .find_map(|prefix| value.strip_prefix(&prefix))
        .map(|value| value.trim_start().to_owned())
        .unwrap_or_else(|| value.to_owned())
}

fn parse_option_rich_content(
    node: scraper::ElementRef<'_>,
    image_selector: &Selector,
) -> Option<QuestionRichContent> {
    let source_html = node.inner_html().trim().to_owned();
    let has_nested_elements = node
        .children()
        .any(|child| scraper::ElementRef::wrap(child).is_some());
    let image_urls = node
        .select(image_selector)
        .filter_map(|image| image.value().attr("src"))
        .map(str::to_owned)
        .collect::<Vec<_>>();

    if !has_nested_elements && image_urls.is_empty() {
        return None;
    }

    Some(QuestionRichContent {
        source_html,
        image_urls,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::{
        ChaoxingClient, QrLoginStatus, build_chapter_work_form_transport_request,
        build_document_reading_report_transport_request,
        build_live_progress_report_transport_request, build_video_play_report_transport_request,
        merge_chapter_progress, parse_chapter_work_attachment_metadata, parse_chapter_work_form,
        parse_chapters, parse_courses, parse_document_reading_report_ack, parse_exam_meta,
        parse_exam_preview_questions, parse_exams, parse_live_progress_report_ack,
        parse_qr_login_bootstrap, parse_qr_login_status, parse_task_card_attachments,
        parse_task_points, parse_video_attachment_status, parse_video_play_report_ack,
    };
    use crate::models::{
        ChapterWorkAttachmentMetadata, ChapterWorkRuntimeRequest, Course, CourseExam,
        DocumentReadingReportRequest, ExamPreviewQuery, LiveProgressReportRequest,
        VideoPlayReportRequest,
    };
    use crate::transport::FixtureChaoxingTransport;

    #[test]
    fn parses_course_fixture() {
        let fixture = include_str!("../../../../fixtures/legacy/course_list.json");
        let value: serde_json::Value = serde_json::from_str(fixture).expect("valid course fixture");
        let courses = parse_courses(&value).expect("courses to parse");
        assert_eq!(courses.len(), 2);
        assert_eq!(courses[0].name, "现代汉语");
    }

    #[test]
    fn parses_chapter_fixture_and_status() {
        let fixture = include_str!("../../../../fixtures/legacy/chapter_list.json");
        let value: serde_json::Value =
            serde_json::from_str(fixture).expect("valid chapter fixture");
        let mut chapters = parse_chapters(&value).expect("chapters to parse");
        let progress_fixture = include_str!("../../../../fixtures/legacy/chapter_status.json");
        let progress: serde_json::Value =
            serde_json::from_str(progress_fixture).expect("valid progress fixture");
        merge_chapter_progress(&mut chapters, &progress);
        assert_eq!(chapters[0].point_total, 3);
        assert_eq!(chapters[0].point_finished, 1);
    }

    #[test]
    fn parses_exam_fixture() {
        let html = include_str!("../../../../fixtures/legacy/exam_list.html");
        let course = Course {
            course_id: 1001,
            class_id: 2002,
            cpi: 3003,
            key: 4004,
            name: "现代汉语".to_owned(),
            teacher_name: "王老师".to_owned(),
            state: "进行中".to_owned(),
        };
        let exams = parse_exams(html, &course).expect("exam list to parse");
        assert_eq!(exams.len(), 6);
        assert_eq!(exams[0].exam_id, 555001);
        assert_eq!(exams[2].status, "未开始");
    }

    #[test]
    fn parses_account_fixture() {
        let fixture = include_str!("../../../../fixtures/legacy/account_info.json");
        let value: serde_json::Value =
            serde_json::from_str(fixture).expect("valid account fixture");
        assert_eq!(value["msg"]["phone"], "13800138000");
    }

    #[test]
    fn parses_qr_login_bootstrap_fixture() {
        let fixture = r#"<!DOCTYPE html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8" />
    <title>登录</title>
  </head>
  <body>
    <form>
      <input id="uuid" value="qr-uuid-001" />
      <input id="enc" value="qr-enc-001" />
    </form>
  </body>
</html>
"#;
        let bootstrap = parse_qr_login_bootstrap(fixture).expect("qr bootstrap");
        assert_eq!(bootstrap.uuid, "qr-uuid-001");
        assert_eq!(bootstrap.enc, "qr-enc-001");
    }

    #[test]
    fn parses_qr_login_status_fixture() {
        let fixture = r#"{"status": true}"#;
        let value: serde_json::Value =
            serde_json::from_str(fixture).expect("valid qr auth status fixture");
        assert_eq!(
            parse_qr_login_status(&value).expect("qr status"),
            QrLoginStatus::Success
        );
    }

    #[test]
    fn parses_task_point_fixture() {
        let fixture = include_str!("../../../../fixtures/legacy/chapter_cards_11.json");
        let value: serde_json::Value = serde_json::from_str(fixture).expect("valid task fixture");
        let task_points = parse_task_points(&value).expect("task points to parse");
        assert_eq!(task_points.len(), 2);
        assert_eq!(task_points[0].module, "insertvideo");
        assert_eq!(
            task_points[0].iframe_data.as_deref(),
            Some("{\"objectid\":\"video-001\"}")
        );
        assert_eq!(task_points[1].module, "work");
        assert_eq!(
            task_points[1].iframe_data.as_deref(),
            Some("{\"workid\":\"work-001\",\"_jobid\":\"job-001\"}")
        );
    }

    #[test]
    fn parses_live_task_point_fixture() {
        let fixture = include_str!("../../../../fixtures/legacy_live/chapter_cards_13.json");
        let value: serde_json::Value = serde_json::from_str(fixture).expect("valid task fixture");
        let task_points = parse_task_points(&value).expect("live task point to parse");

        assert_eq!(task_points.len(), 1);
        assert_eq!(task_points[0].module, "insertlive");
        assert_eq!(task_points[0].resource_id, "live-course-001");
        assert_eq!(
            task_points[0].iframe_data.as_deref(),
            Some(
                "{\"liveId\":\"live-course-001\",\"vdoid\":\"vdo-live-001\",\"streamName\":\"zhibo_12345\"}"
            )
        );
    }

    #[test]
    fn parses_task_card_attachment_fixtures() {
        let video_fixture =
            include_str!("../../../../fixtures/legacy/chapter_card_attachment_11_0.html");
        let video = parse_task_card_attachments(video_fixture).expect("video attachment fixture");
        assert_eq!(video.fid, Some(9876));
        assert_eq!(video.attachments.len(), 1);
        assert_eq!(
            video.attachments[0].attachment_type.as_deref(),
            Some("video")
        );
        assert_eq!(video.attachments[0].object_id.as_deref(), Some("video-001"));
        assert_eq!(video.attachments[0].title.as_deref(), Some("绪论视频"));
        assert_eq!(
            video.attachments[0].job_id.as_deref(),
            Some("job-video-001")
        );
        assert_eq!(
            video.attachments[0].other_info.as_deref(),
            Some("nodeId_11")
        );
        assert_eq!(video.attachments[0].playback_rate.as_deref(), Some("0.9"));
        assert!(video.attachments[0].is_job);

        let work_fixture =
            include_str!("../../../../fixtures/legacy/chapter_card_attachment_11_1.html");
        let work = parse_task_card_attachments(work_fixture).expect("work attachment fixture");
        assert_eq!(work.ktoken.as_deref(), Some("ktoken-work-001"));
        assert_eq!(work.attachments.len(), 1);
        assert_eq!(work.attachments[0].attachment_type.as_deref(), Some("work"));
        assert_eq!(work.attachments[0].work_id.as_deref(), Some("work-001"));
        assert_eq!(work.attachments[0].job_id.as_deref(), Some("job-001"));
        assert_eq!(
            work.attachments[0].enc.as_deref(),
            Some("enc-work-fetch-001")
        );
        assert!(work.attachments[0].is_job);

        let work_attachment = parse_chapter_work_attachment_metadata(&work, "work-001", "job-001")
            .expect("chapter work attachment metadata");
        assert_eq!(
            work_attachment,
            ChapterWorkAttachmentMetadata {
                work_id: "work-001".to_owned(),
                job_id: "job-001".to_owned(),
                ktoken: "ktoken-work-001".to_owned(),
                enc: "enc-work-fetch-001".to_owned(),
            }
        );

        let document_fixture =
            include_str!("../../../../fixtures/legacy/chapter_card_attachment_12_0.html");
        let document =
            parse_task_card_attachments(document_fixture).expect("document attachment fixture");
        assert_eq!(document.fid, None);
        assert_eq!(document.attachments.len(), 1);
        assert_eq!(
            document.attachments[0].attachment_type.as_deref(),
            Some("document")
        );
        assert_eq!(
            document.attachments[0].object_id.as_deref(),
            Some("doc-001")
        );
        assert_eq!(
            document.attachments[0].title.as_deref(),
            Some("现代汉语词汇材料")
        );
        assert_eq!(document.attachments[0].file_type.as_deref(), Some("pdf"));
        assert_eq!(
            document.attachments[0].job_id.as_deref(),
            Some("job-doc-001")
        );
        assert_eq!(
            document.attachments[0].jtoken.as_deref(),
            Some("jtoken-doc-001")
        );
        assert!(document.attachments[0].is_job);

        let live_fixture =
            include_str!("../../../../fixtures/legacy/chapter_card_attachment_13_0.html");
        let live = parse_task_card_attachments(live_fixture).expect("live attachment fixture");
        assert_eq!(live.fid, None);
        assert_eq!(live.attachments.len(), 1);
        assert_eq!(live.attachments[0].attachment_type.as_deref(), Some("live"));
        assert_eq!(live.attachments[0].title.as_deref(), Some("直播任务点"));
        assert_eq!(
            live.attachments[0].stream_name.as_deref(),
            Some("zhibo_12345")
        );
        assert_eq!(live.attachments[0].vdo_id.as_deref(), Some("vdo-live-001"));
        assert_eq!(
            live.attachments[0].live_id.as_deref(),
            Some("live-course-001")
        );
        assert_eq!(live.attachments[0].job_id.as_deref(), Some("job-live-001"));
        assert!(live.attachments[0].is_job);
    }

    #[test]
    fn parses_video_status_fixture() {
        let fixture = include_str!("../../../../fixtures/legacy/video_status_video_001.json");
        let value: serde_json::Value =
            serde_json::from_str(fixture).expect("valid video status fixture");
        let status = parse_video_attachment_status(&value).expect("video status");
        assert_eq!(status.status, "success");
        assert_eq!(status.filename, "绪论视频.mp4");
        assert_eq!(status.duration_secs, 602);
        assert_eq!(status.dtoken.as_deref(), Some("video-dtoken-001"));
    }

    #[test]
    fn builds_video_play_report_transport_request() {
        let request = build_video_play_report_transport_request(&VideoPlayReportRequest {
            cpi: 3003,
            class_id: 2002,
            user_id: 9009,
            dtoken: "video-dtoken-001".to_owned(),
            object_id: "video-001".to_owned(),
            job_id: "job-video-001".to_owned(),
            other_info: "nodeId=11&courseId=1001".to_owned(),
            playing_time_secs: 602,
            duration_secs: 602,
            clip_time: "0_602".to_owned(),
            enc: "enc-001".to_owned(),
            playback_rate: "0.9".to_owned(),
            report_timestamp_millis: 1_717_000_000_123,
        });

        assert_eq!(
            request.url,
            "https://mooc1-api.chaoxing.com/multimedia/log/a/3003/video-dtoken-001"
        );
        assert!(request.query.is_empty());
        assert_eq!(
            request.raw_query.as_deref(),
            Some(
                "otherInfo=nodeId=11&courseId=1001&playingTime=602&duration=602&jobid=job-video-001&clipTime=0_602&clazzId=2002&objectId=video-001&userid=9009&isdrag=0&enc=enc-001&rt=0.9&dtype=Video&view=pc&_t=1717000000123"
            )
        );
    }

    #[test]
    fn builds_document_reading_report_transport_request() {
        let request =
            build_document_reading_report_transport_request(&DocumentReadingReportRequest {
                course_id: 1001,
                class_id: 2002,
                knowledge_id: 12,
                job_id: "job-doc-001".to_owned(),
                jtoken: "jtoken-doc-001".to_owned(),
                report_timestamp_millis: 1_717_000_000_123,
            });

        assert_eq!(
            request.url,
            "https://mooc1.chaoxing.com/ananas/job/document"
        );
        assert_eq!(
            request.query,
            vec![
                ("jobid".to_owned(), "job-doc-001".to_owned()),
                ("knowledgeid".to_owned(), "12".to_owned()),
                ("courseid".to_owned(), "1001".to_owned()),
                ("clazzid".to_owned(), "2002".to_owned()),
                ("jtoken".to_owned(), "jtoken-doc-001".to_owned()),
                ("_dc".to_owned(), "1717000000123".to_owned()),
            ]
        );
        assert!(request.raw_query.is_none());
    }

    #[test]
    fn builds_live_progress_report_transport_request() {
        let request = build_live_progress_report_transport_request(&LiveProgressReportRequest {
            stream_name: "zhibo_12345".to_owned(),
            vdo_id: "vdo-live-001".to_owned(),
            user_id: 9009,
            is_start: false,
            course_id: 1001,
            report_timestamp_millis: 1_717_000_000_123,
        });

        assert_eq!(request.url, "https://zhibo.chaoxing.com/saveTimePc");
        assert_eq!(
            request.query,
            vec![
                ("streamName".to_owned(), "zhibo_12345".to_owned()),
                ("vdoid".to_owned(), "vdo-live-001".to_owned()),
                ("userId".to_owned(), "9009".to_owned()),
                ("isStart".to_owned(), "0".to_owned()),
                ("t".to_owned(), "1717000000123".to_owned()),
                ("courseId".to_owned(), "1001".to_owned()),
            ]
        );
        assert!(request.raw_query.is_none());
    }

    #[test]
    fn builds_chapter_work_form_transport_request() {
        let request = build_chapter_work_form_transport_request(
            &ChapterWorkRuntimeRequest {
                course_id: 1001,
                class_id: 2002,
                knowledge_id: 11,
                user_id: 9009,
                cpi: 3003,
                card_index: 1,
                work_id: "work-001".to_owned(),
                job_id: "job-001".to_owned(),
                school_id: None,
            },
            &ChapterWorkAttachmentMetadata {
                work_id: "work-001".to_owned(),
                job_id: "job-001".to_owned(),
                ktoken: "ktoken-work-001".to_owned(),
                enc: "enc-work-fetch-001".to_owned(),
            },
        );

        assert_eq!(
            request.url,
            "https://mooc1-api.chaoxing.com/android/mworkspecial"
        );
        assert_eq!(
            request.query,
            vec![
                ("courseid".to_owned(), "1001".to_owned()),
                ("workid".to_owned(), "work-001".to_owned()),
                ("jobid".to_owned(), "job-001".to_owned()),
                ("needRedirect".to_owned(), "true".to_owned()),
                ("knowledgeid".to_owned(), "11".to_owned()),
                ("userid".to_owned(), "9009".to_owned()),
                ("ut".to_owned(), "s".to_owned()),
                ("clazzId".to_owned(), "2002".to_owned()),
                ("cpi".to_owned(), "3003".to_owned()),
                ("ktoken".to_owned(), "ktoken-work-001".to_owned()),
                ("enc".to_owned(), "enc-work-fetch-001".to_owned()),
            ]
        );
        assert!(request.raw_query.is_none());
    }

    #[test]
    fn parses_video_play_report_ack_fixtures() {
        let progress_fixture =
            include_str!("../../../../fixtures/legacy/video_play_report_video_001_58.json");
        let progress_value: serde_json::Value =
            serde_json::from_str(progress_fixture).expect("valid progress fixture");
        let progress = parse_video_play_report_ack(&progress_value).expect("progress ack");
        assert_eq!(progress.is_passed, Some(false));
        assert_eq!(progress.playing_time_secs, Some(58));
        assert_eq!(progress.duration_secs, Some(602));
        assert_eq!(progress.clip_time.as_deref(), Some("0_602"));

        let completion_fixture =
            include_str!("../../../../fixtures/legacy/video_play_report_video_001.json");
        let completion_value: serde_json::Value =
            serde_json::from_str(completion_fixture).expect("valid completion fixture");
        let completion =
            parse_video_play_report_ack(&completion_value).expect("completion play-report ack");
        assert_eq!(completion.is_passed, Some(true));
        assert_eq!(completion.playing_time_secs, Some(602));
        assert_eq!(completion.duration_secs, Some(602));
        assert_eq!(completion.clip_time.as_deref(), Some("0_602"));
    }

    #[test]
    fn parses_document_reading_report_ack_fixtures() {
        let fixture =
            include_str!("../../../../fixtures/legacy/document_reading_report_doc_001.json");
        let value: serde_json::Value =
            serde_json::from_str(fixture).expect("valid document reading-report fixture");
        let ack = parse_document_reading_report_ack(&value).expect("document reading-report ack");

        assert!(ack.success);
    }

    #[test]
    fn parses_live_progress_report_ack_fixtures() {
        let fixture =
            include_str!("../../../../fixtures/legacy_live/live_progress_report_live_001.txt");
        let ack = parse_live_progress_report_ack(fixture).expect("live progress-report ack");

        assert!(ack.success);
    }

    #[test]
    fn rejects_video_play_report_error_fixture() {
        let fixture =
            include_str!("../../../../fixtures/legacy/video_play_report_video_001_error.json");
        let value: serde_json::Value =
            serde_json::from_str(fixture).expect("valid video play-report error fixture");
        let err = parse_video_play_report_ack(&value).expect_err("error fixture to fail");
        assert!(
            err.to_string()
                .contains("video play-report rejected: progress rejected")
        );
    }

    #[test]
    fn rejects_document_reading_report_error_fixture() {
        let fixture =
            include_str!("../../../../fixtures/legacy/document_reading_report_doc_001_error.json");
        let value: serde_json::Value =
            serde_json::from_str(fixture).expect("valid document reading-report error fixture");
        let err = parse_document_reading_report_ack(&value)
            .expect_err("document reading-report error fixture to fail");
        assert!(
            err.to_string()
                .contains("document reading-report rejected: document progress rejected")
        );
    }

    #[test]
    fn rejects_live_progress_report_error_fixture() {
        let fixture = include_str!(
            "../../../../fixtures/legacy_live/live_progress_report_live_001_error.txt"
        );
        let err = parse_live_progress_report_ack(fixture)
            .expect_err("live progress-report error fixture to fail");
        assert!(
            err.to_string()
                .contains("live progress-report rejected: live progress rejected")
        );
    }

    #[test]
    fn parses_exam_cover_meta_fixture() {
        let fixture = include_str!("../../../../fixtures/legacy/exam_cover_555001.html");
        let meta = parse_exam_meta(
            fixture,
            "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/task-exam",
        )
        .expect("exam cover meta");
        assert_eq!(meta.entry_state, "ready");
        assert_eq!(meta.title.as_deref(), Some("第一单元测验"));
        assert_eq!(meta.exam_answer_id, Some(900001));
        assert!(meta.need_face);
        assert!(meta.need_captcha);
    }

    #[test]
    fn parses_completed_exam_cover_from_redirect_url() {
        let meta = parse_exam_meta(
            "<html></html>",
            "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/look",
        )
        .expect("completed exam meta");
        assert_eq!(meta.entry_state, "completed");
        assert_eq!(meta.title, None);
    }

    #[test]
    fn parses_blocked_exam_cover_meta_fixtures() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let cases = [
            ("exam_cover_555003.html", "考试尚未开始"),
            (
                "exam_cover_555004.html",
                "章节任务点未完成，请完成任务点后再参加考试",
            ),
            ("exam_cover_555005.html", "请使用指定的IP环境进行考试。"),
            (
                "exam_cover_555006.html",
                "该试卷只允许在电脑考试客户端考试,完成考试后可在手机端查看",
            ),
        ];

        for (fixture_name, expected_message) in cases {
            let fixture = std::fs::read_to_string(fixture_root.join(fixture_name))
                .expect("blocked exam cover fixture");
            let meta = parse_exam_meta(
                &fixture,
                "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/task-exam",
            )
            .expect("blocked exam meta");
            assert_eq!(meta.entry_state, "blocked");
            assert_eq!(meta.blocked_message.as_deref(), Some(expected_message));
            assert_eq!(meta.title, None);
            assert_eq!(meta.exam_answer_id, None);
            assert_eq!(meta.monitor_enc, None);
            assert!(!meta.need_code);
            assert!(!meta.need_face);
            assert!(!meta.need_captcha);
            assert_eq!(meta.captcha_id, None);
        }
    }

    #[test]
    fn parses_exam_preview_question_inventory_fixture() {
        let fixture = include_str!("../../../../fixtures/legacy/exam_preview_555001.html");
        let questions = parse_exam_preview_questions(fixture).expect("preview questions to parse");
        assert_eq!(questions.len(), 4);

        assert_eq!(questions[0].question_index, 0);
        assert_eq!(questions[0].question_id, 700001);
        assert_eq!(questions[0].question_type, 0);
        assert_eq!(questions[0].question_type_label, "单选题");
        assert_eq!(questions[0].prompt, "普通话以哪种方言为基础方言？");
        assert_eq!(questions[0].options.len(), 4);
        assert_eq!(questions[0].options[1].key, "B");
        assert_eq!(questions[0].options[1].value, "北方方言");
        assert!(questions[0].options[1].rich_content.is_none());

        assert_eq!(questions[2].question_type_label, "填空题");
        assert_eq!(questions[2].blanks, vec!["词义：", "词性："]);

        assert_eq!(questions[3].question_type_label, "判断题");
        assert_eq!(questions[3].prompt, "现代汉语共同语就是普通话。");
        assert!(questions[3].options.is_empty());
    }

    #[test]
    fn preview_types_use_the_exact_question_id_and_preserve_unknown_types() {
        let fixture = include_str!("../../../../fixtures/legacy/exam_preview_555001.html");
        let fixture = fixture.replace(
            "<input name=\"type700001\" value=\"0\" />",
            "<input name=\"typeName700001\" value=\"0\" /><input name=\"type700001\" value=\"999\" />",
        );
        let questions = parse_exam_preview_questions(&fixture).expect("valid preview inventory");
        assert_eq!(questions[0].question_type, 999);
        assert_eq!(questions[0].question_kind, "unknown");
    }

    #[test]
    fn question_inventories_reject_invalid_ids_options_counts_and_empty_prompts() {
        let preview = include_str!("../../../../fixtures/legacy/exam_preview_555001.html");
        for (old, new) in [
            ("700001", "0"),
            ("700001", "+1"),
            ("700002", "700001"),
            ("name=\"B\"", "name=\"A\""),
            ("name=\"B\"", "name=\"?\""),
            ("普通话以哪种方言为基础方言？", ""),
        ] {
            assert!(
                parse_exam_preview_questions(&preview.replace(old, new)).is_err(),
                "{old} -> {new}"
            );
        }
        let work = include_str!("../../../../fixtures/legacy/chapter_work_11_work_001.html");
        for (old, new) in [
            ("700101", "0"),
            ("700101", "+1"),
            ("700102", "700101"),
            ("id-param=\"B\"", "id-param=\"A\""),
            ("id-param=\"B\"", "id-param=\"?\""),
            (
                "id=\"totalQuestionNum\" value=\"3\"",
                "id=\"totalQuestionNum\" value=\"2\"",
            ),
            ("普通话以哪种方言为基础方言？", ""),
        ] {
            assert!(
                parse_chapter_work_form(&work.replace(old, new)).is_err(),
                "{old} -> {new}"
            );
        }
    }

    #[test]
    fn preview_and_work_preserve_inline_text_and_formula_structure() {
        let preview = include_str!("../../../../fixtures/legacy/exam_preview_555001.html");
        let work = include_str!("../../../../fixtures/legacy/chapter_work_11_work_001.html");
        for (html, expected) in [
            (
                "题<span>目</span><p>后<strong>段</strong></p>",
                "题目\n后段",
            ),
            ("x<sup>2</sup>+x<sub>i<sup>2</sup></sub>", "x^{2}+x_{i^{2}}"),
            (
                "<math><mfrac><mi>a</mi><mi>b</mi></mfrac></math>",
                "<math><mfrac><mi>a</mi><mi>b</mi></mfrac></math>",
            ),
            (
                r#"<script type="math/tex; mode=display">\frac{a}{b}</script>"#,
                r"\(\frac{a}{b}\)",
            ),
            (
                "题干<script>untrusted()</script><script type='math/tex-not'>hidden</script>",
                "题干",
            ),
            ("3.14 + x", "3.14 + x"),
            ("<span>New</span>\n<span>York</span>", "New York"),
        ] {
            let preview = preview
                .replace("普通话以哪种方言为基础方言？", &format!(" {html}"))
                .replace("北方方言", html);
            let work = work
                .replace("<p>普通话以哪种方言为基础方言？</p>", html)
                .replace("北方方言", html);
            let questions =
                parse_exam_preview_questions(&preview).expect("valid preview inventory");
            let snapshot = parse_chapter_work_form(&work).expect("valid chapter work inventory");
            assert_eq!(questions[0].prompt, expected);
            assert_eq!(snapshot.questions[0].prompt, expected);
            assert_eq!(questions[0].options[1].value, expected);
            assert_eq!(snapshot.questions[0].options[1].value, expected);
        }
        let inline = work.replace(
            "<span>1.</span>\n        <span>单选题</span>\n        <p>普通话以哪种方言为基础方言？</p>",
            "1.<span>（单选题，5.0分）</span>第一<span>行</span><br>第二行<p>第三行</p>",
        );
        assert_eq!(
            parse_chapter_work_form(&inline)
                .expect("valid inline chapter work")
                .questions[0]
                .prompt,
            "第一行\n第二行\n第三行"
        );
        let bare = work.replace(
            "<span>1.</span>\n        <span>单选题</span>\n        <p>普通话以哪种方言为基础方言？</p>",
            "题<span>目</span><p>后段</p>",
        );
        assert_eq!(
            parse_chapter_work_form(&bare)
                .expect("valid chapter work without heading")
                .questions[0]
                .prompt,
            "题目\n后段"
        );
    }

    #[test]
    fn parses_chapter_work_form_fixture() {
        let fixture = include_str!("../../../../fixtures/legacy/chapter_work_11_work_001.html");
        let snapshot = parse_chapter_work_form(fixture).expect("chapter work form");
        assert_eq!(snapshot.title, "绪论测验");
        assert_eq!(snapshot.work_answer_id, 99001);
        assert_eq!(snapshot.total_question_num, 3);
        assert_eq!(snapshot.work_relation_id, 88001);
        assert_eq!(snapshot.full_score, "100");
        assert_eq!(snapshot.enc_work, "enc-work-submit-001");
        assert_eq!(snapshot.questions.len(), 3);

        assert_eq!(snapshot.questions[0].question_id, 700101);
        assert_eq!(snapshot.questions[0].question_type_label, "单选题");
        assert_eq!(snapshot.questions[0].prompt, "普通话以哪种方言为基础方言？");
        assert_eq!(snapshot.questions[0].options[1].key, "B");
        assert_eq!(snapshot.questions[0].options[1].value, "北方方言");
        assert!(snapshot.questions[0].options[1].rich_content.is_none());

        assert_eq!(snapshot.questions[1].question_type_label, "填空题");
        assert_eq!(snapshot.questions[1].blanks, vec!["词义：", "词性："]);

        assert_eq!(snapshot.questions[2].question_type_label, "判断题");
        assert!(snapshot.questions[2].options.is_empty());
    }

    #[test]
    fn rejects_chapter_work_access_denied_fixture() {
        let fixture =
            include_str!("../../../../fixtures/legacy/chapter_work_11_work_001_invalid.html");
        let err = parse_chapter_work_form(fixture).expect_err("invalid chapter work fixture");
        assert!(
            err.to_string()
                .contains("chapter work page rejected: 无效的权限")
        );
    }

    #[test]
    fn parses_exam_preview_option_rich_content() {
        let fixture = r#"
            <html>
              <body>
                <div class="questionWrap singleQuesId ans-cc-exam">
                  <input name="questionId" value="810001" />
                  <input name="type810001" value="0" />
                  <div class="tit">
                    <h3>单选题</h3>
                    1. 请选择图文选项
                  </div>
                  <div class="answerList radioList" name="A">
                    <cc><span>图文 <strong>选项</strong><img src="https://static.example/a.png" /></span></cc>
                  </div>
                  <div class="answerList radioList" name="B">
                    <cc><img src="https://static.example/b.png" /></cc>
                  </div>
                </div>
              </body>
            </html>
        "#;

        let questions = parse_exam_preview_questions(fixture).expect("preview rich option");
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].prompt, "请选择图文选项");
        assert_eq!(questions[0].options[0].value, "图文 选项");
        let rich_option = questions[0].options[0]
            .rich_content
            .as_ref()
            .expect("rich option metadata");
        assert!(rich_option.source_html.contains("<strong>选项</strong>"));
        assert_eq!(
            rich_option.image_urls,
            vec!["https://static.example/a.png".to_owned()]
        );

        assert_eq!(questions[0].options[1].value, "");
        let image_only_option = questions[0].options[1]
            .rich_content
            .as_ref()
            .expect("image-only option metadata");
        assert_eq!(
            image_only_option.image_urls,
            vec!["https://static.example/b.png".to_owned()]
        );
    }

    #[test]
    fn parses_chapter_work_option_rich_content() {
        let fixture = r#"
            <html>
              <body>
                <h3 class="py-Title">图文作业</h3>
                <form id="form1">
                  <input id="workAnswerId" value="99009" />
                  <input id="totalQuestionNum" value="1" />
                  <input id="workRelationId" value="88009" />
                  <input id="fullScore" value="100" />
                  <input id="enc_work" value="enc-rich-work" />
                </form>
                <div class="Py-mian1">
                  <div class="Py-m1-title">
                    <span>1.</span>
                    <span>单选题</span>
                    <p>请选择图文选项</p>
                  </div>
                  <input id="answertype710001" value="0" />
                  <ul>
                    <li class="more-choose-item">
                      <em class="choose-opt" id-param="A">A</em>
                      <div class="choose-desc">
                        <span>图文 <strong>选项</strong><img src="/static/work-a.png" /></span>
                      </div>
                    </li>
                    <li class="more-choose-item">
                      <em class="choose-opt" id-param="B">B</em>
                      <div class="choose-desc"><img src="/static/work-b.png" /></div>
                    </li>
                  </ul>
                </div>
              </body>
            </html>
        "#;

        let snapshot = parse_chapter_work_form(fixture).expect("chapter work rich option");
        assert_eq!(snapshot.questions.len(), 1);
        assert_eq!(snapshot.questions[0].prompt, "请选择图文选项");
        assert_eq!(snapshot.questions[0].options[0].value, "图文 选项");
        let rich_option = snapshot.questions[0].options[0]
            .rich_content
            .as_ref()
            .expect("rich chapter-work option metadata");
        assert!(rich_option.source_html.contains("<strong>选项</strong>"));
        assert_eq!(
            rich_option.image_urls,
            vec!["/static/work-a.png".to_owned()]
        );

        assert_eq!(snapshot.questions[0].options[1].value, "");
        let image_only_option = snapshot.questions[0].options[1]
            .rich_content
            .as_ref()
            .expect("image-only chapter-work option metadata");
        assert_eq!(
            image_only_option.image_urls,
            vec!["/static/work-b.png".to_owned()]
        );
    }

    #[tokio::test]
    async fn fetches_exam_preview_question_inventory_with_fixture_transport() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let course = Course {
            course_id: 1001,
            class_id: 2002,
            cpi: 3003,
            key: 4004,
            name: "现代汉语".to_owned(),
            teacher_name: "王老师".to_owned(),
            state: "进行中".to_owned(),
        };
        let exam = CourseExam {
            exam_id: 555001,
            course_id: course.course_id,
            class_id: course.class_id,
            cpi: course.cpi,
            enc_task: "enc-alpha".to_owned(),
            name: "第一单元测验".to_owned(),
            status: "待完成".to_owned(),
            expire_time: None,
            meta: None,
        };
        let preview = ExamPreviewQuery {
            exam_answer_id: 900001,
            enc: "preview-enc-alpha".to_owned(),
            remain_time_param: Some(3600),
            relation_answer_last_update_time: Some(1_710_000_000_000),
        };

        let questions = client
            .fetch_exam_preview_questions(&course, &exam, &preview)
            .await
            .expect("preview inventory through fixture transport");

        assert_eq!(questions.len(), 4);
        assert_eq!(questions[1].question_type_label, "多选题");
        assert_eq!(questions[2].blanks, vec!["词义：", "词性："]);
    }

    #[tokio::test]
    async fn fetches_task_scan_attachment_metadata_with_fixture_transport() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let course = Course {
            course_id: 1001,
            class_id: 2002,
            cpi: 3003,
            key: 4004,
            name: "现代汉语".to_owned(),
            teacher_name: "王老师".to_owned(),
            state: "进行中".to_owned(),
        };

        let chapters = client
            .fetch_task_scan_with_attachment_metadata(&course)
            .await
            .expect("task scan with attachment metadata");

        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].task_points.len(), 2);

        let video_metadata = chapters[0].task_points[0]
            .attachment_metadata
            .as_ref()
            .expect("video attachment metadata");
        assert_eq!(video_metadata.fid, Some(9876));
        assert_eq!(
            video_metadata
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.object_id.as_deref()),
            Some("video-001")
        );
        assert_eq!(
            video_metadata
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.playback_rate.as_deref()),
            Some("0.9")
        );
        assert_eq!(
            video_metadata
                .video_status
                .as_ref()
                .map(|status| status.duration_secs),
            Some(602)
        );

        assert!(chapters[0].task_points[1].attachment_metadata.is_none());

        let document_metadata = chapters[1].task_points[0]
            .attachment_metadata
            .as_ref()
            .expect("document attachment metadata");
        assert_eq!(document_metadata.fid, None);
        assert_eq!(
            document_metadata
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.file_type.as_deref()),
            Some("pdf")
        );
        assert!(document_metadata.video_status.is_none());
    }

    #[tokio::test]
    async fn fetches_live_task_scan_attachment_metadata_with_fixture_transport() {
        let fixture_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy_live");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));
        let course = Course {
            course_id: 1001,
            class_id: 2002,
            cpi: 3003,
            key: 4004,
            name: "现代汉语".to_owned(),
            teacher_name: "王老师".to_owned(),
            state: "进行中".to_owned(),
        };

        let chapters = client
            .fetch_task_scan_with_attachment_metadata(&course)
            .await
            .expect("live task scan with attachment metadata");

        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].task_points.len(), 1);
        assert_eq!(chapters[0].task_points[0].module, "insertlive");
        assert_eq!(chapters[0].task_points[0].resource_id, "live-course-001");

        let live_metadata = chapters[0].task_points[0]
            .attachment_metadata
            .as_ref()
            .expect("live attachment metadata");
        assert_eq!(live_metadata.fid, None);
        assert!(live_metadata.video_status.is_none());
        assert_eq!(
            live_metadata
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.attachment_type.as_deref()),
            Some("live")
        );
        assert_eq!(
            live_metadata
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.stream_name.as_deref()),
            Some("zhibo_12345")
        );
        assert_eq!(
            live_metadata
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.vdo_id.as_deref()),
            Some("vdo-live-001")
        );
        assert_eq!(
            live_metadata
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.live_id.as_deref()),
            Some("live-course-001")
        );
    }

    #[tokio::test]
    async fn fetches_chapter_work_form_with_fixture_transport() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));

        let snapshot = client
            .fetch_chapter_work_form(&ChapterWorkRuntimeRequest {
                course_id: 1001,
                class_id: 2002,
                knowledge_id: 11,
                user_id: 9009,
                cpi: 3003,
                card_index: 1,
                work_id: "work-001".to_owned(),
                job_id: "job-001".to_owned(),
                school_id: None,
            })
            .await
            .expect("chapter work form through fixture transport");

        assert_eq!(snapshot.title, "绪论测验");
        assert_eq!(snapshot.questions.len(), 3);
        assert_eq!(snapshot.questions[0].question_type_label, "单选题");
        assert_eq!(snapshot.questions[1].blanks, vec!["词义：", "词性："]);
    }

    #[tokio::test]
    async fn reports_video_progress_with_fixture_transport() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));

        let progress = client
            .report_video_progress(&VideoPlayReportRequest {
                cpi: 3003,
                class_id: 2002,
                user_id: 9009,
                dtoken: "video-dtoken-001".to_owned(),
                object_id: "video-001".to_owned(),
                job_id: "job-video-001".to_owned(),
                other_info: "nodeId_11".to_owned(),
                playing_time_secs: 58,
                duration_secs: 602,
                clip_time: "0_602".to_owned(),
                enc: "enc-001".to_owned(),
                playback_rate: "0.9".to_owned(),
                report_timestamp_millis: 1_717_000_000_123,
            })
            .await
            .expect("video progress ack through fixture transport");

        assert_eq!(progress.is_passed, Some(false));
        assert_eq!(progress.playing_time_secs, Some(58));

        let completion = client
            .report_video_progress(&VideoPlayReportRequest {
                cpi: 3003,
                class_id: 2002,
                user_id: 9009,
                dtoken: "video-dtoken-001".to_owned(),
                object_id: "video-001".to_owned(),
                job_id: "job-video-001".to_owned(),
                other_info: "nodeId_11".to_owned(),
                playing_time_secs: 602,
                duration_secs: 602,
                clip_time: "0_602".to_owned(),
                enc: "enc-001".to_owned(),
                playback_rate: "0.9".to_owned(),
                report_timestamp_millis: 1_717_000_000_123,
            })
            .await
            .expect("video completion ack through fixture transport");

        assert_eq!(completion.is_passed, Some(true));
        assert_eq!(completion.playing_time_secs, Some(602));
        assert_eq!(completion.duration_secs, Some(602));
    }

    #[tokio::test]
    async fn reports_document_progress_with_fixture_transport() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));

        let ack = client
            .report_document_progress(&DocumentReadingReportRequest {
                course_id: 1001,
                class_id: 2002,
                knowledge_id: 12,
                job_id: "job-doc-001".to_owned(),
                jtoken: "jtoken-doc-001".to_owned(),
                report_timestamp_millis: 1_717_000_000_123,
            })
            .await
            .expect("document reading-report ack through fixture transport");

        assert!(ack.success);

        let err = client
            .report_document_progress(&DocumentReadingReportRequest {
                course_id: 1001,
                class_id: 2002,
                knowledge_id: 12,
                job_id: "job-doc-001".to_owned(),
                jtoken: "jtoken-doc-001-error".to_owned(),
                report_timestamp_millis: 1_717_000_000_124,
            })
            .await
            .expect_err("document reading-report error fixture to fail");

        assert!(
            err.to_string()
                .contains("document reading-report rejected: document progress rejected")
        );
    }

    #[tokio::test]
    async fn reports_live_progress_with_fixture_transport() {
        let fixture_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy_live");
        let client = ChaoxingClient::new(Arc::new(FixtureChaoxingTransport::new(fixture_root)));

        let start_ack = client
            .report_live_progress(&LiveProgressReportRequest {
                stream_name: "zhibo_12345".to_owned(),
                vdo_id: "vdo-live-001".to_owned(),
                user_id: 9009,
                is_start: false,
                course_id: 1001,
                report_timestamp_millis: 1_717_000_000_123,
            })
            .await
            .expect("live start ack through fixture transport");
        assert!(start_ack.success);

        let heartbeat_ack = client
            .report_live_progress(&LiveProgressReportRequest {
                stream_name: "zhibo_12345".to_owned(),
                vdo_id: "vdo-live-001".to_owned(),
                user_id: 9009,
                is_start: true,
                course_id: 1001,
                report_timestamp_millis: 1_717_000_000_153,
            })
            .await
            .expect("live heartbeat ack through fixture transport");
        assert!(heartbeat_ack.success);

        let err = client
            .report_live_progress(&LiveProgressReportRequest {
                stream_name: "zhibo_12345".to_owned(),
                vdo_id: "vdo-live-001-error".to_owned(),
                user_id: 9009,
                is_start: true,
                course_id: 1001,
                report_timestamp_millis: 1_717_000_000_183,
            })
            .await
            .expect_err("live progress-report error fixture to fail");
        assert!(
            err.to_string()
                .contains("live progress-report rejected: live progress rejected")
        );
    }
}
