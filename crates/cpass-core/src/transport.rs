use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{COOKIE, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode};
use url::Url;

use crate::config::TransportConfig;
use crate::error::{CpassError, Result};
use crate::event::{RunEvent, RunEventSink};
use crate::models::CookieSnapshot;

#[derive(Debug, Clone)]
pub enum RequestBody {
    Empty,
    Form(BTreeMap<String, String>),
}

#[derive(Debug, Clone)]
pub struct TransportRequest {
    pub method: Method,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub raw_query: Option<String>,
    pub headers: HeaderMap,
    pub body: RequestBody,
}

impl TransportRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::GET,
            url: url.into(),
            query: Vec::new(),
            raw_query: None,
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
        }
    }

    pub fn post_form(url: impl Into<String>, form: BTreeMap<String, String>) -> Self {
        Self {
            method: Method::POST,
            url: url.into(),
            query: Vec::new(),
            raw_query: None,
            headers: HeaderMap::new(),
            body: RequestBody::Form(form),
        }
    }

    pub fn with_raw_query(mut self, raw_query: impl Into<String>) -> Self {
        self.query.clear();
        self.raw_query = Some(raw_query.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct TransportResponse {
    pub status: StatusCode,
    pub final_url: String,
    pub body: String,
}

impl TransportResponse {
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_str(&self.body)?)
    }
}

#[async_trait]
pub trait ChaoxingTransport: Send + Sync {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse>;
}

#[derive(Debug, Clone)]
pub struct FixtureChaoxingTransport {
    root: PathBuf,
}

