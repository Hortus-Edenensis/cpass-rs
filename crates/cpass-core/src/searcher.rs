use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::SearcherConfig;
use crate::error::{CpassError, Result};
use crate::models::{
    ChapterWorkFormSnapshot, ChapterWorkQuestionSummary, ExamQuestionOption, ExamQuestionSummary,
};
use crate::question_kind::NormalizedQuestionKind;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerQuerySource {
    ChapterWork,
    ExamPreview,
}

pub type AnswerQuestionKind = NormalizedQuestionKind;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnswerQuery {
    pub source: AnswerQuerySource,
    pub question_index: usize,
    pub question_id: u64,
    pub question_type: u64,
    pub question_type_label: String,
    pub question_kind: AnswerQuestionKind,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ExamQuestionOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blanks: Vec<String>,
}

impl AnswerQuery {
    #[must_use]
    pub fn render_search_text(&self) -> String {
        let mut lines = vec![format!(
            "[{} / {}] {}",
            self.question_type_label,
            self.question_kind.as_str(),
            self.prompt
        )];

        lines.extend(self.options.iter().map(render_option_search_line));
        lines.extend(
            self.blanks
                .iter()
                .enumerate()
                .map(|(index, blank)| format!("Blank {}: {}", index + 1, blank)),
        );

        lines.join("\n")
    }
}

fn render_option_search_line(option: &ExamQuestionOption) -> String {
    format!("{}. {}", option.key, render_option_search_value(option))
}

fn render_option_search_value(option: &ExamQuestionOption) -> String {
    let mut fragments = Vec::new();
    let visible_text = option.value.trim();

    if !visible_text.is_empty() {
        fragments.push(visible_text.to_owned());
    } else if option.rich_content.is_some() {
        fragments.push("[no visible option text]".to_owned());
    }

    if let Some(rich_content) = &option.rich_content {
        if let Some(source_html) = normalize_searcher_source_html(&rich_content.source_html) {
            fragments.push(format!("rich_html={source_html}"));
        }
        if !rich_content.image_urls.is_empty() {
            fragments.push(format!("image_urls={}", rich_content.image_urls.join(", ")));
        }
    }

    fragments.join(" | ")
}

