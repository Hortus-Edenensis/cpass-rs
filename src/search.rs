use std::{fmt, fs::File, time::Duration};

use anyhow::{Result, anyhow, bail};
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor},
};
use serde_json::{Map, Value, json};
use serde_json_path::JsonPath;

use crate::{
    model::Question,
    questions::{normalize_answer, normalize_text},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub source: String,
    pub answer: Value,
    pub error: Option<String>,
    pub conflict: bool,
}

pub struct Searchers {
    client: Client,
    sources: Vec<(String, Result<Source, String>)>,
}

enum Source {
    Json(Vec<(String, Value)>),
    Sqlite {
        path: String,
        table: String,
        question: String,
        answer: String,
    },
    Http(Box<HttpSource>),
}

struct HttpSource {
    kind: String,
    config: Value,
    url: String,
    headers: HeaderMap,
    method: String,
    query: JsonPath,
    timeout: Duration,
}

impl Searchers {
    pub fn new(configs: &[Value]) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| anyhow!("无法创建搜题客户端"))?;
        let sources = configs
            .iter()
            .enumerate()
            .map(|(index, config)| {
                let kind = canonical_kind(config.get("type").and_then(Value::as_str).unwrap_or(""));
                let name = format!("{}#{}", kind.unwrap_or("UnknownSearcher"), index + 1);
                let source = resolve_env(config)
                    .and_then(|config| {
                        Source::new(kind.ok_or_else(|| anyhow!("不支持的搜索器类型"))?, &config)
                    })
                    .map_err(|error| error.to_string());
                (name, source)
            })
            .collect();
        Ok(Self { client, sources })
    }

    pub fn validate(&self) -> Result<()> {
        let errors: Vec<String> = self
            .sources
            .iter()
            .filter_map(|(name, source)| {
                source
                    .as_ref()
                    .err()
                    .map(|error| format!("{name}: {error}"))
            })
            .collect();
        if !errors.is_empty() {
            bail!("{}", errors.join("; "));
        }
        Ok(())
    }

    pub fn search(&self, question: &Question) -> Vec<Candidate> {
        self.sources
            .iter()
            .flat_map(|(name, source)| {
                let result = match source {
                    Ok(source) => source.search(&self.client, question),
                    Err(error) => Err(anyhow!(error.clone())),
                };
                match result {
                    Ok(answers) => candidates(name, question, answers),
                    Err(error) => vec![Candidate {
                        source: name.clone(),
                        answer: Value::Null,
                        conflict: error.to_string() == "题库答案冲突",
                        error: Some(error.to_string()),
                    }],
                }
            })
            .collect()
    }
}

fn canonical_kind(value: &str) -> Option<&'static str> {
    [
        "JsonFileSearcher",
        "SqliteSearcher",
        "RestApiSearcher",
        "JsonApiSearcher",
        "EnncySearcher",
        "CxSearcher",
        "TiKuHaiSearcher",
        "MukeSearcher",
        "LyCk6Searcher",
        "LemonSearcher",
        "OpenAISearcher",
        "OllamaSearcherAPI",
    ]
    .into_iter()
    .find(|kind| kind.eq_ignore_ascii_case(value))
}

fn resolve_env(value: &Value) -> Result<Value> {
    match value {
        Value::String(text) if text.starts_with("$ENV:") => {
            let key = &text[5..];
            if key.is_empty() || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                bail!("环境变量引用无效");
            }
            Ok(Value::String(
                std::env::var(key).map_err(|_| anyhow!("缺少搜索器环境变量"))?,
            ))
        }
        Value::Array(values) => values
            .iter()
            .map(resolve_env)
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), resolve_env(value)?)))
            .collect::<Result<Map<_, _>>>()
            .map(Value::Object),
        _ => Ok(value.clone()),
    }
}

fn string<'a>(config: &'a Value, key: &str, default: &'a str) -> &'a str {
    config.get(key).and_then(Value::as_str).unwrap_or(default)
}