impl FixtureChaoxingTransport {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn fixture_path(&self, request: &TransportRequest) -> Result<PathBuf> {
        let url = Url::parse(&request.url)?;
        let path = url.path();
        let query = request_query_map(request);
        let file_name = if path.ends_with("/mycourse/backclazzdata") {
            "course_list.json"
        } else if path.ends_with("/login") {
            "login_page_qr.html"
        } else if path.ends_with("/createqr") {
            "qr_create_response.txt"
        } else if path.ends_with("/getauthstatus") {
            "qr_auth_status_success.json"
        } else if path.ends_with("/gas/clazz") {
            "chapter_list.json"
        } else if path.ends_with("/knowledge/cards") {
            match (
                query.get("knowledgeid").map(String::as_str),
                query.get("num").map(String::as_str),
            ) {
                (Some("11"), Some("0")) => "chapter_card_attachment_11_0.html",
                (Some("11"), Some("1")) => "chapter_card_attachment_11_1.html",
                (Some("12"), Some("0")) => "chapter_card_attachment_12_0.html",
                (Some("13"), Some("0")) => "chapter_card_attachment_13_0.html",
                _ => {
                    return Err(CpassError::UnexpectedResponse(format!(
                        "fixture transport has no chapter-card attachment fixture for request {:?}",
                        request.query
                    )));
                }
            }
        } else if path.ends_with("/gas/knowledge") {
            match query.get("id").map(String::as_str) {
                Some("11") => "chapter_cards_11.json",
                Some("12") => "chapter_cards_12.json",
                Some("13") => "chapter_cards_13.json",
                _ => {
                    return Err(CpassError::UnexpectedResponse(format!(
                        "fixture transport has no chapter-card fixture for request {:?}",
                        request.query
                    )));
                }
            }
        } else if path.ends_with("/exam-ans/exam/phone/task-exam") {
            match query.get("taskrefId").map(String::as_str) {
                Some("555001") => "exam_cover_555001.html",
                Some("555002") => "exam_cover_555002.html",
                Some("555003") => "exam_cover_555003.html",
                Some("555004") => "exam_cover_555004.html",
                Some("555005") => "exam_cover_555005.html",
                Some("555006") => "exam_cover_555006.html",
                _ => {
                    return Err(CpassError::UnexpectedResponse(format!(
                        "fixture transport has no exam-cover fixture for request {:?}",
                        request.query
                    )));
                }
            }
        } else if path.ends_with("/exam-ans/exam/phone/preview") {
            match query.get("examRelationId").map(String::as_str) {
                Some("555001") => "exam_preview_555001.html",
                _ => {
                    return Err(CpassError::UnexpectedResponse(format!(
                        "fixture transport has no exam-preview fixture for request {:?}",
                        request.query
                    )));
                }
            }
        } else if path.ends_with("/android/mworkspecial") {
            match (
                query.get("workid").map(String::as_str),
                query.get("jobid").map(String::as_str),
                query.get("knowledgeid").map(String::as_str),
                query.get("ktoken").map(String::as_str),
                query.get("enc").map(String::as_str),
            ) {
                (
                    Some("work-001"),
                    Some("job-001"),
                    Some("11"),
                    Some("ktoken-work-001"),
                    Some("enc-work-fetch-001"),
                ) => "chapter_work_11_work_001.html",
                _ => {
                    return Err(CpassError::UnexpectedResponse(format!(
                        "fixture transport has no chapter-work page fixture for request {:?}",
                        query
                    )));
                }
            }
        } else if path.ends_with("/ananas/job/document") {
            match (
                query.get("jobid").map(String::as_str),
                query.get("knowledgeid").map(String::as_str),
                query.get("jtoken").map(String::as_str),
            ) {
                (Some("job-doc-001"), Some("12"), Some("jtoken-doc-001")) => {
                    "document_reading_report_doc_001.json"
                }
                (Some("job-doc-001"), Some("12"), Some("jtoken-doc-001-error")) => {
                    "document_reading_report_doc_001_error.json"
                }
                _ => {
                    return Err(CpassError::UnexpectedResponse(format!(
                        "fixture transport has no document reading-report fixture for request {:?}",
                        query
                    )));
                }
            }
        } else if path.ends_with("/saveTimePc") {
            match (
                query.get("streamName").map(String::as_str),
                query.get("vdoid").map(String::as_str),
                query.get("isStart").map(String::as_str),
                query.get("courseId").map(String::as_str),
            ) {
                (Some("zhibo_12345"), Some("vdo-live-001"), Some("0"), Some("1001"))
                | (Some("zhibo_12345"), Some("vdo-live-001"), Some("1"), Some("1001")) => {
                    "live_progress_report_live_001.txt"
                }
                (Some("zhibo_12345"), Some("vdo-live-001-error"), Some("1"), Some("1001")) => {
                    "live_progress_report_live_001_error.txt"
                }
                _ => {
                    return Err(CpassError::UnexpectedResponse(format!(
                        "fixture transport has no live progress-report fixture for request {:?}",
                        query
                    )));
                }
            }
        } else if path.contains("/multimedia/log/a/") {
            match (
                path.rsplit('/').next(),
                query.get("playingTime").map(String::as_str),
            ) {
                (Some("video-dtoken-001"), Some("58")) => "video_play_report_video_001_58.json",
                (Some("video-dtoken-001"), Some("602")) => "video_play_report_video_001.json",
                _ => {
                    return Err(CpassError::UnexpectedResponse(format!(
                        "fixture transport has no video play-report fixture for path {path} and request {:?}",
                        query
                    )));
                }
            }
        } else if path.contains("/ananas/status/") {
            match path.rsplit('/').next() {
                Some("video-001") => "video_status_video_001.json",
                _ => {
                    return Err(CpassError::UnexpectedResponse(format!(
                        "fixture transport has no video-status fixture for path {path}",
                    )));
                }
            }
        } else if path.ends_with("/job/myjobsnodesmap") {
            "chapter_status.json"
        } else if path.ends_with("/exam/phone/task-list") {
            "exam_list.html"
        } else if path.ends_with("/apis/login/userLogin4Uname.do") {
            "account_info.json"
        } else {
            return Err(CpassError::UnsupportedCommand(
                "fixture transport does not have a mapping for this request",
            ));
        };
        Ok(self.root.join(file_name))
    }
}

#[derive(Clone)]
pub struct ReqwestChaoxingTransport {
    client: reqwest::Client,
    config: TransportConfig,
    cookies: CookieSnapshot,
    sink: Option<Arc<dyn RunEventSink>>,
}

