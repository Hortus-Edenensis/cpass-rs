use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{COOKIE, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use reqwest::{Method, StatusCode};
use url::Url;

use crate::config::TransportConfig;
use crate::error::{CpassError, Result};
use crate::event::{RunEvent, RunEventSink};
use crate::models::CookieSnapshot;

const CHA0XING_APP_USER_AGENT: &str = "Dalvik/2.1.0 (Linux; U; Android 12; MI12 Build/SKQ1.211006.001) (schild:6b4dd07967f3ebc3fdbf9e89fbd2d0a1) (device:MI12) Language/zh_CN com.chaoxing.mobile/ChaoXingStudy_3_6.3.9_android_phone_10824_250 (@Kalimdor)_0123456789abcdef0123456789abcdef";
const X_REQUESTED_WITH_HEADER: &str = "x-requested-with";
const CHA0XING_APP_PACKAGE: &str = "com.chaoxing.mobile";

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
        serde_json::from_str(&self.body).map_err(|error| {
            let preview = self
                .body
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(120)
                .collect::<String>();
            CpassError::UnexpectedResponse(format!(
                "failed to parse JSON response from {} (status {}): {error}; body starts with {:?}",
                self.final_url, self.status, preview
            ))
        })
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

    fn apply_default_request_headers(headers: &mut HeaderMap) {
        if !headers.contains_key(USER_AGENT) {
            headers.insert(
                USER_AGENT,
                HeaderValue::from_static(CHA0XING_APP_USER_AGENT),
            );
        }
        let requested_with = HeaderName::from_static(X_REQUESTED_WITH_HEADER);
        if !headers.contains_key(&requested_with) {
            headers.insert(
                requested_with,
                HeaderValue::from_static(CHA0XING_APP_PACKAGE),
            );
        }
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
            let mut headers = request.headers.clone();
            Self::apply_default_request_headers(&mut headers);
            let builder = self
                .client
                .request(request.method.clone(), &request_url)
                .headers(headers);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reqwest_transport_applies_mobile_app_headers_by_default() {
        let mut headers = HeaderMap::new();
        ReqwestChaoxingTransport::apply_default_request_headers(&mut headers);

        assert_eq!(
            headers
                .get(USER_AGENT)
                .and_then(|value| value.to_str().ok()),
            Some(CHA0XING_APP_USER_AGENT)
        );
        assert_eq!(
            headers
                .get(HeaderName::from_static(X_REQUESTED_WITH_HEADER))
                .and_then(|value| value.to_str().ok()),
            Some(CHA0XING_APP_PACKAGE)
        );
    }

    #[test]
    fn reqwest_transport_keeps_explicit_user_agent_overrides() {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 test"));

        ReqwestChaoxingTransport::apply_default_request_headers(&mut headers);

        assert_eq!(
            headers
                .get(USER_AGENT)
                .and_then(|value| value.to_str().ok()),
            Some("Mozilla/5.0 test")
        );
        assert_eq!(
            headers
                .get(HeaderName::from_static(X_REQUESTED_WITH_HEADER))
                .and_then(|value| value.to_str().ok()),
            Some(CHA0XING_APP_PACKAGE)
        );
    }

    #[test]
    fn transport_response_json_reports_url_status_and_body_preview() {
        let response = TransportResponse {
            status: StatusCode::FORBIDDEN,
            final_url: "https://mooc1-api.chaoxing.com/gas/clazz".to_owned(),
            body: "<!doctype html><title>403</title>".to_owned(),
        };

        let error = response
            .json::<serde_json::Value>()
            .expect_err("html response should fail JSON parsing");

        let message = error.to_string();
        assert!(message.contains("failed to parse JSON response"));
        assert!(message.contains("https://mooc1-api.chaoxing.com/gas/clazz"));
        assert!(message.contains("403"));
        assert!(message.contains("<!doctype html>"));
    }
}