fn required<'a>(config: &'a Value, key: &str) -> Result<&'a str> {
    config
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("搜索器配置缺少必需字段: {key}"))
}

fn identifier(value: &str) -> Result<String> {
    let mut chars = value.bytes();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        bail!("SQLite 表名或字段名无效");
    }
    Ok(format!("\"{value}\""))
}

fn sqlite_connection(path: &str) -> Result<Connection> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| anyhow!("无法只读打开 SQLite 题库"))?;
    // Missing quoted columns must fail instead of becoming string-literal answers.
    connection
        .set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DQS_DML, false)
        .map_err(|_| anyhow!("无法配置 SQLite 题库只读查询"))?;
    Ok(connection)
}

impl Source {
    fn new(kind: &str, config: &Value) -> Result<Self> {
        match kind {
            "JsonFileSearcher" => {
                let file = File::open(required(config, "file_path")?)
                    .map_err(|_| anyhow!("无法读取 JSON 题库"))?;
                let Entries(entries) =
                    serde_json::from_reader(file).map_err(|_| anyhow!("JSON 题库格式无效"))?;
                Ok(Self::Json(entries))
            }
            "SqliteSearcher" => {
                let path = required(config, "file_path")?.to_owned();
                let table = identifier(string(config, "table", "question"))?;
                let question = identifier(string(config, "req_field", "question"))?;
                let answer = identifier(string(config, "rsp_field", "answer"))?;
                let connection = sqlite_connection(&path)?;
                connection
                    .prepare(&format!("SELECT {question}, {answer} FROM {table} LIMIT 0"))
                    .map_err(|_| anyhow!("SQLite 题库字段无效"))?;
                Ok(Self::Sqlite {
                    path,
                    table,
                    question,
                    answer,
                })
            }
            _ => Ok(Self::Http(Box::new(HttpSource::new(kind, config)?))),
        }
    }

    fn search(&self, client: &Client, question: &Question) -> Result<Vec<Value>> {
        match self {
            Self::Json(entries) => {
                let title = normalize_text(&question.value);
                Ok(entries
                    .iter()
                    .filter(|(key, _)| normalize_text(key) == title)
                    .map(|(_, answer)| answer.clone())
                    .collect())
            }
            Self::Sqlite {
                path,
                table,
                question: field,
                answer,
            } => {
                let connection = sqlite_connection(path)?;
                let mut statement = connection
                    .prepare(&format!("SELECT {answer} FROM {table} WHERE {field} = ?1"))
                    .map_err(|_| anyhow!("SQLite 题库字段无效"))?;
                let rows = statement
                    .query_map([&question.value], |row| {
                        Ok(match row.get_ref(0)? {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(value) => json!(value),
                            ValueRef::Real(value) => json!(value),
                            ValueRef::Text(value) => {
                                Value::String(String::from_utf8_lossy(value).into_owned())
                            }
                            ValueRef::Blob(_) => Value::Null,
                        })
                    })
                    .map_err(|_| anyhow!("SQLite 题库查询失败"))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| anyhow!("SQLite 题库读取失败"))
            }
            Self::Http(source) => source.search(client, question),
        }
    }
}

fn candidates(source: &str, question: &Question, answers: Vec<Value>) -> Vec<Candidate> {
    if answers.is_empty() {
        return vec![Candidate {
            source: source.to_owned(),
            answer: Value::Null,
            error: Some("题目未匹配".to_owned()),
            conflict: false,
        }];
    }
    let normalized: Vec<Value> = answers
        .iter()
        .filter_map(|answer| normalize_answer(question, answer))
        .collect();
    let conflict = normalized
        .first()
        .is_some_and(|first| normalized.iter().any(|answer| answer != first));
    answers
        .into_iter()
        .map(|answer| Candidate {
            source: source.to_owned(),
            answer,
            error: conflict.then(|| "题库答案冲突".to_owned()),
            conflict,
        })
        .collect()
}

struct Entries(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct EntriesVisitor;
        impl<'de> Visitor<'de> for EntriesVisitor {
            type Value = Entries;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a question-to-answer object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_map(EntriesVisitor)
    }
}