impl ReqwestChaoxingTransport {
    pub fn new(config: TransportConfig, cookies: CookieSnapshot) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("cpass-rs/0.1")
            .build()?;
        Ok(Self {
            client,
            config,
            cookies,
            sink: None,
        })
    }

    pub fn with_cookie_jar(config: TransportConfig) -> Result<(Self, Arc<Jar>)> {
        let jar = Arc::new(Jar::default());
        let client = reqwest::Client::builder()
            .cookie_provider(jar.clone())
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("cpass-rs/0.1")
            .build()?;
        Ok((
            Self {
                client,
                config,
                cookies: CookieSnapshot::empty(),
                sink: None,
            },
            jar,
        ))
    }

    pub fn with_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn attach_cookie_header(
        &self,
        request: &TransportRequest,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder> {
        let url = Url::parse(&request.url)?;
        if request.headers.contains_key(COOKIE) {
            return Ok(builder);
        }
        if let Some(cookies) = self.cookies.get(url.host_str().unwrap_or_default()) {
            return Ok(builder.header(
                COOKIE,
                HeaderValue::from_str(cookies).map_err(|err| {
                    CpassError::Config(format!("invalid cookie header for {}: {err}", request.url))
                })?,
            ));
        }
        Ok(builder)
    }

    pub fn snapshot_from_jar(jar: &Jar) -> CookieSnapshot {
        let mut snapshot = CookieSnapshot::empty();
        let urls = [
            "https://passport2.chaoxing.com",
            "https://sso.chaoxing.com",
            "https://mooc1-api.chaoxing.com",
            "https://mooc1.chaoxing.com",
        ];

        for url in urls {
            if let Ok(parsed) = Url::parse(url)
                && let Some(cookies) = jar.cookies(&parsed)
                && let Ok(cookies) = cookies.to_str()
                && let Some(host) = parsed.host_str()
            {
                snapshot.insert(host, cookies.to_owned());
            }
        }
        snapshot
    }
}

fn request_query_map(request: &TransportRequest) -> BTreeMap<String, String> {
    match &request.raw_query {
        Some(raw_query) => url::form_urlencoded::parse(raw_query.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
        None => request.query.iter().cloned().collect(),
    }
}

#[async_trait]
impl ChaoxingTransport for ReqwestChaoxingTransport {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        let mut attempt = 0_u32;

        loop {
            attempt += 1;
            let request_url = match &request.raw_query {
                Some(raw_query) => format!("{}?{raw_query}", request.url),
                None => request.url.clone(),
            };
            let builder = self
                .client
                .request(request.method.clone(), &request_url)
                .headers(request.headers.clone());
            let builder = match &request.raw_query {
                Some(_) => builder,
                None => builder.query(&request.query),
            };
            let mut builder = self.attach_cookie_header(&request, builder)?;
            match &request.body {
                RequestBody::Empty => {}
                RequestBody::Form(form) => {
                    builder = builder.form(form);
                }
            }

            match builder.send().await {
                Ok(response) => {
                    let status = response.status();
                    let final_url = response.url().to_string();
                    let body = response.text().await?;
                    return Ok(TransportResponse {
                        status,
                        final_url,
                        body,
                    });
                }
                Err(error) if attempt <= self.config.retries => {
                    if let Some(sink) = &self.sink {
                        sink.emit(RunEvent::RetryScheduled {
                            url: request.url.clone(),
                            attempt,
                            reason: error.to_string(),
                        });
                    }
                    tokio::time::sleep(Duration::from_millis(self.config.retry_delay_millis)).await;
                }
                Err(error) => return Err(CpassError::Http(error)),
            }
        }
    }
}

#[async_trait]
impl ChaoxingTransport for FixtureChaoxingTransport {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        let path = self.fixture_path(&request)?;
        let final_url = if request.url.ends_with("/exam-ans/exam/phone/task-exam")
            && request
                .query
                .iter()
                .any(|(key, value)| key == "taskrefId" && value == "555002")
        {
            "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/look".to_owned()
        } else {
            request.url.clone()
        };
        let body = fs::read_to_string(&path).map_err(|error| {
            CpassError::UnexpectedResponse(format!(
                "failed to read fixture response {}: {error}",
                path.display()
            ))
        })?;
        Ok(TransportResponse {
            status: StatusCode::OK,
            final_url,
            body,
        })
    }
}