fn normalize_searcher_source_html(source_html: &str) -> Option<String> {
    let normalized = source_html.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

impl From<&ExamQuestionSummary> for AnswerQuery {
    fn from(value: &ExamQuestionSummary) -> Self {
        Self {
            source: AnswerQuerySource::ExamPreview,
            question_index: value.question_index,
            question_id: value.question_id,
            question_type: value.question_type,
            question_type_label: value.question_type_label.clone(),
            question_kind: AnswerQuestionKind::from_question_type(value.question_type),
            prompt: value.prompt.clone(),
            options: value.options.clone(),
            blanks: value.blanks.clone(),
        }
    }
}

impl From<&ChapterWorkQuestionSummary> for AnswerQuery {
    fn from(value: &ChapterWorkQuestionSummary) -> Self {
        Self {
            source: AnswerQuerySource::ChapterWork,
            question_index: value.question_index,
            question_id: value.question_id,
            question_type: value.question_type,
            question_type_label: value.question_type_label.clone(),
            question_kind: AnswerQuestionKind::from_question_type(value.question_type),
            prompt: value.prompt.clone(),
            options: value.options.clone(),
            blanks: value.blanks.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterWorkQueryBatch {
    pub title: String,
    pub work_answer_id: u64,
    pub work_relation_id: u64,
    pub total_question_num: usize,
    pub queries: Vec<AnswerQuery>,
}

impl ChapterWorkQueryBatch {
    #[must_use]
    pub fn len(&self) -> usize {
        self.queries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queries.is_empty()
    }
}

impl From<&ChapterWorkFormSnapshot> for ChapterWorkQueryBatch {
    fn from(value: &ChapterWorkFormSnapshot) -> Self {
        Self {
            title: value.title.clone(),
            work_answer_id: value.work_answer_id,
            work_relation_id: value.work_relation_id,
            total_question_num: value.total_question_num,
            queries: value.questions.iter().map(AnswerQuery::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnswerCandidateSelection {
    pub query: AnswerQuery,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<AnswerCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_candidate: Option<AnswerCandidate>,
}

impl AnswerCandidateSelection {
    #[must_use]
    pub fn new(query: AnswerQuery, candidates: Vec<AnswerCandidate>) -> Self {
        let selected_candidate = candidates.first().cloned();
        Self {
            query,
            candidates,
            selected_candidate,
        }
    }

    #[must_use]
    pub fn is_resolved(&self) -> bool {
        self.selected_candidate.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnswerCandidate {
    pub provider: String,
    pub confidence: Option<f32>,
    pub answer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChapterWorkCandidateSelectionBatch {
    pub title: String,
    pub work_answer_id: u64,
    pub work_relation_id: u64,
    pub total_question_num: usize,
    pub selections: Vec<AnswerCandidateSelection>,
}

impl ChapterWorkCandidateSelectionBatch {
    #[must_use]
    pub fn len(&self) -> usize {
        self.selections.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.selections.is_empty()
    }

    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.selections
            .iter()
            .filter(|selection| selection.is_resolved())
            .count()
    }

    #[must_use]
    pub fn unresolved_count(&self) -> usize {
        self.selections.len().saturating_sub(self.selected_count())
    }
}

#[async_trait]
pub trait SearcherProvider: Send + Sync {
    async fn search(&self, query: &AnswerQuery) -> Result<Vec<AnswerCandidate>>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HttpSearcherMethod {
    Get,
    Post,
}

impl HttpSearcherMethod {
    fn from_config_value(raw: Option<&str>, config_kind: &str) -> Result<Self> {
        match raw.unwrap_or("POST") {
            value if value.eq_ignore_ascii_case("get") => Ok(Self::Get),
            value if value.eq_ignore_ascii_case("post") => Ok(Self::Post),
            other => Err(CpassError::Config(format!(
                "searcher type '{config_kind}' field 'method' must be GET or POST, got '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HttpSearcherPayloadMode {
    Form,
    Json,
}

impl HttpSearcherPayloadMode {
    fn from_config_value(raw: Option<&str>, config_kind: &str) -> Result<Self> {
        match raw.unwrap_or("form") {
            value if value.eq_ignore_ascii_case("form") => Ok(Self::Form),
            value if value.eq_ignore_ascii_case("json") => Ok(Self::Json),
            other => Err(CpassError::Config(format!(
                "searcher type '{config_kind}' field 'payload_mode' must be form or json, got '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpSearcherRequestTemplate {
    pub url: String,
    pub method: HttpSearcherMethod,
    pub payload_mode: HttpSearcherPayloadMode,
    pub question_field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_field: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra_fields: BTreeMap<String, String>,
    pub answer_path: String,
}

impl HttpSearcherRequestTemplate {
    pub fn from_config(config: &SearcherConfig) -> Result<Self> {
        let payload_mode = if config.kind.eq_ignore_ascii_case("jsonapisearcher") {
            HttpSearcherPayloadMode::Json
        } else if config.kind.eq_ignore_ascii_case("restapisearcher") {
            HttpSearcherPayloadMode::Form
        } else {
            HttpSearcherPayloadMode::from_config_value(
                config
                    .values
                    .get("payload_mode")
                    .and_then(serde_json::Value::as_str),
                &config.kind,
            )?
        };
        let method = HttpSearcherMethod::from_config_value(
            config
                .values
                .get("method")
                .and_then(serde_json::Value::as_str),
            &config.kind,
        )?;
        if matches!(payload_mode, HttpSearcherPayloadMode::Json)
            && matches!(method, HttpSearcherMethod::Get)
        {
            return Err(CpassError::Config(format!(
                "searcher type '{}' cannot use method GET with payload_mode json",
                config.kind
            )));
        }

        Ok(Self {
            url: required_searcher_string_field(config, "url")?,
            method,
            payload_mode,
            question_field: optional_searcher_string_field(config, "q_field")?
                .unwrap_or_else(|| "question".to_owned()),
            option_field: optional_searcher_string_field(config, "o_field")?,
            headers: searcher_string_map_field(config, "headers")?,
            extra_fields: searcher_string_map_field(config, "ext_params")?,
            answer_path: optional_searcher_string_field(config, "a_field")?
                .unwrap_or_else(|| "$.data".to_owned()),
        })
    }

    #[must_use]
    pub fn build_request(&self, query: &AnswerQuery) -> HttpSearcherRequest {
        let option_text = render_http_option_text(query);
        match (self.method, self.payload_mode) {
            (HttpSearcherMethod::Get, HttpSearcherPayloadMode::Form) => HttpSearcherRequest {
                url: self.url.clone(),
                method: self.method,
                headers: self.headers.clone(),
                answer_path: self.answer_path.clone(),
                payload: HttpSearcherRequestPayload::Query {
                    fields: self.scalar_fields(query, &option_text),
                },
            },
            (HttpSearcherMethod::Post, HttpSearcherPayloadMode::Form) => HttpSearcherRequest {
                url: self.url.clone(),
                method: self.method,
                headers: self.headers.clone(),
                answer_path: self.answer_path.clone(),
                payload: HttpSearcherRequestPayload::Form {
                    fields: self.scalar_fields(query, &option_text),
                },
            },
            (HttpSearcherMethod::Post, HttpSearcherPayloadMode::Json) => HttpSearcherRequest {
                url: self.url.clone(),
                method: self.method,
                headers: self.headers.clone(),
                answer_path: self.answer_path.clone(),
                payload: HttpSearcherRequestPayload::Json {
                    body: self.json_body(query, &option_text),
                },
            },
            (HttpSearcherMethod::Get, HttpSearcherPayloadMode::Json) => {
                unreachable!("HTTP JSON request templates reject GET during config validation")
            }
        }
    }

    fn scalar_fields(&self, query: &AnswerQuery, option_text: &str) -> BTreeMap<String, String> {
        let mut fields = self.extra_fields.clone();
        fields.insert(self.question_field.clone(), query.prompt.clone());
        if let Some(option_field) = &self.option_field
            && !option_text.is_empty()
        {
            fields.insert(option_field.clone(), option_text.to_owned());
        }
        fields
    }

    fn json_body(&self, query: &AnswerQuery, option_text: &str) -> serde_json::Value {
        let mut body = serde_json::Map::new();
        for (key, value) in &self.extra_fields {
            body.insert(key.clone(), serde_json::Value::String(value.clone()));
        }
        body.insert(
            self.question_field.clone(),
            serde_json::Value::String(query.prompt.clone()),
        );
        body.insert(
            "type".to_owned(),
            serde_json::Value::Number(query.question_type.into()),
        );
        body.insert(
            "id".to_owned(),
            serde_json::Value::Number(query.question_id.into()),
        );
        if let Some(option_field) = &self.option_field {
            if !option_text.is_empty() {
                body.insert(
                    option_field.clone(),
                    serde_json::Value::String(option_text.to_owned()),
                );
            }
        } else if !query.options.is_empty() {
            let option_map = query
                .options
                .iter()
                .map(|option| (option.key.clone(), render_option_search_value(option)))
                .collect::<BTreeMap<_, _>>();
            body.insert(
                "options".to_owned(),
                serde_json::to_value(option_map).expect("option map to serialize"),
            );
        }

        serde_json::Value::Object(body)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HttpSearcherRequest {
    pub url: String,
    pub method: HttpSearcherMethod,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    pub answer_path: String,
    pub payload: HttpSearcherRequestPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HttpSearcherRequestPayload {
    Query { fields: BTreeMap<String, String> },
    Form { fields: BTreeMap<String, String> },
    Json { body: serde_json::Value },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HttpSearcherResponsePathSegment {
    Key(String),
    Index(usize),
}

#[async_trait]
trait HttpSearcherBackend: Send + Sync {
    async fn execute(
        &self,
        request: HttpSearcherRequest,
        headers: &HeaderMap,
    ) -> Result<serde_json::Value>;
}

#[derive(Debug, Clone)]
struct ReqwestHttpSearcherBackend {
    client: reqwest::Client,
}

impl ReqwestHttpSearcherBackend {
    fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().build()?,
        })
    }
}

#[async_trait]
impl HttpSearcherBackend for ReqwestHttpSearcherBackend {
    async fn execute(
        &self,
        request: HttpSearcherRequest,
        headers: &HeaderMap,
    ) -> Result<serde_json::Value> {
        let mut builder = match (request.method, request.payload) {
            (HttpSearcherMethod::Get, HttpSearcherRequestPayload::Query { fields }) => {
                self.client.get(request.url).query(&fields)
            }
            (HttpSearcherMethod::Post, HttpSearcherRequestPayload::Form { fields }) => {
                self.client.post(request.url).form(&fields)
            }
            (HttpSearcherMethod::Post, HttpSearcherRequestPayload::Json { body }) => {
                self.client.post(request.url).json(&body)
            }
            (method, payload) => {
                return Err(CpassError::Config(format!(
                    "http searcher request template produced unsupported request combination: method {method:?}, payload {payload:?}"
                )));
            }
        };

        for (name, value) in headers {
            builder = builder.header(name, value);
        }

        let response = builder.send().await?.error_for_status()?;
        Ok(response.json::<serde_json::Value>().await?)
    }
}

#[derive(Debug, Clone)]
struct FixtureHttpSearcherBackend {
    response: serde_json::Value,
}

impl FixtureHttpSearcherBackend {
    fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let response = serde_json::from_str(&fs::read_to_string(path.as_ref())?)?;
        Ok(Self { response })
    }
}

#[async_trait]
impl HttpSearcherBackend for FixtureHttpSearcherBackend {
    async fn execute(
        &self,
        _request: HttpSearcherRequest,
        _headers: &HeaderMap,
    ) -> Result<serde_json::Value> {
        Ok(self.response.clone())
    }
}

pub struct HttpSearcherProvider {
    request_template: HttpSearcherRequestTemplate,
    response_path: Vec<HttpSearcherResponsePathSegment>,
    request_headers: HeaderMap,
    provider_label: String,
    backend: Arc<dyn HttpSearcherBackend>,
}

impl std::fmt::Debug for HttpSearcherProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpSearcherProvider")
            .field("request_template", &self.request_template)
            .field("response_path", &self.response_path)
            .field("request_headers", &self.request_headers)
            .field("provider_label", &self.provider_label)
            .finish_non_exhaustive()
    }
}

impl Clone for HttpSearcherProvider {
    fn clone(&self) -> Self {
        Self {
            request_template: self.request_template.clone(),
            response_path: self.response_path.clone(),
            request_headers: self.request_headers.clone(),
            provider_label: self.provider_label.clone(),
            backend: Arc::clone(&self.backend),
        }
    }
}

impl HttpSearcherProvider {
    pub fn from_config(config: &SearcherConfig) -> Result<Self> {
        Self::from_config_with_base_dir(config, Path::new("."))
    }

    pub fn from_config_with_base_dir(
        config: &SearcherConfig,
        config_base_dir: &Path,
    ) -> Result<Self> {
        let request_template = HttpSearcherRequestTemplate::from_config(config)?;
        let backend: Arc<dyn HttpSearcherBackend> = if let Some(path) =
            resolve_searcher_path(config, config_base_dir, "fixture_response_path")?
        {
            Arc::new(FixtureHttpSearcherBackend::from_path(path)?)
        } else {
            Arc::new(ReqwestHttpSearcherBackend::new()?)
        };
        Self::from_request_template_with_backend(request_template, &config.kind, backend)
    }

    fn from_request_template_with_backend(
        request_template: HttpSearcherRequestTemplate,
        config_kind: &str,
        backend: Arc<dyn HttpSearcherBackend>,
    ) -> Result<Self> {
        validate_http_searcher_url(&request_template.url, config_kind)?;
        let response_path =
            parse_http_searcher_response_path(&request_template.answer_path, config_kind)?;
        let request_headers =
            build_http_searcher_header_map(&request_template.headers, config_kind)?;
        let provider_label = format!("http:{}", request_template.url);

        Ok(Self {
            request_template,
            response_path,
            request_headers,
            provider_label,
            backend,
        })
    }

    fn extract_answers(&self, response: &serde_json::Value) -> Vec<String> {
        let Some(value) = resolve_http_searcher_response_value(response, &self.response_path)
        else {
            return Vec::new();
        };

        let mut answers = Vec::new();
        collect_http_searcher_answer_strings(value, &mut answers);
        answers
    }
}

#[async_trait]
impl SearcherProvider for HttpSearcherProvider {
    async fn search(&self, query: &AnswerQuery) -> Result<Vec<AnswerCandidate>> {
        let request = self.request_template.build_request(query);
        let response = self.backend.execute(request, &self.request_headers).await?;
        Ok(self
            .extract_answers(&response)
            .into_iter()
            .map(|answer| AnswerCandidate {
                provider: self.provider_label.clone(),
                confidence: None,
                answer,
            })
            .collect())
    }
}

const DEFAULT_OPENAI_COMPATIBLE_SYSTEM_PROMPT: &str =
    "你是一位答题助手。请只输出答案本身，不要解释。";
const DEFAULT_OPENAI_COMPATIBLE_PROMPT_TEMPLATE: &str = "{search_text}";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OpenAiCompatibleMessageRole {
    System,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenAiCompatibleMessage {
    pub role: OpenAiCompatibleMessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenAiCompatibleRequest {
    pub model: String,
    pub messages: Vec<OpenAiCompatibleMessage>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenAiCompatibleResponse {
    pub choices: Vec<OpenAiCompatibleResponseChoice>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenAiCompatibleResponseChoice {
    pub message: OpenAiCompatibleResponseMessage,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenAiCompatibleResponseMessage {
    #[serde(default)]
    pub content: Option<String>,
}

impl OpenAiCompatibleResponse {
    #[must_use]
    pub fn first_message_content(&self) -> Option<&str> {
        self.choices
            .iter()
            .find_map(|choice| choice.message.content.as_deref())
            .map(str::trim)
            .filter(|content| !content.is_empty())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OpenAiCompatibleRequestTemplate {
    endpoint_url: String,
    api_key: String,
    model: String,
    system_prompt: String,
    prompt_template: String,
}

impl std::fmt::Debug for OpenAiCompatibleRequestTemplate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiCompatibleRequestTemplate")
            .field("endpoint_url", &self.endpoint_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("system_prompt", &self.system_prompt)
            .field("prompt_template", &self.prompt_template)
            .finish()
    }
}

impl OpenAiCompatibleRequestTemplate {
    pub fn from_config(config: &SearcherConfig) -> Result<Self> {
        let endpoint_url = normalize_openai_compatible_endpoint_url(
            &required_searcher_string_field(config, "base_url")?,
            &config.kind,
        )?;
        let api_key = required_searcher_string_field(config, "api_key")?;
        let model = required_searcher_string_field(config, "model")?;
        let system_prompt = optional_searcher_string_field(config, "system_prompt")?
            .unwrap_or_else(|| DEFAULT_OPENAI_COMPATIBLE_SYSTEM_PROMPT.to_owned());
        let prompt_template = resolve_openai_compatible_prompt_template(config)?;

        Ok(Self {
            endpoint_url,
            api_key,
            model,
            system_prompt,
            prompt_template,
        })
    }

    #[must_use]
    pub fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }

    #[must_use]
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    #[must_use]
    pub fn prompt_template(&self) -> &str {
        &self.prompt_template
    }

    #[must_use]
    pub fn build_request(&self, query: &AnswerQuery) -> OpenAiCompatibleRequest {
        OpenAiCompatibleRequest {
            model: self.model.clone(),
            messages: vec![
                OpenAiCompatibleMessage {
                    role: OpenAiCompatibleMessageRole::System,
                    content: self.system_prompt.clone(),
                },
                OpenAiCompatibleMessage {
                    role: OpenAiCompatibleMessageRole::User,
                    content: render_openai_compatible_prompt(&self.prompt_template, query),
                },
            ],
        }
    }
}

#[async_trait]
trait OpenAiCompatibleBackend: Send + Sync {
    async fn execute(
        &self,
        endpoint_url: &str,
        api_key: &str,
        request: OpenAiCompatibleRequest,
    ) -> Result<OpenAiCompatibleResponse>;
}

#[derive(Debug, Clone)]
struct ReqwestOpenAiCompatibleBackend {
    client: reqwest::Client,
}

impl ReqwestOpenAiCompatibleBackend {
    fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().build()?,
        })
    }
}

#[async_trait]
impl OpenAiCompatibleBackend for ReqwestOpenAiCompatibleBackend {
    async fn execute(
        &self,
        endpoint_url: &str,
        api_key: &str,
        request: OpenAiCompatibleRequest,
    ) -> Result<OpenAiCompatibleResponse> {
        let response = self
            .client
            .post(endpoint_url)
            .bearer_auth(api_key)
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json::<OpenAiCompatibleResponse>().await?)
    }
}

#[derive(Debug, Clone)]
struct FixtureOpenAiCompatibleBackend {
    response: OpenAiCompatibleResponse,
}

impl FixtureOpenAiCompatibleBackend {
    fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let response = serde_json::from_str(&fs::read_to_string(path.as_ref())?)?;
        Ok(Self { response })
    }
}

#[async_trait]
impl OpenAiCompatibleBackend for FixtureOpenAiCompatibleBackend {
    async fn execute(
        &self,
        _endpoint_url: &str,
        _api_key: &str,
        _request: OpenAiCompatibleRequest,
    ) -> Result<OpenAiCompatibleResponse> {
        Ok(self.response.clone())
    }
}

pub struct OpenAiCompatibleSearcherProvider {
    request_template: OpenAiCompatibleRequestTemplate,
    provider_label: String,
    backend: Arc<dyn OpenAiCompatibleBackend>,
}

impl std::fmt::Debug for OpenAiCompatibleSearcherProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiCompatibleSearcherProvider")
            .field("request_template", &self.request_template)
            .field("provider_label", &self.provider_label)
            .finish_non_exhaustive()
    }
}

impl Clone for OpenAiCompatibleSearcherProvider {
    fn clone(&self) -> Self {
        Self {
            request_template: self.request_template.clone(),
            provider_label: self.provider_label.clone(),
            backend: Arc::clone(&self.backend),
        }
    }
}

impl OpenAiCompatibleSearcherProvider {
    pub fn from_config(config: &SearcherConfig) -> Result<Self> {
        Self::from_config_with_base_dir(config, Path::new("."))
    }

    pub fn from_config_with_base_dir(
        config: &SearcherConfig,
        config_base_dir: &Path,
    ) -> Result<Self> {
        let request_template = OpenAiCompatibleRequestTemplate::from_config(config)?;
        let backend: Arc<dyn OpenAiCompatibleBackend> = if let Some(path) =
            resolve_searcher_path(config, config_base_dir, "fixture_response_path")?
        {
            Arc::new(FixtureOpenAiCompatibleBackend::from_path(path)?)
        } else {
            Arc::new(ReqwestOpenAiCompatibleBackend::new()?)
        };

        Ok(Self::from_request_template_with_backend(
            request_template,
            backend,
        ))
    }

    fn from_request_template_with_backend(
        request_template: OpenAiCompatibleRequestTemplate,
        backend: Arc<dyn OpenAiCompatibleBackend>,
    ) -> Self {
        let provider_label = format!(
            "openai-compatible:{}@{}",
            request_template.model(),
            request_template.endpoint_url()
        );

        Self {
            request_template,
            provider_label,
            backend,
        }
    }

    fn extract_answers(&self, response: &OpenAiCompatibleResponse) -> Vec<String> {
        response
            .first_message_content()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect()
    }
}

#[async_trait]
impl SearcherProvider for OpenAiCompatibleSearcherProvider {
    async fn search(&self, query: &AnswerQuery) -> Result<Vec<AnswerCandidate>> {
        let request = self.request_template.build_request(query);
        let response = self
            .backend
            .execute(
                self.request_template.endpoint_url(),
                self.request_template.api_key(),
                request,
            )
            .await?;

        Ok(self
            .extract_answers(&response)
            .into_iter()
            .map(|answer| AnswerCandidate {
                provider: self.provider_label.clone(),
                confidence: None,
                answer,
            })
            .collect())
    }
}

fn render_http_option_text(query: &AnswerQuery) -> String {
    query
        .options
        .iter()
        .map(render_option_search_value)
        .collect::<Vec<_>>()
        .join("#")
}

fn validate_http_searcher_url(url: &str, config_kind: &str) -> Result<()> {
    let parsed = Url::parse(url).map_err(|error| {
        CpassError::Config(format!(
            "searcher type '{config_kind}' field 'url' must be a valid absolute URL: {error}"
        ))
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        scheme => Err(CpassError::Config(format!(
            "searcher type '{config_kind}' field 'url' must use http or https, got '{scheme}'"
        ))),
    }
}

fn build_http_searcher_header_map(
    headers: &BTreeMap<String, String>,
    config_kind: &str,
) -> Result<HeaderMap> {
    let mut header_map = HeaderMap::new();

    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
            CpassError::Config(format!(
                "searcher type '{config_kind}' field 'headers' contains invalid header name '{name}': {error}"
            ))
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            CpassError::Config(format!(
                "searcher type '{config_kind}' field 'headers' contains invalid header value for '{name}': {error}"
            ))
        })?;
        header_map.insert(header_name, header_value);
    }

    Ok(header_map)
}

fn parse_http_searcher_response_path(
    raw_path: &str,
    config_kind: &str,
) -> Result<Vec<HttpSearcherResponsePathSegment>> {
    let path = raw_path.trim();
    if path.is_empty() {
        return Err(CpassError::Config(format!(
            "searcher type '{config_kind}' field 'a_field' must not be empty"
        )));
    }

    let chars = path.chars().collect::<Vec<_>>();
    let mut index = usize::from(chars.first() == Some(&'$'));
    let mut segments = Vec::new();

    while index < chars.len() {
        match chars[index] {
            '.' => {
                index += 1;
                let start = index;
                while index < chars.len() && !matches!(chars[index], '.' | '[' | ']') {
                    index += 1;
                }
                if start == index {
                    return Err(CpassError::Config(format!(
                        "searcher type '{config_kind}' field 'a_field' contains an empty key segment: '{path}'"
                    )));
                }
                segments.push(HttpSearcherResponsePathSegment::Key(
                    chars[start..index].iter().collect(),
                ));
            }
            '[' => {
                index += 1;
                let start = index;
                while index < chars.len() && chars[index].is_ascii_digit() {
                    index += 1;
                }
                if start == index || chars.get(index) != Some(&']') {
                    return Err(CpassError::Config(format!(
                        "searcher type '{config_kind}' field 'a_field' only supports numeric array indexes like '[0]': '{path}'"
                    )));
                }
                let raw_index = chars[start..index].iter().collect::<String>();
                let parsed_index = raw_index.parse::<usize>().map_err(|error| {
                    CpassError::Config(format!(
                        "searcher type '{config_kind}' field 'a_field' contains invalid array index '{raw_index}': {error}"
                    ))
                })?;
                segments.push(HttpSearcherResponsePathSegment::Index(parsed_index));
                index += 1;
            }
            ']' => {
                return Err(CpassError::Config(format!(
                    "searcher type '{config_kind}' field 'a_field' contains an unexpected ']': '{path}'"
                )));
            }
            _ => {
                let start = index;
                while index < chars.len() && !matches!(chars[index], '.' | '[' | ']') {
                    index += 1;
                }
                segments.push(HttpSearcherResponsePathSegment::Key(
                    chars[start..index].iter().collect(),
                ));
            }
        }
    }

    Ok(segments)
}

fn resolve_http_searcher_response_value<'a>(
    value: &'a serde_json::Value,
    path: &[HttpSearcherResponsePathSegment],
) -> Option<&'a serde_json::Value> {
    let mut current = value;

    for segment in path {
        current = match segment {
            HttpSearcherResponsePathSegment::Key(key) => current.get(key)?,
            HttpSearcherResponsePathSegment::Index(index) => current.get(*index)?,
        };
    }

    Some(current)
}

fn collect_http_searcher_answer_strings(value: &serde_json::Value, answers: &mut Vec<String>) {
    match value {
        serde_json::Value::String(answer) => {
            let trimmed = answer.trim();
            if !trimmed.is_empty() {
                answers.push(trimmed.to_owned());
            }
        }
        serde_json::Value::Number(number) => answers.push(number.to_string()),
        serde_json::Value::Bool(boolean) => answers.push(boolean.to_string()),
        serde_json::Value::Array(values) => {
            for entry in values {
                collect_http_searcher_answer_strings(entry, answers);
            }
        }
        serde_json::Value::Null | serde_json::Value::Object(_) => {}
    }
}

fn normalize_openai_compatible_endpoint_url(base_url: &str, config_kind: &str) -> Result<String> {
    let mut parsed = Url::parse(base_url).map_err(|error| {
        CpassError::Config(format!(
            "searcher type '{config_kind}' field 'base_url' must be a valid absolute URL: {error}"
        ))
    })?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(CpassError::Config(format!(
                "searcher type '{config_kind}' field 'base_url' must use http or https, got '{scheme}'"
            )));
        }
    }

    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(CpassError::Config(format!(
            "searcher type '{config_kind}' field 'base_url' must not include query or fragment components"
        )));
    }

    {
        let mut path_segments = parsed.path_segments_mut().map_err(|_| {
            CpassError::Config(format!(
                "searcher type '{config_kind}' field 'base_url' must be a hierarchical URL"
            ))
        })?;
        path_segments.pop_if_empty();
    }

    let segments = parsed
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .ok_or_else(|| {
            CpassError::Config(format!(
                "searcher type '{config_kind}' field 'base_url' must be a hierarchical URL"
            ))
        })?;

    if !segments.ends_with(&["chat", "completions"]) {
        let mut path_segments = parsed.path_segments_mut().map_err(|_| {
            CpassError::Config(format!(
                "searcher type '{config_kind}' field 'base_url' must be a hierarchical URL"
            ))
        })?;
        path_segments.push("chat");
        path_segments.push("completions");
    }

    Ok(parsed.to_string())
}

fn resolve_openai_compatible_prompt_template(config: &SearcherConfig) -> Result<String> {
    let legacy_prompt = optional_searcher_string_field(config, "prompt")?;
    let prompt_template = optional_searcher_string_field(config, "prompt_template")?;

    match (legacy_prompt, prompt_template) {
        (Some(_), Some(_)) => Err(CpassError::Config(format!(
            "searcher type '{}' must set only one of 'prompt' or 'prompt_template'",
            config.kind
        ))),
        (Some(prompt), None) | (None, Some(prompt)) => Ok(prompt),
        (None, None) => Ok(DEFAULT_OPENAI_COMPATIBLE_PROMPT_TEMPLATE.to_owned()),
    }
}

fn render_openai_compatible_prompt(prompt_template: &str, query: &AnswerQuery) -> String {
    prompt_template
        .replace("{type}", &query.question_type_label)
        .replace("{value}", &query.prompt)
        .replace("{question}", &query.prompt)
        .replace("{options}", &render_openai_compatible_options_text(query))
        .replace("{blanks}", &render_openai_compatible_blanks_text(query))
        .replace("{search_text}", &query.render_search_text())
}

fn render_openai_compatible_options_text(query: &AnswerQuery) -> String {
    if query.options.is_empty() {
        return String::new();
    }

    if query
        .options
        .iter()
        .all(|option| option.rich_content.is_none())
    {
        let mut rendered = String::from("选项：\n");
        for option in &query.options {
            rendered.push_str(&option.key);
            rendered.push_str(". ");
            rendered.push_str(&option.value);
            rendered.push(';');
        }
        return rendered;
    }

    let mut rendered = String::from("选项：\n");
    for option in &query.options {
        rendered.push_str(&render_option_search_line(option));
        rendered.push('\n');
    }
    rendered.pop();
    rendered
}

fn render_openai_compatible_blanks_text(query: &AnswerQuery) -> String {
    query
        .blanks
        .iter()
        .enumerate()
        .map(|(index, blank)| format!("Blank {}: {}", index + 1, blank))
        .collect::<Vec<_>>()
        .join("\n")
}

fn required_searcher_string_field(config: &SearcherConfig, field_name: &str) -> Result<String> {
    let Some(value) = config
        .values
        .get(field_name)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
    else {
        return Err(CpassError::Config(format!(
            "searcher type '{}' requires string field '{}'",
            config.kind, field_name
        )));
    };
    if value.is_empty() {
        return Err(CpassError::Config(format!(
            "searcher type '{}' field '{}' must not be empty",
            config.kind, field_name
        )));
    }
    Ok(value.to_owned())
}

fn optional_searcher_string_field(
    config: &SearcherConfig,
    field_name: &str,
) -> Result<Option<String>> {
    match config.values.get(field_name) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value.to_owned()))
            }
        }
        Some(_) => Err(CpassError::Config(format!(
            "searcher type '{}' field '{}' must be a string when present",
            config.kind, field_name
        ))),
    }
}

fn resolve_searcher_path(
    config: &SearcherConfig,
    config_base_dir: &Path,
    field_name: &str,
) -> Result<Option<PathBuf>> {
    let Some(raw_path) = optional_searcher_string_field(config, field_name)? else {
        return Ok(None);
    };

    let path = PathBuf::from(raw_path);
    Ok(Some(if path.is_absolute() {
        path
    } else {
        config_base_dir.join(path)
    }))
}

fn searcher_string_map_field(
    config: &SearcherConfig,
    field_name: &str,
) -> Result<BTreeMap<String, String>> {
    match config.values.get(field_name) {
        None | Some(serde_json::Value::Null) => Ok(BTreeMap::new()),
        Some(serde_json::Value::Object(entries)) => {
            let mut values = BTreeMap::new();
            for (key, value) in entries {
                let Some(value) = value.as_str() else {
                    return Err(CpassError::Config(format!(
                        "searcher type '{}' field '{}' must contain only string values",
                        config.kind, field_name
                    )));
                };
                values.insert(key.clone(), value.to_owned());
            }
            Ok(values)
        }
        Some(_) => Err(CpassError::Config(format!(
            "searcher type '{}' field '{}' must be an object of string values",
            config.kind, field_name
        ))),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum JsonSearcherValue {
    Answer(String),
    Answers(Vec<String>),
}

impl JsonSearcherValue {
    fn into_answers(self) -> Vec<String> {
        match self {
            Self::Answer(answer) => vec![answer],
            Self::Answers(answers) => answers,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JsonSearcherProvider {
    path: PathBuf,
    answers_by_key: HashMap<String, Vec<String>>,
}

impl JsonSearcherProvider {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let raw: HashMap<String, JsonSearcherValue> =
            serde_json::from_str(&fs::read_to_string(&path)?)?;
        let mut answers_by_key = HashMap::new();

        for (key, value) in raw {
            let normalized_key = normalize_search_key(&key);
            if normalized_key.is_empty() {
                continue;
            }

            let entry = answers_by_key
                .entry(normalized_key)
                .or_insert_with(Vec::new);
            entry.extend(
                value
                    .into_answers()
                    .into_iter()
                    .map(|answer| answer.trim().to_owned())
                    .filter(|answer| !answer.is_empty()),
            );
        }

        Ok(Self {
            path,
            answers_by_key,
        })
    }

    pub fn from_config(config: &SearcherConfig, config_base_dir: &Path) -> Result<Self> {
        let file_path = config
            .values
            .get("file_path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                CpassError::Config(format!(
                    "searcher type '{}' requires string field 'file_path'",
                    config.kind
                ))
            })?;
        let path = PathBuf::from(file_path);
        let resolved_path = if path.is_absolute() {
            path
        } else {
            config_base_dir.join(path)
        };

        Self::from_path(resolved_path)
    }
}

fn normalize_search_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '(' | ')' | '（' | '）' | '.' | '。' | '?' | '？' | '!' | '！'
            )
        })
        .to_lowercase()
}

#[async_trait]
impl SearcherProvider for JsonSearcherProvider {
    async fn search(&self, query: &AnswerQuery) -> Result<Vec<AnswerCandidate>> {
        let prompt_key = normalize_search_key(&query.prompt);
        let rendered_key = normalize_search_key(&query.render_search_text());
        let answers = self
            .answers_by_key
            .get(&prompt_key)
            .or_else(|| self.answers_by_key.get(&rendered_key));

        Ok(answers
            .into_iter()
            .flatten()
            .map(|answer| AnswerCandidate {
                provider: format!("json:{}", self.path.display()),
                confidence: None,
                answer: answer.clone(),
            })
            .collect())
    }
}

type SqliteDbHandle = c_void;
type SqliteStatementHandle = c_void;

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const SQLITE_OPEN_READONLY: c_int = 0x0000_0001;
#[cfg(test)]
const SQLITE_OPEN_READWRITE: c_int = 0x0000_0002;
#[cfg(test)]
const SQLITE_OPEN_CREATE: c_int = 0x0000_0004;

#[link(name = "sqlite3")]
unsafe extern "C" {
    fn sqlite3_open_v2(
        filename: *const c_char,
        db: *mut *mut SqliteDbHandle,
        flags: c_int,
        vfs: *const c_char,
    ) -> c_int;
    fn sqlite3_close(db: *mut SqliteDbHandle) -> c_int;
    fn sqlite3_prepare_v2(
        db: *mut SqliteDbHandle,
        sql: *const c_char,
        nbytes: c_int,
        statement: *mut *mut SqliteStatementHandle,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_bind_text(
        statement: *mut SqliteStatementHandle,
        index: c_int,
        value: *const c_char,
        nbytes: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_step(statement: *mut SqliteStatementHandle) -> c_int;
    fn sqlite3_column_text(statement: *mut SqliteStatementHandle, column: c_int) -> *const u8;
    fn sqlite3_finalize(statement: *mut SqliteStatementHandle) -> c_int;
    #[cfg(test)]
    fn sqlite3_exec(
        db: *mut SqliteDbHandle,
        sql: *const c_char,
        callback: Option<
            unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int,
        >,
        context: *mut c_void,
        errmsg: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_errmsg(db: *mut SqliteDbHandle) -> *const c_char;
    #[cfg(test)]
    fn sqlite3_free(value: *mut c_void);
}

#[derive(Debug)]
struct SqliteConnection {
    raw: *mut SqliteDbHandle,
}

impl SqliteConnection {
    fn open_read_only(path: &Path) -> Result<Self> {
        Self::open_with_flags(path, SQLITE_OPEN_READONLY)
    }

    #[cfg(test)]
    fn open_read_write_create(path: &Path) -> Result<Self> {
        Self::open_with_flags(path, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE)
    }

    fn open_with_flags(path: &Path, flags: c_int) -> Result<Self> {
        let filename = make_c_string(
            path.to_string_lossy().as_ref(),
            &format!("sqlite database path '{}'", path.display()),
        )?;
        let mut raw = ptr::null_mut();
        let code = unsafe { sqlite3_open_v2(filename.as_ptr(), &mut raw, flags, ptr::null()) };

        if code != SQLITE_OK || raw.is_null() {
            let message = if raw.is_null() {
                format!("sqlite open returned code {code}")
            } else {
                sqlite_error_message(raw)
            };
            if !raw.is_null() {
                unsafe {
                    sqlite3_close(raw);
                }
            }
            return Err(CpassError::Config(format!(
                "sqlite searcher failed to open '{}': {message}",
                path.display()
            )));
        }

        Ok(Self { raw })
    }

    fn prepare(&self, sql: &str) -> Result<SqliteStatement> {
        let sql = make_c_string(sql, "sqlite statement")?;
        let mut raw = ptr::null_mut();
        let code =
            unsafe { sqlite3_prepare_v2(self.raw, sql.as_ptr(), -1, &mut raw, ptr::null_mut()) };
        if code != SQLITE_OK || raw.is_null() {
            return Err(CpassError::Config(format!(
                "sqlite searcher failed to prepare statement: {}",
                sqlite_error_message(self.raw)
            )));
        }

        Ok(SqliteStatement { db: self.raw, raw })
    }

    fn query_text_values(&self, sql: &str, parameter: &str) -> Result<Vec<String>> {
        let statement = self.prepare(sql)?;
        statement.bind_text(1, parameter)?;

        let mut values = Vec::new();
        while let SqliteStep::Row = statement.step()? {
            if let Some(value) = statement.column_text(0) {
                let value = value.trim();
                if !value.is_empty() {
                    values.push(value.to_owned());
                }
            }
        }

        Ok(values)
    }

    #[cfg(test)]
    fn execute_batch(&self, sql: &str) -> Result<()> {
        let sql = make_c_string(sql, "sqlite batch SQL")?;
        let mut error_message = ptr::null_mut();
        let code = unsafe {
            sqlite3_exec(
                self.raw,
                sql.as_ptr(),
                None,
                ptr::null_mut(),
                &mut error_message,
            )
        };
        if code != SQLITE_OK {
            let message = sqlite_exec_error_message(error_message);
            return Err(CpassError::Config(format!(
                "sqlite searcher failed to execute batch SQL: {message}"
            )));
        }

        Ok(())
    }
}

impl Drop for SqliteConnection {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                sqlite3_close(self.raw);
            }
        }
    }
}

enum SqliteStep {
    Row,
    Done,
}

#[derive(Debug)]
struct SqliteStatement {
    db: *mut SqliteDbHandle,
    raw: *mut SqliteStatementHandle,
}

impl SqliteStatement {
    fn bind_text(&self, index: c_int, value: &str) -> Result<()> {
        let value = make_c_string(value, "sqlite bound text")?;
        let code =
            unsafe { sqlite3_bind_text(self.raw, index, value.as_ptr(), -1, sqlite_transient()) };
        if code != SQLITE_OK {
            return Err(CpassError::Config(format!(
                "sqlite searcher failed to bind query text: {}",
                sqlite_error_message(self.db)
            )));
        }

        Ok(())
    }

    fn step(&self) -> Result<SqliteStep> {
        match unsafe { sqlite3_step(self.raw) } {
            SQLITE_ROW => Ok(SqliteStep::Row),
            SQLITE_DONE => Ok(SqliteStep::Done),
            _ => Err(CpassError::Config(format!(
                "sqlite searcher query failed: {}",
                sqlite_error_message(self.db)
            ))),
        }
    }

    fn column_text(&self, column: c_int) -> Option<String> {
        let text = unsafe { sqlite3_column_text(self.raw, column) };
        if text.is_null() {
            return None;
        }

        Some(
            unsafe { CStr::from_ptr(text.cast::<c_char>()) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

unsafe fn sqlite_transient() -> Option<unsafe extern "C" fn(*mut c_void)> {
    unsafe { std::mem::transmute::<isize, Option<unsafe extern "C" fn(*mut c_void)>>(-1) }
}

impl Drop for SqliteStatement {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                sqlite3_finalize(self.raw);
            }
        }
    }
}

fn make_c_string(value: &str, label: &str) -> Result<CString> {
    CString::new(value).map_err(|_| {
        CpassError::Config(format!(
            "{label} contains an unsupported NUL byte for the sqlite searcher"
        ))
    })
}

fn sqlite_error_message(db: *mut SqliteDbHandle) -> String {
    let message = unsafe { sqlite3_errmsg(db) };
    if message.is_null() {
        "unknown sqlite error".to_owned()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
fn sqlite_exec_error_message(error_message: *mut c_char) -> String {
    if error_message.is_null() {
        return "unknown sqlite exec error".to_owned();
    }

    let message = unsafe { CStr::from_ptr(error_message) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        sqlite3_free(error_message.cast::<c_void>());
    }
    message
}

fn validate_sqlite_identifier(value: &str, field_name: &str) -> Result<String> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(CpassError::Config(format!(
            "sqlite searcher field '{field_name}' must not be empty"
        )));
    };

    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(CpassError::Config(format!(
            "sqlite searcher field '{field_name}' must use only ASCII letters, digits, or underscores"
        )));
    }

    Ok(value.to_owned())
}

#[derive(Debug, Clone)]
pub struct SqliteSearcherProvider {
    path: PathBuf,
    table: String,
    req_field: String,
    rsp_field: String,
}

impl SqliteSearcherProvider {
    pub fn from_config(config: &SearcherConfig, config_base_dir: &Path) -> Result<Self> {
        let file_path = config
            .values
            .get("file_path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                CpassError::Config(format!(
                    "searcher type '{}' requires string field 'file_path'",
                    config.kind
                ))
            })?;
        let path = PathBuf::from(file_path);
        let resolved_path = if path.is_absolute() {
            path
        } else {
            config_base_dir.join(path)
        };
        let table = config
            .values
            .get("table")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("question");
        let req_field = config
            .values
            .get("req_field")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("question");
        let rsp_field = config
            .values
            .get("rsp_field")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("answer");

        let provider = Self {
            path: resolved_path,
            table: validate_sqlite_identifier(table, "table")?,
            req_field: validate_sqlite_identifier(req_field, "req_field")?,
            rsp_field: validate_sqlite_identifier(rsp_field, "rsp_field")?,
        };
        provider.validate_database()?;
        Ok(provider)
    }

    fn validate_database(&self) -> Result<()> {
        let connection = SqliteConnection::open_read_only(&self.path)?;
        let _statement = connection.prepare(&self.select_statement())?;
        Ok(())
    }

    fn select_statement(&self) -> String {
        format!(
            "SELECT {} FROM {} WHERE {} = (?)",
            self.rsp_field, self.table, self.req_field
        )
    }

    fn query_answers(&self, search_key: &str) -> Result<Vec<String>> {
        let connection = SqliteConnection::open_read_only(&self.path)?;
        connection.query_text_values(&self.select_statement(), search_key)
    }
}