impl HttpSource {
    fn new(kind: &str, input: &Value) -> Result<Self> {
        let mut config = input.clone();
        let token = string(input, "token", "");
        let (default_url, method, path, params, headers) = match kind {
            "EnncySearcher" => (
                "https://tk.enncy.cn/query".to_owned(),
                "GET",
                "$.data.answer",
                json!({"v":1,"token":required(input,"token")?}),
                json!({}),
            ),
            "CxSearcher" => (
                "https://cx.icodef.com/wyn-nb".to_owned(),
                "POST",
                "$.data",
                json!({"v":4}),
                json!({"Authorization":required(input,"token")?}),
            ),
            "TiKuHaiSearcher" => (
                "http://api.tikuhai.com/search".to_owned(),
                "JSON",
                "$.data.answer[*]",
                json!({"key":required(input,"token")?}),
                json!({"Host":"api.tikuhai.com","referer":"https://mooc1.chaoxing.com","Content-Type":"application/json"}),
            ),
            "MukeSearcher" => (
                "https://api.muketool.com/cx/v2/query".to_owned(),
                "JSON",
                "$.data[*]",
                json!({}),
                json!({"Host":"api.muketool.com","Content-Type":"application/json"}),
            ),
            "LyCk6Searcher" => {
                let url = if token.chars().count() == 10 {
                    let mut url =
                        reqwest::Url::parse("https://lyck6.cn//scriptService/api/autoAnswer/")
                            .expect("static URL");
                    url.path_segments_mut()
                        .expect("static URL")
                        .pop_if_empty()
                        .push(token);
                    url.query_pairs_mut().append_pair(
                        "gpt",
                        &input
                            .get("gpt")
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                            .to_string(),
                    );
                    url.to_string()
                } else {
                    "https://lyck6.cn/scriptService/api/autoFreeAnswer".to_owned()
                };
                (
                    url,
                    "JSON",
                    "$.result.answers[*][0]",
                    json!({}),
                    json!({"Content-Type":"application/json"}),
                )
            }
            "LemonSearcher" => (
                "https://api.lemtk.xyz/api/v1/mcx".to_owned(),
                "JSON",
                "$.data.answer",
                json!({"v":"1.0","uid":"703382225"}),
                json!({"Authorization":format!("Bearer {}",required(input,"token")?),"Content-Type":"application/json","User-Agent":"CxKitty"}),
            ),
            "OpenAISearcher" => (
                format!(
                    "{}/chat/completions",
                    required(input, "base_url")?.trim_end_matches('/')
                ),
                "OPENAI",
                "$.choices[0].message.content",
                json!({}),
                json!({"Authorization":format!("Bearer {}",required(input,"api_key")?)}),
            ),
            "OllamaSearcherAPI" => (
                format!(
                    "{}/api/generate",
                    required(input, "base_url")?.trim_end_matches('/')
                ),
                "OLLAMA",
                "$.response",
                json!({}),
                json!({}),
            ),
            "RestApiSearcher" => (
                required(input, "url")?.to_owned(),
                string(input, "method", "POST"),
                string(input, "a_field", "$.data"),
                json!({}),
                json!({}),
            ),
            "JsonApiSearcher" => (
                required(input, "url")?.to_owned(),
                "JSON",
                string(input, "a_field", "$.data"),
                json!({}),
                json!({}),
            ),
            _ => bail!("不支持的 HTTP 搜索器"),
        };
        if !matches!(method, "GET" | "POST" | "JSON" | "OPENAI" | "OLLAMA") {
            bail!("搜索器请求方法无效");
        }
        if matches!(method, "OPENAI" | "OLLAMA") {
            required(input, "prompt")?;
            required(input, "system_prompt")?;
            if method == "OPENAI" {
                required(input, "model")?;
            }
        }
        let mut ext_params = params.as_object().expect("static object").clone();
        if let Some(extra) = input.get("ext_params")
            && !extra.is_null()
        {
            ext_params.extend(
                extra
                    .as_object()
                    .ok_or_else(|| anyhow!("搜索器 ext_params 必须是对象"))?
                    .clone(),
            );
        }
        config["ext_params"] = Value::Object(ext_params);
        if kind == "EnncySearcher" {
            config["q_field"] = json!("title");
        }
        let mut header_values = headers.as_object().expect("static object").clone();
        if matches!(kind, "CxSearcher" | "TiKuHaiSearcher" | "MukeSearcher") {
            header_values.insert(
                "User-Agent".to_owned(),
                json!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/117.0.0.0 Safari/537.36"),
            );
        }
        if let Some(extra) = input.get("headers")
            && !extra.is_null()
        {
            header_values.extend(
                extra
                    .as_object()
                    .ok_or_else(|| anyhow!("搜索器 headers 必须是对象"))?
                    .clone(),
            );
        }
        let mut headers = HeaderMap::new();
        for (key, value) in header_values {
            headers.insert(
                HeaderName::from_bytes(key.as_bytes())
                    .map_err(|_| anyhow!("搜索器请求头名称无效"))?,
                HeaderValue::from_str(
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("搜索器请求头值必须是字符串"))?,
                )
                .map_err(|_| anyhow!("搜索器请求头值无效"))?,
            );
        }
        let url = string(input, "url", &default_url).to_owned();
        let parsed = reqwest::Url::parse(&url).map_err(|_| anyhow!("搜索器 URL 无效"))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            bail!("搜索器 URL 必须使用 HTTP(S) 且不含用户凭证");
        }
        let query = JsonPath::parse(path).map_err(|_| anyhow!("搜索器 JSONPath 无效"))?;
        let timeout = input
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(30);
        if timeout == 0 || timeout > 300 {
            bail!("搜索器超时必须为 1 至 300 秒");
        }
        Ok(Self {
            kind: kind.to_owned(),
            config,
            url,
            headers,
            method: method.to_owned(),
            query,
            timeout: Duration::from_secs(timeout),
        })
    }

    fn search(&self, client: &Client, question: &Question) -> Result<Vec<Value>> {
        let payload = self.payload(question)?;
        let request = if self.method == "GET" {
            client.get(&self.url).query(&form(&payload)?)
        } else if self.method == "POST" {
            client.post(&self.url).form(&form(&payload)?)
        } else {
            client.post(&self.url).json(&payload)
        };
        let response = request
            .headers(self.headers.clone())
            .timeout(self.timeout)
            .send()
            .map_err(|_| anyhow!("搜索器请求失败"))?;
        if response.status().as_u16() == 409 {
            bail!("题库答案冲突");
        }
        if !response.status().is_success() {
            bail!("搜索器 HTTP 请求失败 ({})", response.status().as_u16());
        }
        let response: Value = response
            .json()
            .map_err(|_| anyhow!("搜索器返回了无效 JSON"))?;
        self.parse(&response)
    }

    fn payload(&self, question: &Question) -> Result<Value> {
        if matches!(self.method.as_str(), "OPENAI" | "OLLAMA") {
            let options = match &question.options {
                Value::Object(values) => values
                    .iter()
                    .map(|(key, value)| format!("{key}. {};", scalar(value)))
                    .collect::<String>(),
                Value::Array(values) => values
                    .iter()
                    .map(|value| format!("{};", scalar(value)))
                    .collect(),
                _ => String::new(),
            };
            let options = if options.is_empty() {
                options
            } else {
                format!("选项：\n{options}")
            };
            let prompt = format_prompt(required(&self.config, "prompt")?, question, &options)?;
            let system = required(&self.config, "system_prompt")?;
            return if self.method == "OPENAI" {
                Ok(
                    json!({"model":required(&self.config,"model")?,"temperature":0.5,"messages":[{"role":"system","content":system},{"role":"user","content":prompt}]}),
                )
            } else {
                Ok(
                    json!({"model":string(&self.config,"model","llama3"),"system":system,"prompt":prompt,"stream":false}),
                )
            };
        }
        let mut payload = Map::new();
        payload.insert(
            string(&self.config, "q_field", "question").to_owned(),
            json!(question.value),
        );
        if self.method == "JSON" {
            payload.insert("type".to_owned(), json!(question.kind.0));
            payload.insert("id".to_owned(), json!(question.id));
        }
        if let Some(extra) = self.config["ext_params"].as_object() {
            payload.extend(extra.clone());
        }
        if let (Some(field), Some(options)) = (
            self.config.get("o_field").and_then(Value::as_str),
            question.options.as_object(),
        ) {
            payload.insert(
                field.to_owned(),
                Value::String(options.values().map(scalar).collect::<Vec<_>>().join("#")),
            );
        } else if self.method == "JSON" && !question.options.is_null() {
            payload.insert("options".to_owned(), question.options.clone());
        }
        Ok(Value::Object(payload))
    }

    fn parse(&self, response: &Value) -> Result<Vec<Value>> {
        if matches!(
            response.get("code").and_then(Value::as_i64),
            Some(-409) | Some(409)
        ) {
            bail!("题库答案冲突");
        }
        let expected = match self.kind.as_str() {
            "CxSearcher" | "MukeSearcher" => Some(1),
            "TiKuHaiSearcher" => Some(200),
            "LyCk6Searcher" => Some(0),
            "LemonSearcher" => Some(1000),
            _ => None,
        };
        if expected.is_some_and(|code| response.get("code").and_then(Value::as_i64) != Some(code)) {
            bail!("题库返回失败状态");
        }
        let values: Vec<Value> = self
            .query
            .query(response)
            .all()
            .into_iter()
            .cloned()
            .collect();
        if self.kind == "EnncySearcher" {
            for value in &values {
                if let Some(text) = value.as_str() {
                    if matches!(text, "很抱歉, 题目搜索不到。" | "非常抱歉，题目搜索不到。")
                    {
                        bail!("题目未匹配");
                    }
                    if text == "配置为空或者配置错误，请自行检查或者联系作者查看。"
                        || text.starts_with("题库配置的“凭证”被刷新")
                    {
                        bail!("题库凭证无效");
                    }
                }
            }
        }
        Ok(values)
    }
}