#[async_trait]
impl SearcherProvider for SqliteSearcherProvider {
    async fn search(&self, query: &AnswerQuery) -> Result<Vec<AnswerCandidate>> {
        let mut answers = Vec::new();
        let mut seen = HashSet::new();
        let search_keys = [query.prompt.clone(), query.render_search_text()];

        for search_key in search_keys {
            if !seen.insert(search_key.clone()) {
                continue;
            }

            for answer in self.query_answers(&search_key)? {
                answers.push(AnswerCandidate {
                    provider: format!("sqlite:{}", self.path.display()),
                    confidence: None,
                    answer,
                });
            }
        }

        Ok(answers)
    }
}

#[derive(Default)]
pub struct SearcherPipeline {
    providers: Vec<Box<dyn SearcherProvider>>,
}

impl SearcherPipeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_provider<P>(&mut self, provider: P)
    where
        P: SearcherProvider + 'static,
    {
        self.providers.push(Box::new(provider));
    }

    #[must_use]
    pub fn with_provider<P>(mut self, provider: P) -> Self
    where
        P: SearcherProvider + 'static,
    {
        self.add_provider(provider);
        self
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub async fn select_candidates(&self, query: &AnswerQuery) -> Result<AnswerCandidateSelection> {
        let candidates = self.search(query).await?;
        Ok(AnswerCandidateSelection::new(query.clone(), candidates))
    }

    pub async fn search(&self, query: &AnswerQuery) -> Result<Vec<AnswerCandidate>> {
        let mut candidates = Vec::new();

        for provider in &self.providers {
            candidates.extend(provider.search(query).await?);
        }

        Ok(candidates)
    }

    pub async fn select_chapter_work_candidates(
        &self,
        batch: &ChapterWorkQueryBatch,
    ) -> Result<ChapterWorkCandidateSelectionBatch> {
        let mut selections = Vec::with_capacity(batch.queries.len());

        for query in &batch.queries {
            selections.push(self.select_candidates(query).await?);
        }

        Ok(ChapterWorkCandidateSelectionBatch {
            title: batch.title.clone(),
            work_answer_id: batch.work_answer_id,
            work_relation_id: batch.work_relation_id,
            total_question_num: batch.total_question_num,
            selections,
        })
    }
}

pub fn build_searcher_pipeline(
    configs: &[SearcherConfig],
    config_base_dir: &Path,
) -> Result<Option<SearcherPipeline>> {
    if configs.is_empty() {
        return Ok(None);
    }

    let mut pipeline = SearcherPipeline::new();

    for config in configs {
        if config.kind.eq_ignore_ascii_case("json")
            || config.kind.eq_ignore_ascii_case("jsonfilesearcher")
        {
            pipeline.add_provider(JsonSearcherProvider::from_config(config, config_base_dir)?);
        } else if config.kind.eq_ignore_ascii_case("sqlite")
            || config.kind.eq_ignore_ascii_case("sqlitesearcher")
        {
            pipeline.add_provider(SqliteSearcherProvider::from_config(
                config,
                config_base_dir,
            )?);
        } else if config.kind.eq_ignore_ascii_case("http")
            || config.kind.eq_ignore_ascii_case("restapisearcher")
            || config.kind.eq_ignore_ascii_case("jsonapisearcher")
        {
            pipeline.add_provider(HttpSearcherProvider::from_config_with_base_dir(
                config,
                config_base_dir,
            )?);
        } else if config.kind.eq_ignore_ascii_case("openai-compatible")
            || config.kind.eq_ignore_ascii_case("openaisearcher")
            || config.kind.eq_ignore_ascii_case("openai")
        {
            pipeline.add_provider(OpenAiCompatibleSearcherProvider::from_config_with_base_dir(
                config,
                config_base_dir,
            )?);
        } else {
            return Err(CpassError::Config(format!(
                "searcher type '{}' is not supported in Rust yet; currently supported: json, sqlite, http, openai-compatible",
                config.kind
            )));
        }
    }

    Ok(Some(pipeline))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        AnswerCandidate, AnswerQuery, AnswerQuerySource, AnswerQuestionKind,
        ChapterWorkCandidateSelectionBatch, ChapterWorkQueryBatch, HttpSearcherBackend,
        HttpSearcherMethod, HttpSearcherPayloadMode, HttpSearcherProvider, HttpSearcherRequest,
        HttpSearcherRequestPayload, HttpSearcherRequestTemplate, JsonSearcherProvider,
        OpenAiCompatibleBackend, OpenAiCompatibleMessage, OpenAiCompatibleMessageRole,
        OpenAiCompatibleRequest, OpenAiCompatibleRequestTemplate, OpenAiCompatibleResponse,
        OpenAiCompatibleResponseChoice, OpenAiCompatibleResponseMessage,
        OpenAiCompatibleSearcherProvider, SearcherPipeline, SearcherProvider, SqliteConnection,
        SqliteSearcherProvider, build_searcher_pipeline,
    };
    use crate::config::SearcherConfig;
    use crate::error::Result;
    use crate::models::{
        ChapterWorkFormSnapshot, ChapterWorkQuestionSummary, ExamQuestionOption,
        ExamQuestionSummary, QuestionRichContent,
    };
    use reqwest::header::HeaderMap;

    struct StubSearcherProvider {
        provider: &'static str,
        answers: Vec<&'static str>,
    }

    fn sample_answer_query() -> AnswerQuery {
        AnswerQuery {
            source: AnswerQuerySource::ChapterWork,
            question_index: 0,
            question_id: 700303,
            question_type: 1,
            question_type_label: "多选题".to_owned(),
            question_kind: AnswerQuestionKind::MultipleChoice,
            prompt: "哪些选项属于元音？".to_owned(),
            options: vec![
                ExamQuestionOption {
                    key: "A".to_owned(),
                    value: "a".to_owned(),
                    rich_content: None,
                },
                ExamQuestionOption {
                    key: "B".to_owned(),
                    value: "b".to_owned(),
                    rich_content: None,
                },
            ],
            blanks: vec!["忽略".to_owned()],
        }
    }

    fn sample_rich_option_answer_query() -> AnswerQuery {
        AnswerQuery {
            source: AnswerQuerySource::ChapterWork,
            question_index: 0,
            question_id: 700304,
            question_type: 0,
            question_type_label: "单选题".to_owned(),
            question_kind: AnswerQuestionKind::SingleChoice,
            prompt: "请选择图文选项".to_owned(),
            options: vec![
                ExamQuestionOption {
                    key: "A".to_owned(),
                    value: "图文 选项".to_owned(),
                    rich_content: Some(QuestionRichContent {
                        source_html:
                            "<span>图文 <strong>选项</strong><img src=\"https://static.example/a.png\" /></span>"
                                .to_owned(),
                        image_urls: vec!["https://static.example/a.png".to_owned()],
                    }),
                },
                ExamQuestionOption {
                    key: "B".to_owned(),
                    value: String::new(),
                    rich_content: Some(QuestionRichContent {
                        source_html: "<img src=\"https://static.example/b.png\" />".to_owned(),
                        image_urls: vec!["https://static.example/b.png".to_owned()],
                    }),
                },
            ],
            blanks: Vec::new(),
        }
    }

    fn json_searcher_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/legacy/json_searcher_questions.json")
    }

    struct TempSqliteFixture {
        path: PathBuf,
    }

    struct TempHttpResponseFixture {
        path: PathBuf,
    }

    #[derive(Debug)]
    struct RecordedHttpRequest {
        request: HttpSearcherRequest,
        headers: HeaderMap,
    }

    #[derive(Debug)]
    struct RecordedOpenAiCompatibleRequest {
        endpoint_url: String,
        api_key: String,
        request: OpenAiCompatibleRequest,
    }

    #[derive(Debug)]
    struct RecordingHttpSearcherBackend {
        response: serde_json::Value,
        requests: Mutex<Vec<RecordedHttpRequest>>,
    }

    #[derive(Debug)]
    struct RecordingOpenAiCompatibleBackend {
        response: OpenAiCompatibleResponse,
        requests: Mutex<Vec<RecordedOpenAiCompatibleRequest>>,
    }

    impl TempSqliteFixture {
        fn new(file_name: &str, table: &str, req_field: &str, rsp_field: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "cpass-searcher-{file_name}-{}-{unique}.db",
                std::process::id()
            ));
            let connection =
                SqliteConnection::open_read_write_create(&path).expect("sqlite fixture db");
            let create_table = format!(
                "CREATE TABLE {table} ({req_field} TEXT NOT NULL, {rsp_field} TEXT NOT NULL);"
            );
            let rendered_fill_blank =
                "[填空题 / fill_blank] 请补全“词汇”相关术语。\nBlank 1: 词义：\nBlank 2: 词性：";
            let insert_rows = [
                ("普通话以哪种方言为基础方言？", "B"),
                (rendered_fill_blank, "语义内容"),
                (rendered_fill_blank, "名词"),
            ];
            let mut sql = String::new();
            sql.push_str(&create_table);
            for (question, answer) in insert_rows {
                sql.push_str(&format!(
                    "INSERT INTO {table} ({req_field}, {rsp_field}) VALUES ({}, {});",
                    sql_string(question),
                    sql_string(answer)
                ));
            }
            connection
                .execute_batch(&sql)
                .expect("sqlite fixture seeded");
            drop(connection);

            Self { path }
        }
    }

    impl TempHttpResponseFixture {
        fn new(file_name: &str, response: &serde_json::Value) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "cpass-searcher-{file_name}-{}-{unique}.json",
                std::process::id()
            ));
            fs::write(
                &path,
                serde_json::to_string_pretty(response).expect("fixture response to serialize"),
            )
            .expect("response fixture written");
            Self { path }
        }
    }

    impl RecordingHttpSearcherBackend {
        fn new(response: serde_json::Value) -> Arc<Self> {
            Arc::new(Self {
                response,
                requests: Mutex::new(Vec::new()),
            })
        }

        fn single_request(&self) -> RecordedHttpRequest {
            let mut requests = self.requests.lock().expect("recorded requests");
            assert_eq!(requests.len(), 1, "expected exactly one recorded request");
            requests.remove(0)
        }
    }

    impl RecordingOpenAiCompatibleBackend {
        fn new(response: OpenAiCompatibleResponse) -> Arc<Self> {
            Arc::new(Self {
                response,
                requests: Mutex::new(Vec::new()),
            })
        }

        fn single_request(&self) -> RecordedOpenAiCompatibleRequest {
            let mut requests = self.requests.lock().expect("recorded requests");
            assert_eq!(requests.len(), 1, "expected exactly one recorded request");
            requests.remove(0)
        }
    }

    #[async_trait::async_trait]
    impl HttpSearcherBackend for RecordingHttpSearcherBackend {
        async fn execute(
            &self,
            request: HttpSearcherRequest,
            headers: &HeaderMap,
        ) -> Result<serde_json::Value> {
            self.requests
                .lock()
                .expect("recorded requests")
                .push(RecordedHttpRequest {
                    request,
                    headers: headers.clone(),
                });
            Ok(self.response.clone())
        }
    }

    #[async_trait::async_trait]
    impl OpenAiCompatibleBackend for RecordingOpenAiCompatibleBackend {
        async fn execute(
            &self,
            endpoint_url: &str,
            api_key: &str,
            request: OpenAiCompatibleRequest,
        ) -> Result<OpenAiCompatibleResponse> {
            self.requests.lock().expect("recorded requests").push(
                RecordedOpenAiCompatibleRequest {
                    endpoint_url: endpoint_url.to_owned(),
                    api_key: api_key.to_owned(),
                    request,
                },
            );
            Ok(self.response.clone())
        }
    }

    impl Drop for TempSqliteFixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn sql_string(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    fn summary_question_kind(question_type: u64) -> String {
        AnswerQuestionKind::from_question_type(question_type)
            .as_str()
            .to_owned()
    }

    #[async_trait::async_trait]
    impl SearcherProvider for StubSearcherProvider {
        async fn search(&self, query: &AnswerQuery) -> Result<Vec<AnswerCandidate>> {
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

    #[test]
    fn builds_answer_query_from_exam_preview_question() {
        let question = ExamQuestionSummary {
            question_index: 2,
            question_id: 700101,
            question_type: 0,
            question_type_label: "单选题".to_owned(),
            question_kind: summary_question_kind(0),
            prompt: "普通话以哪种方言为基础方言？".to_owned(),
            options: vec![
                ExamQuestionOption {
                    key: "A".to_owned(),
                    value: "吴方言".to_owned(),
                    rich_content: None,
                },
                ExamQuestionOption {
                    key: "B".to_owned(),
                    value: "北方方言".to_owned(),
                    rich_content: None,
                },
            ],
            blanks: Vec::new(),
        };

        let query = AnswerQuery::from(&question);

        assert_eq!(query.source, AnswerQuerySource::ExamPreview);
        assert_eq!(query.question_index, 2);
        assert_eq!(query.question_id, 700101);
        assert_eq!(query.question_type, 0);
        assert_eq!(query.question_kind, AnswerQuestionKind::SingleChoice);
        assert_eq!(query.prompt, "普通话以哪种方言为基础方言？");
        assert_eq!(query.options.len(), 2);
        assert!(query.blanks.is_empty());
    }

    #[test]
    fn builds_answer_query_from_chapter_work_question() {
        let question = ChapterWorkQuestionSummary {
            question_index: 1,
            question_id: 700202,
            question_type: 2,
            question_type_label: "填空题".to_owned(),
            question_kind: summary_question_kind(2),
            prompt: "请依次填写词语。".to_owned(),
            options: Vec::new(),
            blanks: vec!["第1空".to_owned(), "第2空".to_owned()],
        };

        let query = AnswerQuery::from(&question);

        assert_eq!(query.source, AnswerQuerySource::ChapterWork);
        assert_eq!(query.question_index, 1);
        assert_eq!(query.question_id, 700202);
        assert_eq!(query.question_type, 2);
        assert_eq!(query.question_kind, AnswerQuestionKind::FillBlank);
        assert!(query.options.is_empty());
        assert_eq!(query.blanks, vec!["第1空", "第2空"]);
    }

    #[test]
    fn normalizes_extended_legacy_question_types() {
        let cases = [
            (4, AnswerQuestionKind::ShortAnswer, "short_answer"),
            (5, AnswerQuestionKind::TermExplanation, "term_explanation"),
            (6, AnswerQuestionKind::Essay, "essay"),
            (7, AnswerQuestionKind::Calculation, "calculation"),
            (8, AnswerQuestionKind::Other, "other"),
            (9, AnswerQuestionKind::JournalEntry, "journal_entry"),
            (10, AnswerQuestionKind::Material, "material"),
            (11, AnswerQuestionKind::Matching, "matching"),
            (13, AnswerQuestionKind::Ordering, "ordering"),
            (14, AnswerQuestionKind::Cloze, "cloze"),
            (
                15,
                AnswerQuestionKind::ReadingComprehension,
                "reading_comprehension",
            ),
            (18, AnswerQuestionKind::Spoken, "spoken"),
            (19, AnswerQuestionKind::Listening, "listening"),
            (20, AnswerQuestionKind::SharedOption, "shared_option"),
            (21, AnswerQuestionKind::Assessment, "assessment"),
        ];

        for (question_type, expected_kind, expected_label) in cases {
            let kind = AnswerQuestionKind::from_question_type(question_type);
            assert_eq!(kind, expected_kind);
            assert_eq!(kind.as_str(), expected_label);
        }

        assert_eq!(
            AnswerQuestionKind::from_question_type(99),
            AnswerQuestionKind::Unknown(99)
        );
        assert_eq!(
            AnswerQuestionKind::from_question_type(99).as_str(),
            "unknown"
        );
    }

    #[test]
    fn renders_search_text_with_option_and_blank_context() {
        let text = sample_answer_query().render_search_text();

        assert_eq!(
            text,
            "[多选题 / multiple_choice] 哪些选项属于元音？\nA. a\nB. b\nBlank 1: 忽略"
        );
    }

    #[test]
    fn renders_search_text_with_rich_option_metadata() {
        let text = sample_rich_option_answer_query().render_search_text();

        assert_eq!(
            text,
            "[单选题 / single_choice] 请选择图文选项\nA. 图文 选项 | rich_html=<span>图文 <strong>选项</strong><img src=\"https://static.example/a.png\" /></span> | image_urls=https://static.example/a.png\nB. [no visible option text] | rich_html=<img src=\"https://static.example/b.png\" /> | image_urls=https://static.example/b.png"
        );
    }

    #[test]
    fn builds_openai_compatible_request_template_with_defaults() {
        let config = SearcherConfig {
            kind: "openai-compatible".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("https://api.openai.com/v1".to_owned()),
                ),
                (
                    "model".to_owned(),
                    serde_json::Value::String("gpt-4.1-mini".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    serde_json::Value::String("sk-example".to_owned()),
                ),
            ]),
        };

        let template = OpenAiCompatibleRequestTemplate::from_config(&config)
            .expect("openai-compatible template");
        let request = template.build_request(&sample_answer_query());

        assert_eq!(
            template.endpoint_url(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(template.api_key(), "sk-example");
        assert_eq!(template.model(), "gpt-4.1-mini");
        assert_eq!(
            template.system_prompt(),
            "你是一位答题助手。请只输出答案本身，不要解释。"
        );
        assert_eq!(template.prompt_template(), "{search_text}");
        assert_eq!(
            request,
            OpenAiCompatibleRequest {
                model: "gpt-4.1-mini".to_owned(),
                messages: vec![
                    OpenAiCompatibleMessage {
                        role: OpenAiCompatibleMessageRole::System,
                        content: "你是一位答题助手。请只输出答案本身，不要解释。"
                            .to_owned(),
                    },
                    OpenAiCompatibleMessage {
                        role: OpenAiCompatibleMessageRole::User,
                        content:
                            "[多选题 / multiple_choice] 哪些选项属于元音？\nA. a\nB. b\nBlank 1: 忽略"
                                .to_owned(),
                    },
                ],
            }
        );
    }

    #[test]
    fn renders_openai_compatible_prompt_from_legacy_prompt_fields() {
        let config = SearcherConfig {
            kind: "OpenAISearcher".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("https://example.com/api".to_owned()),
                ),
                (
                    "model".to_owned(),
                    serde_json::Value::String("gpt-compatible".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    serde_json::Value::String("legacy-key".to_owned()),
                ),
                (
                    "system_prompt".to_owned(),
                    serde_json::Value::String("只回答答案".to_owned()),
                ),
                (
                    "prompt".to_owned(),
                    serde_json::Value::String(
                        "类型={type}\n题干={question}\n选项={options}\n空={blanks}\n全文={search_text}"
                            .to_owned(),
                    ),
                ),
            ]),
        };

        let template =
            OpenAiCompatibleRequestTemplate::from_config(&config).expect("legacy openai config");
        let request = template.build_request(&sample_answer_query());

        assert_eq!(
            template.endpoint_url(),
            "https://example.com/api/chat/completions"
        );
        assert_eq!(template.system_prompt(), "只回答答案");
        assert_eq!(
            request.messages[1].content,
            "类型=多选题\n题干=哪些选项属于元音？\n选项=选项：\nA. a;B. b;\n空=Blank 1: 忽略\n全文=[多选题 / multiple_choice] 哪些选项属于元音？\nA. a\nB. b\nBlank 1: 忽略"
        );
    }

    #[test]
    fn renders_openai_compatible_prompt_with_rich_option_metadata() {
        let config = SearcherConfig {
            kind: "OpenAISearcher".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("https://example.com/api".to_owned()),
                ),
                (
                    "model".to_owned(),
                    serde_json::Value::String("gpt-compatible".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    serde_json::Value::String("legacy-key".to_owned()),
                ),
                (
                    "prompt".to_owned(),
                    serde_json::Value::String("{options}".to_owned()),
                ),
            ]),
        };

        let template =
            OpenAiCompatibleRequestTemplate::from_config(&config).expect("legacy openai config");
        let request = template.build_request(&sample_rich_option_answer_query());

        assert_eq!(
            request.messages[1].content,
            "选项：\nA. 图文 选项 | rich_html=<span>图文 <strong>选项</strong><img src=\"https://static.example/a.png\" /></span> | image_urls=https://static.example/a.png\nB. [no visible option text] | rich_html=<img src=\"https://static.example/b.png\" /> | image_urls=https://static.example/b.png"
        );
    }

    #[test]
    fn parses_openai_compatible_response_content() {
        let response: OpenAiCompatibleResponse = serde_json::from_value(serde_json::json!({
            "choices": [
                {
                    "message": {
                        "content": "  A#C  "
                    }
                }
            ]
        }))
        .expect("openai-compatible response");

        assert_eq!(response.first_message_content(), Some("A#C"));
    }

    #[test]
    fn rejects_invalid_openai_compatible_base_urls() {
        let config = SearcherConfig {
            kind: "openai-compatible".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("ftp://example.com/v1".to_owned()),
                ),
                (
                    "model".to_owned(),
                    serde_json::Value::String("gpt-4.1-mini".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    serde_json::Value::String("sk-example".to_owned()),
                ),
            ]),
        };

        let error =
            OpenAiCompatibleRequestTemplate::from_config(&config).expect_err("invalid base URL");
        assert!(matches!(error, crate::error::CpassError::Config(_)));
        assert!(
            error
                .to_string()
                .contains("field 'base_url' must use http or https")
        );
    }

    #[tokio::test]
    async fn openai_compatible_searcher_issues_chat_completion_requests() {
        let config = SearcherConfig {
            kind: "OpenAISearcher".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("https://api.example.com/v1".to_owned()),
                ),
                (
                    "model".to_owned(),
                    serde_json::Value::String("gpt-compatible".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    serde_json::Value::String("sk-openai-compatible".to_owned()),
                ),
                (
                    "system_prompt".to_owned(),
                    serde_json::Value::String("只回答答案".to_owned()),
                ),
                (
                    "prompt_template".to_owned(),
                    serde_json::Value::String(
                        "题干：{question}\n选项：{options}\n全文：{search_text}".to_owned(),
                    ),
                ),
            ]),
        };
        let backend = RecordingOpenAiCompatibleBackend::new(OpenAiCompatibleResponse {
            choices: vec![OpenAiCompatibleResponseChoice {
                message: OpenAiCompatibleResponseMessage {
                    content: Some("  A#C  ".to_owned()),
                },
            }],
        });
        let searcher = OpenAiCompatibleSearcherProvider::from_request_template_with_backend(
            OpenAiCompatibleRequestTemplate::from_config(&config).expect("request template"),
            backend.clone(),
        );

        let candidates = searcher
            .search(&sample_answer_query())
            .await
            .expect("openai-compatible search results");
        let recorded = backend.single_request();

        assert_eq!(
            recorded.endpoint_url,
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(recorded.api_key, "sk-openai-compatible");
        assert_eq!(
            recorded.request,
            OpenAiCompatibleRequest {
                model: "gpt-compatible".to_owned(),
                messages: vec![
                    OpenAiCompatibleMessage {
                        role: OpenAiCompatibleMessageRole::System,
                        content: "只回答答案".to_owned(),
                    },
                    OpenAiCompatibleMessage {
                        role: OpenAiCompatibleMessageRole::User,
                        content: "题干：哪些选项属于元音？\n选项：选项：\nA. a;B. b;\n全文：[多选题 / multiple_choice] 哪些选项属于元音？\nA. a\nB. b\nBlank 1: 忽略".to_owned(),
                    },
                ],
            }
        );
        assert_eq!(
            candidates,
            vec![AnswerCandidate {
                provider:
                    "openai-compatible:gpt-compatible@https://api.example.com/v1/chat/completions"
                        .to_owned(),
                confidence: None,
                answer: "A#C".to_owned(),
            }]
        );
    }

    #[test]
    fn openai_compatible_searcher_returns_no_candidates_for_empty_content() {
        let backend = RecordingOpenAiCompatibleBackend::new(OpenAiCompatibleResponse {
            choices: vec![OpenAiCompatibleResponseChoice {
                message: OpenAiCompatibleResponseMessage {
                    content: Some("   ".to_owned()),
                },
            }],
        });
        let config = SearcherConfig {
            kind: "openai-compatible".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("https://api.openai.com/v1".to_owned()),
                ),
                (
                    "model".to_owned(),
                    serde_json::Value::String("gpt-4.1-mini".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    serde_json::Value::String("sk-example".to_owned()),
                ),
            ]),
        };
        let searcher = OpenAiCompatibleSearcherProvider::from_request_template_with_backend(
            OpenAiCompatibleRequestTemplate::from_config(&config).expect("request template"),
            backend,
        );

        let answers = searcher.extract_answers(&OpenAiCompatibleResponse {
            choices: vec![OpenAiCompatibleResponseChoice {
                message: OpenAiCompatibleResponseMessage {
                    content: Some("   ".to_owned()),
                },
            }],
        });

        assert!(answers.is_empty());
    }

    #[test]
    fn builds_chapter_work_query_batch_from_runtime_snapshot() {
        let snapshot = ChapterWorkFormSnapshot {
            title: "绪论测验".to_owned(),
            work_answer_id: 99001,
            total_question_num: 2,
            work_relation_id: 88001,
            full_score: "100".to_owned(),
            enc_work: "enc-work-submit-001".to_owned(),
            questions: vec![
                ChapterWorkQuestionSummary {
                    question_index: 0,
                    question_id: 700401,
                    question_type: 0,
                    question_type_label: "单选题".to_owned(),
                    question_kind: summary_question_kind(0),
                    prompt: "普通话的标准语音是什么？".to_owned(),
                    options: vec![
                        ExamQuestionOption {
                            key: "A".to_owned(),
                            value: "北京语音".to_owned(),
                            rich_content: None,
                        },
                        ExamQuestionOption {
                            key: "B".to_owned(),
                            value: "上海语音".to_owned(),
                            rich_content: None,
                        },
                    ],
                    blanks: Vec::new(),
                },
                ChapterWorkQuestionSummary {
                    question_index: 1,
                    question_id: 700402,
                    question_type: 2,
                    question_type_label: "填空题".to_owned(),
                    question_kind: summary_question_kind(2),
                    prompt: "请填写第一个元音。".to_owned(),
                    options: Vec::new(),
                    blanks: vec!["第1空".to_owned()],
                },
            ],
        };

        let batch = ChapterWorkQueryBatch::from(&snapshot);

        assert_eq!(batch.title, "绪论测验");
        assert_eq!(batch.work_answer_id, 99001);
        assert_eq!(batch.work_relation_id, 88001);
        assert_eq!(batch.total_question_num, 2);
        assert_eq!(batch.len(), 2);
        assert_eq!(
            batch
                .queries
                .iter()
                .map(|query| (query.question_index, query.question_id, query.source))
                .collect::<Vec<_>>(),
            vec![
                (0, 700401, AnswerQuerySource::ChapterWork),
                (1, 700402, AnswerQuerySource::ChapterWork),
            ]
        );
        assert_eq!(
            batch.queries[0].render_search_text(),
            "[单选题 / single_choice] 普通话的标准语音是什么？\nA. 北京语音\nB. 上海语音"
        );
        assert_eq!(batch.queries[1].blanks, vec!["第1空"]);
    }

    #[test]
    fn builds_empty_chapter_work_query_batch_from_empty_snapshot() {
        let snapshot = ChapterWorkFormSnapshot {
            title: "空白测验".to_owned(),
            work_answer_id: 0,
            total_question_num: 0,
            work_relation_id: 0,
            full_score: "0".to_owned(),
            enc_work: "enc-work-submit-ignored".to_owned(),
            questions: Vec::new(),
        };

        let batch = ChapterWorkQueryBatch::from(&snapshot);

        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
        assert_eq!(batch.total_question_num, 0);
    }

    #[tokio::test]
    async fn fans_out_searchers_in_configured_order() {
        let mut pipeline = SearcherPipeline::new();
        pipeline.add_provider(StubSearcherProvider {
            provider: "first",
            answers: vec!["A"],
        });
        pipeline.add_provider(StubSearcherProvider {
            provider: "second",
            answers: vec!["B", "C"],
        });

        let query = AnswerQuery::from(&ExamQuestionSummary {
            question_index: 0,
            question_id: 42,
            question_type: 3,
            question_type_label: "判断题".to_owned(),
            question_kind: summary_question_kind(3),
            prompt: "Rust 是编译型语言吗？".to_owned(),
            options: Vec::new(),
            blanks: Vec::new(),
        });

        let candidates = pipeline.search(&query).await.expect("search results");

        assert_eq!(pipeline.len(), 2);
        assert_eq!(
            candidates,
            vec![
                AnswerCandidate {
                    provider: "first:42".to_owned(),
                    confidence: None,
                    answer: "A".to_owned(),
                },
                AnswerCandidate {
                    provider: "second:42".to_owned(),
                    confidence: None,
                    answer: "B".to_owned(),
                },
                AnswerCandidate {
                    provider: "second:42".to_owned(),
                    confidence: None,
                    answer: "C".to_owned(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn selects_provider_ordered_candidates_for_chapter_work_batch() {
        let snapshot = ChapterWorkFormSnapshot {
            title: "绪论测验".to_owned(),
            work_answer_id: 99001,
            total_question_num: 2,
            work_relation_id: 88001,
            full_score: "100".to_owned(),
            enc_work: "enc-work-submit-001".to_owned(),
            questions: vec![
                ChapterWorkQuestionSummary {
                    question_index: 0,
                    question_id: 700401,
                    question_type: 0,
                    question_type_label: "单选题".to_owned(),
                    question_kind: summary_question_kind(0),
                    prompt: "普通话的标准语音是什么？".to_owned(),
                    options: vec![ExamQuestionOption {
                        key: "A".to_owned(),
                        value: "北京语音".to_owned(),
                        rich_content: None,
                    }],
                    blanks: Vec::new(),
                },
                ChapterWorkQuestionSummary {
                    question_index: 1,
                    question_id: 700402,
                    question_type: 3,
                    question_type_label: "判断题".to_owned(),
                    question_kind: summary_question_kind(3),
                    prompt: "普通话属于汉语。".to_owned(),
                    options: Vec::new(),
                    blanks: Vec::new(),
                },
            ],
        };
        let batch = ChapterWorkQueryBatch::from(&snapshot);
        let mut pipeline = SearcherPipeline::new();
        pipeline.add_provider(StubSearcherProvider {
            provider: "json",
            answers: vec!["A"],
        });
        pipeline.add_provider(StubSearcherProvider {
            provider: "sqlite",
            answers: vec!["B"],
        });

        let selection_batch: ChapterWorkCandidateSelectionBatch = pipeline
            .select_chapter_work_candidates(&batch)
            .await
            .expect("candidate selections");

        assert_eq!(selection_batch.title, "绪论测验");
        assert_eq!(selection_batch.work_answer_id, 99001);
        assert_eq!(selection_batch.total_question_num, 2);
        assert_eq!(selection_batch.len(), 2);
        assert_eq!(selection_batch.selected_count(), 2);
        assert_eq!(selection_batch.unresolved_count(), 0);
        assert_eq!(selection_batch.selections[0].query.question_id, 700401);
        assert_eq!(
            selection_batch.selections[0]
                .selected_candidate
                .as_ref()
                .expect("preferred candidate")
                .provider,
            "json:700401"
        );
        assert_eq!(selection_batch.selections[0].candidates.len(), 2);
        assert_eq!(
            selection_batch.selections[1]
                .selected_candidate
                .as_ref()
                .expect("preferred candidate")
                .answer,
            "A"
        );
    }

    #[tokio::test]
    async fn leaves_chapter_work_questions_unresolved_when_no_provider_returns_candidates() {
        let snapshot = ChapterWorkFormSnapshot {
            title: "空白测验".to_owned(),
            work_answer_id: 99002,
            total_question_num: 1,
            work_relation_id: 88002,
            full_score: "100".to_owned(),
            enc_work: "enc-work-submit-ignored".to_owned(),
            questions: vec![ChapterWorkQuestionSummary {
                question_index: 0,
                question_id: 700501,
                question_type: 2,
                question_type_label: "填空题".to_owned(),
                question_kind: summary_question_kind(2),
                prompt: "请填写标准语。".to_owned(),
                options: Vec::new(),
                blanks: vec!["第1空".to_owned()],
            }],
        };
        let batch = ChapterWorkQueryBatch::from(&snapshot);
        let pipeline = SearcherPipeline::new();

        let selection_batch = pipeline
            .select_chapter_work_candidates(&batch)
            .await
            .expect("candidate selections");

        assert_eq!(selection_batch.selected_count(), 0);
        assert_eq!(selection_batch.unresolved_count(), 1);
        assert!(selection_batch.selections[0].selected_candidate.is_none());
        assert!(selection_batch.selections[0].candidates.is_empty());
    }

    #[tokio::test]
    async fn json_searcher_matches_prompt_and_rendered_search_text() {
        let searcher = JsonSearcherProvider::from_path(json_searcher_fixture_path())
            .expect("json searcher fixture");

        let choice_candidates = searcher
            .search(&AnswerQuery::from(&ExamQuestionSummary {
                question_index: 0,
                question_id: 700101,
                question_type: 0,
                question_type_label: "单选题".to_owned(),
                question_kind: summary_question_kind(0),
                prompt: "普通话以哪种方言为基础方言？".to_owned(),
                options: vec![
                    ExamQuestionOption {
                        key: "A".to_owned(),
                        value: "吴方言".to_owned(),
                        rich_content: None,
                    },
                    ExamQuestionOption {
                        key: "B".to_owned(),
                        value: "北方方言".to_owned(),
                        rich_content: None,
                    },
                ],
                blanks: Vec::new(),
            }))
            .await
            .expect("choice candidates");
        assert_eq!(choice_candidates.len(), 1);
        assert_eq!(choice_candidates[0].answer, "B");

        let fill_blank_candidates = searcher
            .search(&AnswerQuery::from(&ChapterWorkQuestionSummary {
                question_index: 1,
                question_id: 700102,
                question_type: 2,
                question_type_label: "填空题".to_owned(),
                question_kind: summary_question_kind(2),
                prompt: "请补全“词汇”相关术语。".to_owned(),
                options: Vec::new(),
                blanks: vec!["词义：".to_owned(), "词性：".to_owned()],
            }))
            .await
            .expect("fill blank candidates");
        assert_eq!(
            fill_blank_candidates
                .iter()
                .map(|candidate| candidate.answer.as_str())
                .collect::<Vec<_>>(),
            vec!["语义内容", "名词"]
        );
    }

    #[tokio::test]
    async fn sqlite_searcher_matches_prompt_and_rendered_search_text() {
        let fixture =
            TempSqliteFixture::new("prompt-and-rendered", "question", "question", "answer");
        let config = SearcherConfig {
            kind: "sqlite".to_owned(),
            values: BTreeMap::from([(
                "file_path".to_owned(),
                serde_json::Value::String(fixture.path.display().to_string()),
            )]),
        };
        let searcher = SqliteSearcherProvider::from_config(&config, PathBuf::from(".").as_path())
            .expect("sqlite searcher fixture");

        let choice_candidates = searcher
            .search(&AnswerQuery::from(&ExamQuestionSummary {
                question_index: 0,
                question_id: 700101,
                question_type: 0,
                question_type_label: "单选题".to_owned(),
                question_kind: summary_question_kind(0),
                prompt: "普通话以哪种方言为基础方言？".to_owned(),
                options: vec![
                    ExamQuestionOption {
                        key: "A".to_owned(),
                        value: "吴方言".to_owned(),
                        rich_content: None,
                    },
                    ExamQuestionOption {
                        key: "B".to_owned(),
                        value: "北方方言".to_owned(),
                        rich_content: None,
                    },
                ],
                blanks: Vec::new(),
            }))
            .await
            .expect("choice candidates");
        assert_eq!(choice_candidates.len(), 1);
        assert_eq!(choice_candidates[0].answer, "B");

        let fill_blank_candidates = searcher
            .search(&AnswerQuery::from(&ChapterWorkQuestionSummary {
                question_index: 1,
                question_id: 700102,
                question_type: 2,
                question_type_label: "填空题".to_owned(),
                question_kind: summary_question_kind(2),
                prompt: "请补全“词汇”相关术语。".to_owned(),
                options: Vec::new(),
                blanks: vec!["词义：".to_owned(), "词性：".to_owned()],
            }))
            .await
            .expect("fill blank candidates");
        assert_eq!(
            fill_blank_candidates
                .iter()
                .map(|candidate| candidate.answer.as_str())
                .collect::<Vec<_>>(),
            vec!["语义内容", "名词"]
        );
    }

    #[tokio::test]
    async fn builds_pipeline_from_relative_json_searcher_config() {
        let mut values = BTreeMap::new();
        values.insert(
            "file_path".to_owned(),
            serde_json::Value::String("json_searcher_questions.json".to_owned()),
        );
        let config = SearcherConfig {
            kind: "jsonFileSearcher".to_owned(),
            values,
        };
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let pipeline = build_searcher_pipeline(&[config], &fixture_dir)
            .expect("pipeline build")
            .expect("configured pipeline");
        let candidates = pipeline
            .search(&AnswerQuery::from(&ExamQuestionSummary {
                question_index: 2,
                question_id: 700103,
                question_type: 3,
                question_type_label: "判断题".to_owned(),
                question_kind: summary_question_kind(3),
                prompt: "现代汉语共同语就是普通话。".to_owned(),
                options: Vec::new(),
                blanks: Vec::new(),
            }))
            .await
            .expect("true false candidates");

        assert_eq!(pipeline.len(), 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].answer, "对");
    }

    #[tokio::test]
    async fn builds_pipeline_from_relative_sqlite_searcher_config() {
        let fixture = TempSqliteFixture::new(
            "relative-config",
            "answer_bank",
            "prompt_text",
            "answer_text",
        );
        let fixture_dir = fixture.path.parent().expect("fixture parent").to_path_buf();
        let file_name = fixture
            .path
            .file_name()
            .expect("fixture filename")
            .to_string_lossy()
            .into_owned();
        let config = SearcherConfig {
            kind: "SqliteSearcher".to_owned(),
            values: BTreeMap::from([
                ("file_path".to_owned(), serde_json::Value::String(file_name)),
                (
                    "table".to_owned(),
                    serde_json::Value::String("answer_bank".to_owned()),
                ),
                (
                    "req_field".to_owned(),
                    serde_json::Value::String("prompt_text".to_owned()),
                ),
                (
                    "rsp_field".to_owned(),
                    serde_json::Value::String("answer_text".to_owned()),
                ),
            ]),
        };
        let pipeline = build_searcher_pipeline(&[config], &fixture_dir)
            .expect("pipeline build")
            .expect("configured pipeline");
        let candidates = pipeline
            .search(&AnswerQuery::from(&ExamQuestionSummary {
                question_index: 0,
                question_id: 700101,
                question_type: 0,
                question_type_label: "单选题".to_owned(),
                question_kind: summary_question_kind(0),
                prompt: "普通话以哪种方言为基础方言？".to_owned(),
                options: vec![
                    ExamQuestionOption {
                        key: "A".to_owned(),
                        value: "吴方言".to_owned(),
                        rich_content: None,
                    },
                    ExamQuestionOption {
                        key: "B".to_owned(),
                        value: "北方方言".to_owned(),
                        rich_content: None,
                    },
                ],
                blanks: Vec::new(),
            }))
            .await
            .expect("choice candidates");

        assert_eq!(pipeline.len(), 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].answer, "B");
    }

    #[test]
    fn builds_pipeline_from_generic_http_searcher_config() {
        let config = SearcherConfig {
            kind: "http".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "payload_mode".to_owned(),
                    serde_json::Value::String("form".to_owned()),
                ),
                (
                    "method".to_owned(),
                    serde_json::Value::String("GET".to_owned()),
                ),
                (
                    "q_field".to_owned(),
                    serde_json::Value::String("title".to_owned()),
                ),
                (
                    "o_field".to_owned(),
                    serde_json::Value::String("options".to_owned()),
                ),
                (
                    "a_field".to_owned(),
                    serde_json::Value::String("$.data.answer".to_owned()),
                ),
            ]),
        };
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let pipeline = build_searcher_pipeline(&[config], &fixture_dir)
            .expect("pipeline build")
            .expect("configured pipeline");

        assert_eq!(pipeline.len(), 1);
    }

    #[tokio::test]
    async fn builds_pipeline_from_http_fixture_response_config() {
        let fixture = TempHttpResponseFixture::new(
            "http-response",
            &serde_json::json!({
                "data": {
                    "answer": ["B"]
                }
            }),
        );
        let fixture_dir = fixture.path.parent().expect("fixture parent").to_path_buf();
        let file_name = fixture
            .path
            .file_name()
            .expect("fixture filename")
            .to_string_lossy()
            .into_owned();
        let config = SearcherConfig {
            kind: "http".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "payload_mode".to_owned(),
                    serde_json::Value::String("form".to_owned()),
                ),
                (
                    "method".to_owned(),
                    serde_json::Value::String("GET".to_owned()),
                ),
                (
                    "q_field".to_owned(),
                    serde_json::Value::String("title".to_owned()),
                ),
                (
                    "fixture_response_path".to_owned(),
                    serde_json::Value::String(file_name),
                ),
                (
                    "a_field".to_owned(),
                    serde_json::Value::String("$.data.answer".to_owned()),
                ),
            ]),
        };
        let pipeline = build_searcher_pipeline(&[config], &fixture_dir)
            .expect("pipeline build")
            .expect("configured pipeline");
        let candidates = pipeline
            .search(&AnswerQuery::from(&ExamQuestionSummary {
                question_index: 0,
                question_id: 700101,
                question_type: 0,
                question_type_label: "单选题".to_owned(),
                question_kind: summary_question_kind(0),
                prompt: "普通话以哪种方言为基础方言？".to_owned(),
                options: vec![
                    ExamQuestionOption {
                        key: "A".to_owned(),
                        value: "吴方言".to_owned(),
                        rich_content: None,
                    },
                    ExamQuestionOption {
                        key: "B".to_owned(),
                        value: "北方方言".to_owned(),
                        rich_content: None,
                    },
                ],
                blanks: Vec::new(),
            }))
            .await
            .expect("http search results");

        assert_eq!(pipeline.len(), 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].answer, "B");
    }

    #[test]
    fn builds_pipeline_from_legacy_rest_api_searcher_config() {
        let config = SearcherConfig {
            kind: "restApiSearcher".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "method".to_owned(),
                    serde_json::Value::String("POST".to_owned()),
                ),
                (
                    "q_field".to_owned(),
                    serde_json::Value::String("title".to_owned()),
                ),
                (
                    "o_field".to_owned(),
                    serde_json::Value::String("options".to_owned()),
                ),
                (
                    "a_field".to_owned(),
                    serde_json::Value::String("$.data.answer".to_owned()),
                ),
            ]),
        };
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let pipeline = build_searcher_pipeline(&[config], &fixture_dir)
            .expect("pipeline build")
            .expect("configured pipeline");

        assert_eq!(pipeline.len(), 1);
    }

    #[test]
    fn builds_pipeline_from_legacy_json_api_searcher_config() {
        let config = SearcherConfig {
            kind: "JsonApiSearcher".to_owned(),
            values: BTreeMap::from([(
                "url".to_owned(),
                serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
            )]),
        };
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let pipeline = build_searcher_pipeline(&[config], &fixture_dir)
            .expect("pipeline build")
            .expect("configured pipeline");

        assert_eq!(pipeline.len(), 1);
    }

    #[test]
    fn builds_pipeline_from_openai_compatible_searcher_config() {
        let config = SearcherConfig {
            kind: "openai-compatible".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("https://api.openai.com/v1".to_owned()),
                ),
                (
                    "model".to_owned(),
                    serde_json::Value::String("gpt-4.1-mini".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    serde_json::Value::String("sk-example".to_owned()),
                ),
            ]),
        };
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let pipeline = build_searcher_pipeline(&[config], &fixture_dir)
            .expect("pipeline build")
            .expect("configured pipeline");

        assert_eq!(pipeline.len(), 1);
    }

    #[tokio::test]
    async fn builds_pipeline_from_openai_compatible_fixture_response_config() {
        let fixture = TempHttpResponseFixture::new(
            "openai-compatible-response",
            &serde_json::json!({
                "choices": [
                    {
                        "message": {
                            "content": "B"
                        }
                    }
                ]
            }),
        );
        let fixture_dir = fixture.path.parent().expect("fixture parent").to_path_buf();
        let file_name = fixture
            .path
            .file_name()
            .expect("fixture filename")
            .to_string_lossy()
            .into_owned();
        let config = SearcherConfig {
            kind: "openai-compatible".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("https://api.openai.com/v1".to_owned()),
                ),
                (
                    "model".to_owned(),
                    serde_json::Value::String("gpt-4.1-mini".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    serde_json::Value::String("sk-example".to_owned()),
                ),
                (
                    "fixture_response_path".to_owned(),
                    serde_json::Value::String(file_name),
                ),
            ]),
        };
        let pipeline = build_searcher_pipeline(&[config], &fixture_dir)
            .expect("pipeline build")
            .expect("configured pipeline");
        let candidates = pipeline
            .search(&AnswerQuery::from(&ExamQuestionSummary {
                question_index: 0,
                question_id: 700101,
                question_type: 0,
                question_type_label: "单选题".to_owned(),
                question_kind: summary_question_kind(0),
                prompt: "普通话以哪种方言为基础方言？".to_owned(),
                options: vec![
                    ExamQuestionOption {
                        key: "A".to_owned(),
                        value: "吴方言".to_owned(),
                        rich_content: None,
                    },
                    ExamQuestionOption {
                        key: "B".to_owned(),
                        value: "北方方言".to_owned(),
                        rich_content: None,
                    },
                ],
                blanks: Vec::new(),
            }))
            .await
            .expect("openai-compatible search results");

        assert_eq!(pipeline.len(), 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].answer, "B");
    }

    #[tokio::test]
    async fn builds_pipeline_from_legacy_openai_searcher_fixture_response_config() {
        let fixture = TempHttpResponseFixture::new(
            "legacy-openai-response",
            &serde_json::json!({
                "choices": [
                    {
                        "message": {
                            "content": "B"
                        }
                    }
                ]
            }),
        );
        let fixture_dir = fixture.path.parent().expect("fixture parent").to_path_buf();
        let file_name = fixture
            .path
            .file_name()
            .expect("fixture filename")
            .to_string_lossy()
            .into_owned();
        let config = SearcherConfig {
            kind: "OpenAISearcher".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("https://api.example.com/v1".to_owned()),
                ),
                (
                    "model".to_owned(),
                    serde_json::Value::String("gpt-compatible".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    serde_json::Value::String("sk-example".to_owned()),
                ),
                (
                    "prompt".to_owned(),
                    serde_json::Value::String("题干：{question}\n全文：{search_text}".to_owned()),
                ),
                (
                    "fixture_response_path".to_owned(),
                    serde_json::Value::String(file_name),
                ),
            ]),
        };
        let pipeline = build_searcher_pipeline(&[config], &fixture_dir)
            .expect("pipeline build")
            .expect("configured pipeline");
        let candidates = pipeline
            .search(&AnswerQuery::from(&ExamQuestionSummary {
                question_index: 0,
                question_id: 700101,
                question_type: 0,
                question_type_label: "单选题".to_owned(),
                question_kind: summary_question_kind(0),
                prompt: "普通话以哪种方言为基础方言？".to_owned(),
                options: vec![
                    ExamQuestionOption {
                        key: "A".to_owned(),
                        value: "吴方言".to_owned(),
                        rich_content: None,
                    },
                    ExamQuestionOption {
                        key: "B".to_owned(),
                        value: "北方方言".to_owned(),
                        rich_content: None,
                    },
                ],
                blanks: Vec::new(),
            }))
            .await
            .expect("legacy openai search results");

        assert_eq!(pipeline.len(), 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].answer, "B");
    }

    #[test]
    fn builds_query_request_template_from_legacy_rest_api_config() {
        let config = SearcherConfig {
            kind: "restApiSearcher".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "method".to_owned(),
                    serde_json::Value::String("GET".to_owned()),
                ),
                (
                    "q_field".to_owned(),
                    serde_json::Value::String("title".to_owned()),
                ),
                (
                    "o_field".to_owned(),
                    serde_json::Value::String("options".to_owned()),
                ),
                (
                    "headers".to_owned(),
                    serde_json::json!({
                        "Authorization": "Bearer token",
                    }),
                ),
                (
                    "ext_params".to_owned(),
                    serde_json::json!({
                        "v": "1",
                    }),
                ),
                (
                    "a_field".to_owned(),
                    serde_json::Value::String("$.data.answer".to_owned()),
                ),
            ]),
        };
        let template =
            HttpSearcherRequestTemplate::from_config(&config).expect("rest api template");
        let request = template.build_request(&AnswerQuery::from(&ExamQuestionSummary {
            question_index: 0,
            question_id: 700101,
            question_type: 0,
            question_type_label: "单选题".to_owned(),
            question_kind: summary_question_kind(0),
            prompt: "普通话以哪种方言为基础方言？".to_owned(),
            options: vec![
                ExamQuestionOption {
                    key: "A".to_owned(),
                    value: "吴方言".to_owned(),
                    rich_content: None,
                },
                ExamQuestionOption {
                    key: "B".to_owned(),
                    value: "北方方言".to_owned(),
                    rich_content: None,
                },
            ],
            blanks: Vec::new(),
        }));

        assert_eq!(template.method, HttpSearcherMethod::Get);
        assert_eq!(template.payload_mode, HttpSearcherPayloadMode::Form);
        assert_eq!(request.answer_path, "$.data.answer");
        assert_eq!(
            request.payload,
            HttpSearcherRequestPayload::Query {
                fields: BTreeMap::from([
                    ("options".to_owned(), "吴方言#北方方言".to_owned()),
                    (
                        "title".to_owned(),
                        "普通话以哪种方言为基础方言？".to_owned()
                    ),
                    ("v".to_owned(), "1".to_owned()),
                ]),
            }
        );
    }

    #[test]
    fn builds_query_request_template_with_rich_option_metadata() {
        let config = SearcherConfig {
            kind: "restApiSearcher".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "method".to_owned(),
                    serde_json::Value::String("GET".to_owned()),
                ),
                (
                    "q_field".to_owned(),
                    serde_json::Value::String("title".to_owned()),
                ),
                (
                    "o_field".to_owned(),
                    serde_json::Value::String("options".to_owned()),
                ),
                (
                    "a_field".to_owned(),
                    serde_json::Value::String("$.data.answer".to_owned()),
                ),
            ]),
        };
        let template =
            HttpSearcherRequestTemplate::from_config(&config).expect("rest api template");
        let request = template.build_request(&sample_rich_option_answer_query());

        assert_eq!(
            request.payload,
            HttpSearcherRequestPayload::Query {
                fields: BTreeMap::from([
                    (
                        "options".to_owned(),
                        "图文 选项 | rich_html=<span>图文 <strong>选项</strong><img src=\"https://static.example/a.png\" /></span> | image_urls=https://static.example/a.png#[no visible option text] | rich_html=<img src=\"https://static.example/b.png\" /> | image_urls=https://static.example/b.png".to_owned(),
                    ),
                    ("title".to_owned(), "请选择图文选项".to_owned()),
                ]),
            }
        );
    }

    #[test]
    fn builds_json_request_template_from_legacy_json_api_config() {
        let config = SearcherConfig {
            kind: "JsonApiSearcher".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "headers".to_owned(),
                    serde_json::json!({
                        "X-Token": "abc",
                    }),
                ),
                (
                    "ext_params".to_owned(),
                    serde_json::json!({
                        "tenant": "demo",
                    }),
                ),
            ]),
        };
        let template =
            HttpSearcherRequestTemplate::from_config(&config).expect("json api template");
        let request = template.build_request(&AnswerQuery::from(&ExamQuestionSummary {
            question_index: 0,
            question_id: 700101,
            question_type: 0,
            question_type_label: "单选题".to_owned(),
            question_kind: summary_question_kind(0),
            prompt: "普通话以哪种方言为基础方言？".to_owned(),
            options: vec![
                ExamQuestionOption {
                    key: "A".to_owned(),
                    value: "吴方言".to_owned(),
                    rich_content: None,
                },
                ExamQuestionOption {
                    key: "B".to_owned(),
                    value: "北方方言".to_owned(),
                    rich_content: None,
                },
            ],
            blanks: Vec::new(),
        }));

        assert_eq!(template.method, HttpSearcherMethod::Post);
        assert_eq!(template.payload_mode, HttpSearcherPayloadMode::Json);
        assert_eq!(request.headers.get("X-Token"), Some(&"abc".to_owned()));
        assert_eq!(
            request.payload,
            HttpSearcherRequestPayload::Json {
                body: serde_json::json!({
                    "tenant": "demo",
                    "question": "普通话以哪种方言为基础方言？",
                    "type": 0,
                    "id": 700101,
                    "options": {
                        "A": "吴方言",
                        "B": "北方方言"
                    }
                }),
            }
        );
    }

    #[test]
    fn builds_json_request_template_with_rich_option_metadata() {
        let config = SearcherConfig {
            kind: "JsonApiSearcher".to_owned(),
            values: BTreeMap::from([(
                "url".to_owned(),
                serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
            )]),
        };
        let template =
            HttpSearcherRequestTemplate::from_config(&config).expect("json api template");
        let request = template.build_request(&sample_rich_option_answer_query());

        assert_eq!(
            request.payload,
            HttpSearcherRequestPayload::Json {
                body: serde_json::json!({
                    "question": "请选择图文选项",
                    "type": 0,
                    "id": 700304,
                    "options": {
                        "A": "图文 选项 | rich_html=<span>图文 <strong>选项</strong><img src=\"https://static.example/a.png\" /></span> | image_urls=https://static.example/a.png",
                        "B": "[no visible option text] | rich_html=<img src=\"https://static.example/b.png\" /> | image_urls=https://static.example/b.png"
                    }
                }),
            }
        );
    }

    #[test]
    fn rejects_get_json_http_request_templates() {
        let config = SearcherConfig {
            kind: "http".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "method".to_owned(),
                    serde_json::Value::String("GET".to_owned()),
                ),
                (
                    "payload_mode".to_owned(),
                    serde_json::Value::String("json".to_owned()),
                ),
            ]),
        };
        let error = HttpSearcherRequestTemplate::from_config(&config)
            .expect_err("GET + json should be rejected");

        assert!(matches!(error, crate::error::CpassError::Config(_)));
        assert!(
            error
                .to_string()
                .contains("cannot use method GET with payload_mode json")
        );
    }

    #[test]
    fn rejects_non_string_http_headers() {
        let config = SearcherConfig {
            kind: "http".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "headers".to_owned(),
                    serde_json::json!({
                        "Authorization": 123,
                    }),
                ),
            ]),
        };
        let error = HttpSearcherRequestTemplate::from_config(&config)
            .expect_err("non-string headers should fail");

        assert!(matches!(error, crate::error::CpassError::Config(_)));
        assert!(
            error
                .to_string()
                .contains("field 'headers' must contain only string values")
        );
    }

    #[test]
    fn rejects_invalid_http_searcher_urls() {
        let config = SearcherConfig {
            kind: "http".to_owned(),
            values: BTreeMap::from([(
                "url".to_owned(),
                serde_json::Value::String("ftp://127.0.0.1/question/search".to_owned()),
            )]),
        };

        let error =
            HttpSearcherProvider::from_config(&config).expect_err("invalid URL should fail");

        assert!(matches!(error, crate::error::CpassError::Config(_)));
        assert!(
            error
                .to_string()
                .contains("field 'url' must use http or https")
        );
    }

    #[test]
    fn rejects_unsupported_http_answer_path_syntax() {
        let config = SearcherConfig {
            kind: "http".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "a_field".to_owned(),
                    serde_json::Value::String("$.data[*]".to_owned()),
                ),
            ]),
        };

        let error = HttpSearcherProvider::from_config(&config)
            .expect_err("unsupported json path syntax should fail");

        assert!(matches!(error, crate::error::CpassError::Config(_)));
        assert!(
            error
                .to_string()
                .contains("only supports numeric array indexes")
        );
    }

    #[tokio::test]
    async fn http_searcher_issues_get_requests_and_extracts_multiple_answers() {
        let backend = RecordingHttpSearcherBackend::new(serde_json::json!({
            "data": {
                "answer": ["B", "普通话"]
            }
        }));
        let url = "http://127.0.0.1:8088/question/search".to_owned();
        let config = SearcherConfig {
            kind: "restApiSearcher".to_owned(),
            values: BTreeMap::from([
                ("url".to_owned(), serde_json::Value::String(url.clone())),
                (
                    "method".to_owned(),
                    serde_json::Value::String("GET".to_owned()),
                ),
                (
                    "q_field".to_owned(),
                    serde_json::Value::String("title".to_owned()),
                ),
                (
                    "o_field".to_owned(),
                    serde_json::Value::String("options".to_owned()),
                ),
                (
                    "headers".to_owned(),
                    serde_json::json!({
                        "Authorization": "Bearer token",
                    }),
                ),
                (
                    "ext_params".to_owned(),
                    serde_json::json!({
                        "tenant": "demo",
                    }),
                ),
                (
                    "a_field".to_owned(),
                    serde_json::Value::String("$.data.answer".to_owned()),
                ),
            ]),
        };
        let searcher = HttpSearcherProvider::from_request_template_with_backend(
            HttpSearcherRequestTemplate::from_config(&config).expect("request template"),
            &config.kind,
            backend.clone(),
        )
        .expect("http searcher");

        let candidates = searcher
            .search(&AnswerQuery::from(&ExamQuestionSummary {
                question_index: 0,
                question_id: 700101,
                question_type: 0,
                question_type_label: "单选题".to_owned(),
                question_kind: summary_question_kind(0),
                prompt: "普通话以哪种方言为基础方言？".to_owned(),
                options: vec![
                    ExamQuestionOption {
                        key: "A".to_owned(),
                        value: "吴方言".to_owned(),
                        rich_content: None,
                    },
                    ExamQuestionOption {
                        key: "B".to_owned(),
                        value: "北方方言".to_owned(),
                        rich_content: None,
                    },
                ],
                blanks: Vec::new(),
            }))
            .await
            .expect("http search results");

        let recorded = backend.single_request();
        assert_eq!(recorded.request.url, url);
        assert_eq!(recorded.request.method, HttpSearcherMethod::Get);
        assert_eq!(
            recorded.request.payload,
            HttpSearcherRequestPayload::Query {
                fields: BTreeMap::from([
                    ("options".to_owned(), "吴方言#北方方言".to_owned()),
                    ("tenant".to_owned(), "demo".to_owned(),),
                    (
                        "title".to_owned(),
                        "普通话以哪种方言为基础方言？".to_owned(),
                    ),
                ]),
            }
        );
        assert_eq!(
            recorded
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer token")
        );
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.answer.as_str())
                .collect::<Vec<_>>(),
            vec!["B", "普通话"]
        );
    }

    #[tokio::test]
    async fn http_searcher_issues_json_posts_and_extracts_nested_scalar_answers() {
        let backend = RecordingHttpSearcherBackend::new(serde_json::json!({
            "data": {
                "answers": [["语义内容"], ["名词"]]
            }
        }));
        let url = "http://127.0.0.1:8088/question/search".to_owned();
        let config = SearcherConfig {
            kind: "JsonApiSearcher".to_owned(),
            values: BTreeMap::from([
                ("url".to_owned(), serde_json::Value::String(url.clone())),
                (
                    "headers".to_owned(),
                    serde_json::json!({
                        "X-Token": "abc",
                    }),
                ),
                (
                    "ext_params".to_owned(),
                    serde_json::json!({
                        "tenant": "demo",
                    }),
                ),
                (
                    "a_field".to_owned(),
                    serde_json::Value::String("$.data.answers[0]".to_owned()),
                ),
            ]),
        };
        let searcher = HttpSearcherProvider::from_request_template_with_backend(
            HttpSearcherRequestTemplate::from_config(&config).expect("request template"),
            &config.kind,
            backend.clone(),
        )
        .expect("json api searcher");

        let candidates = searcher
            .search(&AnswerQuery::from(&ExamQuestionSummary {
                question_index: 0,
                question_id: 700102,
                question_type: 0,
                question_type_label: "单选题".to_owned(),
                question_kind: summary_question_kind(0),
                prompt: "请补全“词汇”相关术语。".to_owned(),
                options: vec![
                    ExamQuestionOption {
                        key: "A".to_owned(),
                        value: "语义内容".to_owned(),
                        rich_content: None,
                    },
                    ExamQuestionOption {
                        key: "B".to_owned(),
                        value: "名词".to_owned(),
                        rich_content: None,
                    },
                ],
                blanks: Vec::new(),
            }))
            .await
            .expect("json api search results");

        let recorded = backend.single_request();
        assert_eq!(recorded.request.url, url);
        assert_eq!(recorded.request.method, HttpSearcherMethod::Post);
        assert!(recorded.headers.get("content-type").is_none());
        assert_eq!(
            recorded
                .headers
                .get("x-token")
                .and_then(|value| value.to_str().ok()),
            Some("abc")
        );
        assert_eq!(
            recorded.request.payload,
            HttpSearcherRequestPayload::Json {
                body: serde_json::json!({
                    "tenant": "demo",
                    "question": "请补全“词汇”相关术语。",
                    "type": 0,
                    "id": 700102,
                    "options": {
                        "A": "语义内容",
                        "B": "名词"
                    }
                })
            }
        );
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.answer.as_str())
                .collect::<Vec<_>>(),
            vec!["语义内容"]
        );
    }

    #[test]
    fn http_searcher_returns_no_candidates_for_missing_response_path() {
        let backend = RecordingHttpSearcherBackend::new(serde_json::json!({
            "meta": {
                "ok": true
            }
        }));
        let config = SearcherConfig {
            kind: "http".to_owned(),
            values: BTreeMap::from([
                (
                    "url".to_owned(),
                    serde_json::Value::String("http://127.0.0.1:8088/question/search".to_owned()),
                ),
                (
                    "a_field".to_owned(),
                    serde_json::Value::String("$.data.answer".to_owned()),
                ),
            ]),
        };
        let searcher = HttpSearcherProvider::from_request_template_with_backend(
            HttpSearcherRequestTemplate::from_config(&config).expect("request template"),
            &config.kind,
            backend,
        )
        .expect("http searcher");

        let answers = searcher.extract_answers(&serde_json::json!({
            "meta": {
                "ok": true
            }
        }));

        assert!(answers.is_empty());
    }

    #[test]
    fn rejects_unknown_searcher_kinds_during_pipeline_build() {
        let config = SearcherConfig {
            kind: "mystery-searcher".to_owned(),
            values: BTreeMap::new(),
        };
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/legacy");
        let error = match build_searcher_pipeline(&[config], &fixture_dir) {
            Ok(_) => panic!("unsupported searcher kind should fail"),
            Err(error) => error,
        };

        assert!(matches!(error, crate::error::CpassError::Config(_)));
        assert!(
            error
                .to_string()
                .contains("currently supported: json, sqlite, http, openai-compatible")
        );
    }
}