fn scalar(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn form(payload: &Value) -> Result<Vec<(String, String)>> {
    payload
        .as_object()
        .ok_or_else(|| anyhow!("搜索器请求参数必须是对象"))?
        .iter()
        .map(|(key, value)| Ok((key.clone(), scalar(value))))
        .collect()
}

fn format_prompt(template: &str, question: &Question, options: &str) -> Result<String> {
    let mut result = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                result.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                result.push('}');
            }
            '{' => {
                let mut field = String::new();
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == '}' {
                        closed = true;
                        break;
                    }
                    field.push(ch);
                }
                if !closed {
                    bail!("模型提问模板括号不完整");
                }
                result.push_str(match field.as_str() {
                    "type" => question.kind.name(),
                    "value" => &question.value,
                    "options" => options,
                    _ => bail!("模型提问模板仅支持 type、value、options 字段"),
                });
            }
            '}' => bail!("模型提问模板括号不完整"),
            _ => result.push(ch),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::QuestionType;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        sync::mpsc,
        thread,
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(extension: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "cpass-search-{}-{}.{}",
                std::process::id(),
                rand::random::<u64>(),
                extension
            )))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn question() -> Question {
        Question {
            id: 42,
            value: "哪项不正确？".to_owned(),
            kind: QuestionType::SINGLE,
            options: json!({"A":"北京","B":"上海"}),
            answer: Value::Null,
        }
    }

    fn serve(status: u16, body: Value, extra_headers: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (send, receive) = mpsc::channel();
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
            body.to_string().len()
        );
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let length = stream.read(&mut chunk).unwrap();
                if length == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..length]);
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            stream.write_all(response.as_bytes()).unwrap();
            send.send(String::from_utf8(bytes).unwrap()).unwrap();
        });
        (url, receive)
    }

    #[test]
    fn json_preserves_duplicates_and_canonical_conflicts() {
        let file = Fixture::new("json");
        let q = question();
        for (data, count, conflict) in [
            (r#"{"哪项不正确？":"A","哪项不正确？":"北京"}"#, 2, false),
            (r#"{" 哪项不正确？ ":"A","哪项不正确？":"B"}"#, 2, true),
            (
                r#"{"哪项不正确？":"A","哪项不正确？":"未知答案"}"#,
                2,
                false,
            ),
            (r#"{"哪项正确？":"A","哪项不正确":"A"}"#, 1, false),
        ] {
            std::fs::write(&file.0, data).unwrap();
            let sources =
                Searchers::new(&[json!({"type":"jsonFileSearcher","file_path":file.0})]).unwrap();
            let candidates = sources.search(&q);
            assert_eq!(candidates.len(), count);
            assert!(
                candidates
                    .iter()
                    .all(|candidate| candidate.conflict == conflict)
            );
            if count == 1 {
                assert_eq!(candidates[0].error.as_deref(), Some("题目未匹配"));
            }
        }
        let Entries(entries) = serde_json::from_str(r#"{"A&amp;B":false,"A&B":false}"#).unwrap();
        let source = Source::Json(entries);
        let mut q = question();
        q.value = "A&B".to_owned();
        q.kind = QuestionType::TRUE_FALSE;
        assert_eq!(
            source.search(&Client::new(), &q).unwrap(),
            vec![json!(false), json!(false)]
        );
        q.value = "a&b".to_owned();
        assert!(source.search(&Client::new(), &q).unwrap().is_empty());
    }

    #[test]
    fn sqlite_is_read_only_parameterized_and_rejects_identifiers() {
        let file = Fixture::new("sqlite");
        let connection = Connection::open(&file.0).unwrap();
        connection
            .execute_batch("CREATE TABLE question(question TEXT, answer TEXT);")
            .unwrap();
        let mut q = question();
        q.value = "'; DROP TABLE question; --".to_owned();
        connection
            .execute(
                "INSERT INTO question VALUES (?1,?2)",
                rusqlite::params![q.value, "A"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO question VALUES (?1,?2)",
                rusqlite::params![q.value, "B"],
            )
            .unwrap();
        drop(connection);
        let config = json!({"type":"sqliteSearcher","file_path":file.0});
        let sources = Searchers::new(std::slice::from_ref(&config)).unwrap();
        assert!(
            sources
                .search(&q)
                .iter()
                .all(|candidate| candidate.conflict)
        );
        let mut bad = config.clone();
        bad["table"] = json!("question; DROP TABLE question");
        let mut missing = config.clone();
        missing["rsp_field"] = json!("missing_answer");
        assert!(Searchers::new(&[missing]).unwrap().validate().is_err());
        let sources = Searchers::new(&[bad, config]).unwrap();
        let output = sources.search(&q);
        assert_eq!(output.len(), 3);
        assert!(output[0].error.as_ref().unwrap().contains("字段名无效"));
        let absent = Fixture::new("sqlite");
        let sources =
            Searchers::new(&[json!({"type":"SqliteSearcher","file_path":absent.0})]).unwrap();
        assert!(sources.search(&q)[0].error.is_some());
        assert!(!absent.0.exists());
    }

    #[test]
    fn rest_jsonpath_filters_return_all_matches_and_form_fields() {
        let (url, request) = serve(
            200,
            json!({"data":[{"answer":"A","valid":true},{"answer":"B","valid":false},{"answer":"北京","valid":true}]}),
            "",
        );
        let sources = Searchers::new(&[json!({"type":"restApiSearcher","url":url,"q_field":"title","o_field":"choices","a_field":"$.data[?@.valid == true].answer","headers":{"X-Token":"secret"},"ext_params":{"v":1}})]).unwrap();
        let result = sources.search(&question());
        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .all(|candidate| !candidate.conflict && candidate.error.is_none())
        );
        let request = request.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(request.starts_with("POST / HTTP/1.1"));
        assert!(request.to_lowercase().contains("x-token: secret\r\n"));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let parsed = reqwest::Url::parse(&format!("http://localhost/?{body}")).unwrap();
        let params: std::collections::BTreeMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(params.get("title").unwrap(), "哪项不正确？");
        assert_eq!(params.get("choices").unwrap(), "北京#上海");
        assert_eq!(params.get("v").unwrap(), "1");
    }

    #[test]
    fn json_api_passes_question_and_keeps_raw_false() {
        let (url, request) = serve(200, json!({"answer":false}), "");
        let sources =
            Searchers::new(&[json!({"type":"JsonApiSearcher","url":url,"a_field":"$.answer"})])
                .unwrap();
        let mut q = question();
        q.kind = QuestionType::TRUE_FALSE;
        assert_eq!(sources.search(&q)[0].answer, json!(false));
        let request = request.recv_timeout(Duration::from_secs(5)).unwrap();
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["id"], 42);
        assert_eq!(body["type"], 3);
        assert_eq!(body["options"], q.options);
    }

    #[test]
    fn providers_use_original_protocol_and_reject_failure_receipts() {
        for (kind, body, expected_method) in [
            ("EnncySearcher", json!({"data":{"answer":"A"}}), "GET"),
            ("CxSearcher", json!({"code":1,"data":"A"}), "POST"),
            (
                "TiKuHaiSearcher",
                json!({"code":200,"data":{"answer":["A","北京"]}}),
                "JSON",
            ),
            (
                "MukeSearcher",
                json!({"code":1,"data":["A","北京"]}),
                "JSON",
            ),
            (
                "LyCk6Searcher",
                json!({"code":0,"result":{"answers":[["A"],["北京"]]}}),
                "JSON",
            ),
            (
                "LemonSearcher",
                json!({"code":1000,"data":{"answer":"A"}}),
                "JSON",
            ),
        ] {
            let source = HttpSource::new(kind, &json!({"token":"1234567890"})).unwrap();
            assert_eq!(source.method, expected_method);
            assert!(!source.parse(&body).unwrap().is_empty());
            let (url, request) = serve(200, body, "");
            let searchers = Searchers::new(&[json!({
                "type": kind, "token": "1234567890", "url": url
            })])
            .unwrap();
            let result = searchers.search(&question());
            assert!(
                result
                    .iter()
                    .all(|candidate| candidate.error.is_none() && !candidate.conflict)
            );
            let request = request.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(request.starts_with(if expected_method == "GET" {
                "GET /?"
            } else {
                "POST / "
            }));
            assert!(source.parse(&json!({"code":-409})).is_err());
            if kind != "EnncySearcher" {
                assert!(
                    source
                        .parse(&json!({"code":9999,"msg":"secret","data":"A"}))
                        .is_err()
                );
            }
            let payload = source.payload(&question()).unwrap();
            if kind == "EnncySearcher" {
                assert_eq!(payload["title"], question().value);
                assert_eq!(payload["token"], "1234567890");
            }
            if kind == "TiKuHaiSearcher" {
                assert_eq!(payload["key"], "1234567890");
            }
            if kind == "LyCk6Searcher" {
                assert!(source.url.ends_with("1234567890?gpt=0"));
            }
            if kind == "CxSearcher" {
                assert_eq!(source.headers["authorization"], "1234567890");
            }
            if kind == "LemonSearcher" {
                assert_eq!(source.headers["authorization"], "Bearer 1234567890");
            }
        }
        let source = HttpSource::new("EnncySearcher", &json!({"token":"unused"})).unwrap();
        assert!(
            source
                .parse(&json!({"data":{"answer":"非常抱歉，题目搜索不到。"}}))
                .is_err()
        );
    }

    #[test]
    fn model_responses_are_raw_without_fewshots_or_substring_rewriting() {
        for (kind, path, response) in [
            (
                "OpenAISearcher",
                "/v1/chat/completions",
                json!({"choices":[{"message":{"content":"不正确\n解析：北京"}}]}),
            ),
            (
                "OllamaSearcherAPI",
                "/v1/api/generate",
                json!({"response":"不正确\n解析：北京"}),
            ),
        ] {
            let (url, request) = serve(200, response, "");
            let sources = Searchers::new(&[json!({"type":kind,"base_url":format!("{url}/v1/"),"api_key":"secret","model":"test-model","system_prompt":"test-system","prompt":"{{literal}} {type}: {value} {options}"})]).unwrap();
            let result = sources.search(&question());
            assert_eq!(result[0].answer, "不正确\n解析：北京");
            let request = request.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(request.starts_with(&format!("POST {path} HTTP/1.1")));
            let body: Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["model"], "test-model");
            if kind == "OpenAISearcher" {
                assert_eq!(body["messages"].as_array().unwrap().len(), 2);
                assert_eq!(body["messages"][0]["role"], "system");
                assert!(
                    body["messages"][1]["content"]
                        .as_str()
                        .unwrap()
                        .starts_with("{literal} 单选题:")
                );
            } else {
                assert_eq!(body["system"], "test-system");
                assert_eq!(body["stream"], false);
            }
        }
        let mut q = question();
        q.value = "{options}".to_owned();
        assert_eq!(format_prompt("{value}", &q, "A. x").unwrap(), "{options}");
        assert!(format_prompt("{unknown}", &q, "").is_err());
    }

    #[test]
    fn source_errors_are_isolated_and_redacted_without_cookie_persistence() {
        let (first_url, first_request) = serve(
            500,
            json!({"error":"do-not-log-secret"}),
            "Set-Cookie: account_secret=private; Path=/\r\n",
        );
        let (second_url, second_request) = serve(200, json!({"data":"A"}), "");
        let sources = Searchers::new(&[
            json!({"type":"RemovedSearcher"}),
            json!({"type":"RestApiSearcher","url":format!("{first_url}/?token=do-not-log-secret")}),
            json!({"type":"RestApiSearcher","url":second_url}),
        ])
        .unwrap();
        let result = sources.search(&question());
        assert_eq!(result.len(), 3);
        assert!(result[0].error.is_some());
        assert!(result[1].error.is_some());
        assert_eq!(result[2].answer, "A");
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("do-not-log-secret")
        );
        first_request.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            !second_request
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .to_lowercase()
                .contains("cookie:")
        );
        let (url, request) = serve(409, json!({"msg":"private"}), "");
        let sources = Searchers::new(&[json!({"type":"RestApiSearcher","url":url})]).unwrap();
        assert!(sources.search(&question())[0].conflict);
        request.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn invalid_paths_and_environment_secrets_are_safe() {
        assert_eq!(
            resolve_env(&json!({"headers":{"x":"$ENV:PATH"}})).unwrap()["headers"]["x"],
            std::env::var("PATH").unwrap()
        );
        let error = resolve_env(&json!("$ENV:CPASS_TEST_MISSING_SECRET_409BF61")).unwrap_err();
        assert_eq!(error.to_string(), "缺少搜索器环境变量");
        let sources = Searchers::new(&[
            json!({"type":"RestApiSearcher","url":"http://127.0.0.1","a_field":"$.data[secret"}),
        ])
        .unwrap();
        assert!(sources.validate().is_err());
        let result = sources.search(&question());
        assert_eq!(result[0].error.as_deref(), Some("搜索器 JSONPath 无效"));
        let sources = Searchers::new(&[
            json!({"type":"RemovedSearcher"}),
            json!({"type":"EnncySearcher"}),
            json!({"type":"JsonFileSearcher","file_path":"/missing/cpass/question.json"}),
        ])
        .unwrap();
        let error = sources.validate().unwrap_err().to_string();
        assert!(error.contains("UnknownSearcher#1"));
        assert!(error.contains("EnncySearcher#2"));
        assert!(error.contains("JsonFileSearcher#3"));
        assert!(!error.contains("/missing/cpass"));
        Searchers::new(&[]).unwrap().validate().unwrap();
    }
}
