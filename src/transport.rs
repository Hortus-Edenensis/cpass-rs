use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use cookie_store::CookieStore;
use rand::Rng;
use reqwest::blocking::{Client, RequestBuilder, multipart};
use reqwest::{Url, redirect::Policy};
use reqwest_cookie_store::CookieStoreMutex;
use serde_json::{Value, json};

pub type Params = Vec<(String, String)>;

pub struct HttpResponse {
    pub status: u16,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone()).context("response is not UTF-8")
    }

    pub fn json(&self) -> Result<Value> {
        serde_json::from_slice(&self.body).map_err(|_| anyhow::anyhow!("invalid JSON response"))
    }
}

#[derive(Clone)]
pub struct Session {
    client: Client,
    no_redirect: Client,
    cookies: Arc<CookieStoreMutex>,
    user_agent: Arc<RwLock<String>>,
    retries: u32,
    base: Option<Url>,
}

impl Session {
    pub fn new(timeout_secs: u64, retries: u32, base_url: Option<&str>) -> Result<Self> {
        ensure!(
            (1..=300).contains(&timeout_secs),
            "timeout must be between 1 and 300 seconds"
        );
        ensure!(retries <= 5, "GET retries must not exceed five");
        let base = base_url
            .map(Url::parse)
            .transpose()
            .context("invalid fixture base URL")?;
        if let Some(url) = &base {
            ensure!(
                matches!(
                    url.host_str(),
                    Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
                ),
                "fixture base must be localhost"
            );
            ensure!(
                matches!(url.scheme(), "http" | "https"),
                "invalid fixture URL scheme"
            );
            ensure!(
                url.username().is_empty() && url.password().is_none(),
                "fixture URL must not contain credentials"
            );
        }
        let cookies = Arc::new(CookieStoreMutex::new(CookieStore::default()));
        let client = |policy| {
            Client::builder()
                .cookie_provider(cookies.clone())
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert("X-Requested-With", "com.chaoxing.mobile".parse().unwrap());
                    headers
                })
                .timeout(Duration::from_secs(timeout_secs))
                .redirect(policy)
                .build()
                .map_err(|_| anyhow::anyhow!("could not initialize HTTP client"))
        };
        Ok(Self {
            client: client(Policy::limited(10))?,
            no_redirect: client(Policy::none())?,
            cookies,
            user_agent: Arc::new(RwLock::new(generate_mobile_ua())),
            retries,
            base,
        })
    }

    pub fn endpoint(&self, raw: &str) -> Result<String> {
        let url = Url::parse(raw).context("invalid endpoint URL")?;
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "endpoint must not contain credentials"
        );
        if let Some(base) = &self.base {
            let mut mapped = base.clone();
            mapped.set_path(url.path());
            mapped.set_query(url.query());
            mapped.set_fragment(None);
            return Ok(mapped.into());
        }
        ensure!(url.scheme() == "https", "HTTPS is required");
        Ok(url.into())
    }

    pub fn imei(&self) -> String {
        self.user_agent
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .rsplit_once("(@Kalimdor)_")
            .expect("session user agent has a validated device identifier")
            .1
            .to_owned()
    }

    fn with_params(&self, raw: &str, params: &[(String, String)]) -> Result<Url> {
        let mut url = Url::parse(&self.endpoint(raw)?)?;
        if !params.is_empty() {
            let query = encode_params(params);
            let query = match url.query().filter(|query| !query.is_empty()) {
                Some(existing) => format!("{existing}&{query}"),
                None => query,
            };
            url.set_query(Some(&query));
        }
        Ok(url)
    }

    pub fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse> {
        self.get_with_headers(url, params, &BTreeMap::new())
    }

    pub fn get_with_headers(
        &self,
        url: &str,
        params: &[(String, String)],
        headers: &BTreeMap<String, String>,
    ) -> Result<HttpResponse> {
        self.send(
            url,
            self.client.get(self.with_params(url, params)?),
            headers,
            true,
            true,
        )
    }

    pub fn get_no_redirect(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse> {
        self.send(
            url,
            self.no_redirect.get(self.with_params(url, params)?),
            &BTreeMap::new(),
            false,
            false,
        )
    }

    pub fn post_form(
        &self,
        url: &str,
        params: &[(String, String)],
        form: &BTreeMap<String, String>,
    ) -> Result<HttpResponse> {
        self.send(
            url,
            self.no_redirect
                .post(self.with_params(url, params)?)
                .form(form),
            &BTreeMap::new(),
            false,
            true,
        )
    }

    pub fn post_form_no_redirect(
        &self,
        url: &str,
        params: &[(String, String)],
        form: &BTreeMap<String, String>,
    ) -> Result<HttpResponse> {
        self.send(
            url,
            self.no_redirect
                .post(self.with_params(url, params)?)
                .form(form),
            &BTreeMap::new(),
            false,
            false,
        )
    }

    pub fn post_json(
        &self,
        url: &str,
        body: &Value,
        headers: &BTreeMap<String, String>,
    ) -> Result<HttpResponse> {
        self.send(
            url,
            self.no_redirect.post(self.endpoint(url)?).json(body),
            headers,
            false,
            true,
        )
    }

    pub fn post_multipart(
        &self,
        url: &str,
        params: &[(String, String)],
        fields: &BTreeMap<String, String>,
        file_field: &str,
        file: &Path,
    ) -> Result<HttpResponse> {
        let metadata = fs::metadata(file).context("cannot read upload image")?;
        ensure!(
            metadata.is_file() && metadata.len() > 0 && metadata.len() <= 20 * 1024 * 1024,
            "image must be a nonempty file under 20 MiB"
        );
        let image = fs::read(file).context("cannot read upload image")?;
        ensure!(
            image.starts_with(&[0xff, 0xd8, 0xff]),
            "face upload requires a JPEG image"
        );
        let part = multipart::Part::bytes(image)
            .file_name(format!("{}.jpg", timestamp()))
            .mime_str("image/jpeg")?;
        let mut form = multipart::Form::new().part(file_field.to_owned(), part);
        for (name, value) in fields {
            form = form.text(name.clone(), value.clone());
        }
        self.send(
            url,
            self.no_redirect
                .post(self.with_params(url, params)?)
                .multipart(form),
            &BTreeMap::new(),
            false,
            true,
        )
    }

    fn send(
        &self,
        raw_url: &str,
        mut request: RequestBuilder,
        headers: &BTreeMap<String, String>,
        retry_get: bool,
        inspect_redirect: bool,
    ) -> Result<HttpResponse> {
        if !headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("user-agent"))
        {
            let user_agent = self
                .user_agent
                .read()
                .map_err(|_| anyhow::anyhow!("session device unavailable"))?;
            request = request.header(reqwest::header::USER_AGENT, user_agent.as_str());
        }
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let mut request = Some(request);
        let attempts = if retry_get { self.retries + 1 } else { 1 };
        for attempt in 0..attempts {
            let builder = if retry_get {
                request
                    .as_ref()
                    .context("missing GET request")?
                    .try_clone()
                    .context("GET request cannot be replayed")?
            } else {
                request.take().context("request was already sent")?
            };
            let response = builder.send();
            match response {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let url = response.url().to_string();
                    let headers = response
                        .headers()
                        .iter()
                        .filter_map(|(key, value)| {
                            value.to_str().ok().map(|v| (key.to_string(), v.to_owned()))
                        })
                        .collect();
                    let body = response
                        .bytes()
                        .map_err(|_| anyhow::anyhow!("could not read response body"))?
                        .to_vec();
                    let response = HttpResponse {
                        status,
                        url,
                        headers,
                        body,
                    };
                    self.check_action(raw_url, &response, inspect_redirect)?;
                    ensure!(status < 400, "HTTP request failed with status {status}");
                    return Ok(response);
                }
                Err(error)
                    if retry_get
                        && (error.is_connect() || error.is_timeout())
                        && attempt + 1 < attempts =>
                {
                    std::thread::sleep(Duration::from_millis(250 * u64::from(attempt + 1)));
                }
                Err(_) => bail!("HTTP transport failed; request outcome is unconfirmed"),
            }
        }
        bail!("GET retry limit reached")
    }

    fn check_action(
        &self,
        raw_url: &str,
        response: &HttpResponse,
        inspect_redirect: bool,
    ) -> Result<()> {
        let path = Url::parse(&response.url)?.path().to_owned();
        if path.ends_with("/antispiderShowVerify.ac") {
            bail!("action-required: captcha challenge; supply the displayed verification code");
        }
        let is_html = response
            .headers
            .get("content-type")
            .is_some_and(|v| v.starts_with("text/html"));
        if is_html {
            let text = String::from_utf8_lossy(&response.body);
            if text.contains("grayBg") && text.contains("/knowledge/startface") {
                bail!(
                    "action-required: face verification; provide your own image and the displayed course context"
                );
            }
        }
        let requested_path = Url::parse(raw_url)?.path().to_owned();
        if inspect_redirect && path.ends_with("/login") && requested_path != path {
            bail!("action-required: session expired; log in again");
        }
        Ok(())
    }

    pub fn clear_cookies(&self) -> Result<()> {
        *self
            .cookies
            .lock()
            .map_err(|_| anyhow::anyhow!("cookie store unavailable"))? = CookieStore::default();
        Ok(())
    }

    pub fn import_cookie(&self, cookie: &str) -> Result<()> {
        let mut store = self
            .cookies
            .lock()
            .map_err(|_| anyhow::anyhow!("cookie store unavailable"))?;
        let mut imported = store.clone();
        let origin = Url::parse("https://passport2.chaoxing.com/")?;
        for part in cookie.split(';').map(str::trim).filter(|v| !v.is_empty()) {
            let (name, value) = part.split_once('=').context("invalid legacy cookie")?;
            ensure!(
                !name.is_empty()
                    && name.bytes().all(
                        |b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'$')
                    )
                    && !value.contains(['\r', '\n']),
                "invalid legacy cookie"
            );
            imported
                .parse(
                    &format!("{name}={value}; Domain=chaoxing.com; Path=/; Secure"),
                    &origin,
                )
                .map_err(|_| anyhow::anyhow!("invalid legacy cookie"))?;
        }
        *store = imported;
        Ok(())
    }

    pub fn cookie_value(&self, name: &str) -> Option<String> {
        let store = self.cookies.lock().ok()?;
        let origin = Url::parse("https://passport2.chaoxing.com/").ok()?;
        store
            .get_request_values(&origin)
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_owned())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let user_agent = self
            .user_agent
            .read()
            .map_err(|_| anyhow::anyhow!("session device unavailable"))?;
        let mut bytes = Vec::new();
        let store = self
            .cookies
            .lock()
            .map_err(|_| anyhow::anyhow!("cookie store unavailable"))?;
        cookie_store::serde::json::save_incl_expired_and_nonpersistent(&store, &mut bytes)
            .map_err(|_| anyhow::anyhow!("could not serialize cookies"))?;
        drop(store);
        let cookies: Value = serde_json::from_slice(&bytes)?;
        let data = serde_json::to_vec_pretty(
            &json!({"version": 1, "cookies": cookies, "user_agent": &*user_agent}),
        )?;
        drop(user_agent);
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).context("cannot create session directory")?;
        let temporary = parent.join(format!(
            ".cpass-session-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .context("cannot create session file")?;
            file.write_all(&data)?;
            file.sync_all()?;
            fs::rename(&temporary, path).context("cannot replace session file")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn load(&self, path: &Path) -> Result<()> {
        let mut data = String::new();
        fs::File::open(path)
            .context("cannot open session file")?
            .take(8 * 1024 * 1024)
            .read_to_string(&mut data)
            .context("cannot read session file")?;
        let value: Value =
            serde_json::from_str(&data).map_err(|_| anyhow::anyhow!("invalid session file"))?;
        let saved_user_agent = value
            .get("user_agent")
            .map(validate_user_agent)
            .transpose()?;
        if let Some(cookies) = value.get("cookies") {
            ensure!(
                value.get("version").and_then(Value::as_u64) == Some(1),
                "unsupported session format"
            );
            let bytes = serde_json::to_vec(cookies)?;
            let store = cookie_store::serde::json::load(bytes.as_slice())
                .map_err(|_| anyhow::anyhow!("invalid scoped cookies"))?;
            let mut user_agent = self
                .user_agent
                .write()
                .map_err(|_| anyhow::anyhow!("session device unavailable"))?;
            let mut cookies = self
                .cookies
                .lock()
                .map_err(|_| anyhow::anyhow!("cookie store unavailable"))?;
            *cookies = store;
            if let Some(saved) = saved_user_agent {
                *user_agent = saved;
            }
            return Ok(());
        }
        let legacy = value.get("ck").context("session contains no cookies")?;
        let mut user_agent = self
            .user_agent
            .write()
            .map_err(|_| anyhow::anyhow!("session device unavailable"))?;
        match legacy {
            Value::String(cookie) => self.import_cookie(cookie),
            Value::Object(cookies) => {
                let mut values = Vec::new();
                for (name, value) in cookies {
                    values.push(format!(
                        "{name}={}",
                        value.as_str().context("invalid legacy cookie value")?
                    ));
                }
                self.import_cookie(&values.join(";"))
            }
            _ => bail!("invalid legacy cookie format"),
        }?;
        if let Some(saved) = saved_user_agent {
            *user_agent = saved;
        }
        Ok(())
    }
}

fn validate_user_agent(value: &Value) -> Result<String> {
    let ua = value
        .as_str()
        .filter(|ua| {
            ua.len() <= 1024
                && ua.starts_with("Dalvik/2.1.0 (Linux; U; Android ")
                && ua
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() || byte == b' ')
                && reqwest::header::HeaderValue::from_str(ua).is_ok()
                && ua.split("(@Kalimdor)_").count() == 2
                && ua.rsplit_once("(@Kalimdor)_").is_some_and(|(_, imei)| {
                    imei.len() == 32 && imei.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        })
        .context("invalid session user agent")?;
    Ok(ua.to_owned())
}

pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn inf_enc_sign(params: &[(String, String)]) -> Params {
    let mut signed = params.to_vec();
    let signature = md5::compute(format!("{}&DESKey=Z(AfY@XS", encode_params(params)));
    signed.push(("inf_enc".into(), format!("{signature:x}")));
    signed
}

fn encode_params(params: &[(String, String)]) -> String {
    // Python quote_plus keeps '~' and escapes '*'; signatures depend on these exact bytes.
    fn quote(value: &str) -> String {
        let mut quoted = String::new();
        for byte in value.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'~' => {
                    quoted.push(char::from(byte))
                }
                b' ' => quoted.push('+'),
                _ => quoted.push_str(&format!("%{byte:02X}")),
            }
        }
        quoted
    }
    params
        .iter()
        .map(|(key, value)| format!("{}={}", quote(key), quote(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn encrypt_login(value: &str) -> Result<String> {
    let key = b"u2oh6Vu^HWe4_AES";
    let encrypted = cbc::Encryptor::<aes::Aes128>::new(key.into(), key.into())
        .encrypt_padded_vec_mut::<Pkcs7>(value.as_bytes());
    Ok(STANDARD.encode(encrypted))
}

pub fn mobile_ua() -> String {
    static UA: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    UA.get_or_init(generate_mobile_ua).clone()
}

fn generate_mobile_ua() -> String {
    let imei = random_hex(16);
    let model = format!("MI{}", rand::rng().random_range(10..=12));
    let sign = format!(
        "(schild:ipL$TkeiEmfy1gTXb2XHrdLN0a@7c^vu) (device:{model}) Language/zh_CN com.chaoxing.mobile/ChaoXingStudy_3_6.3.9_android_phone_10824_250 (@Kalimdor)_{imei}"
    );
    format!(
        "Dalvik/2.1.0 (Linux; U; Android {}; {model} Build/SKQ1.211006.001) (schild:{:x}) (device:{model}) Language/zh_CN com.chaoxing.mobile/ChaoXingStudy_3_6.3.9_android_phone_10824_250 (@Kalimdor)_{imei}",
        rand::rng().random_range(9..=12),
        md5::compute(sign)
    )
}

pub fn imei() -> String {
    mobile_ua()
        .split("(@Kalimdor)_")
        .nth(1)
        .unwrap_or_default()
        .to_owned()
}

fn random_hex(bytes: usize) -> String {
    (0..bytes)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect()
}

pub fn exam_signature(uid: u64, qid: u64) -> Params {
    let mut rng = rand::rng();
    signature(
        uid,
        qid,
        timestamp(),
        rng.random_range(0..9),
        rng.random_range(0..9),
        &random_hex(16),
        &random_hex(4),
        rng.random::<f64>(),
        rng.random_range(100..=1000),
        rng.random_range(100..=1000),
    )
}

#[allow(clippy::too_many_arguments)]
fn signature(
    uid: u64,
    qid: u64,
    ts: u64,
    r1: u32,
    r2: u32,
    hex32: &str,
    hex8: &str,
    rd: f64,
    x: u32,
    y: u32,
) -> Params {
    let ts = ts.to_string();
    let qid_str = if qid == 0 {
        String::new()
    } else {
        qid.to_string()
    };
    let seed = format!("{hex32}{}{r1}{r2}{qid_str}", &ts[4..]);
    let hash = seed.bytes().fold(0u32, |value, ch| {
        value.wrapping_mul(31).wrapping_add(u32::from(ch))
    });
    let salt = format!("{r1}{r2}{}", (hash & 0x7fff_ffff) % 10);
    let plain = if qid == 0 {
        format!("{uid}|{salt}")
    } else {
        format!("{uid}_{qid}|{salt}")
    };
    let digits: String = plain.bytes().map(|ch| ch.to_string()).collect();
    let b = digits.len() / 5;
    let c: u64 = [b, b * 2, b * 3, b * 4]
        .into_iter()
        .fold(0, |value, index| {
            value * 10 + u64::from(digits.as_bytes()[index] - b'0')
        });
    let d = plain.len() as u64 / 2 + 1;
    let mut e = (c * digits[..10].parse::<u64>().unwrap() + d) % 0x7fff_ffff;
    let value = format!("({x}|{y})");
    let mut pos = String::new();
    for ch in value.bytes() {
        pos.push_str(&format!("{:02x}", u64::from(ch) ^ (e * 255 / 0x7fff_ffff)));
        e = (c * e + d) % 0x7fff_ffff;
    }
    pos.push_str(hex8);
    vec![
        ("pos".into(), pos),
        ("rd".into(), rd.to_string()),
        ("value".into(), value),
        ("_edt".into(), format!("{ts}{salt}")),
    ]
}

#[cfg(test)]
pub(crate) fn test_server(
    responses: Vec<(&'static str, Vec<u8>)>,
) -> (String, std::thread::JoinHandle<Vec<Vec<u8>>>) {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (headers, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0, "request ended before its body");
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]);
                    let length = head
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n", body.len()).as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            requests.push(request);
        }
        requests
    });
    (url, server)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_device_is_shared_restored_validated_and_allows_desktop_override() {
        let path =
            std::env::temp_dir().join(format!("cpass-device-{:016x}.json", rand::random::<u64>()));
        let (base, server) = test_server(vec![("", b"{}".to_vec()); 3]);
        let initial = Session::new(2, 0, Some(&base)).unwrap();
        initial
            .import_cookie("UID=123; token=PRIVATE_COOKIE")
            .unwrap();
        initial.save(&path).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let ua = saved["user_agent"].as_str().unwrap();
        assert!(ua.ends_with(&initial.imei()));
        let restored = Session::new(2, 0, Some(&base)).unwrap();
        assert_ne!(restored.imei(), initial.imei());
        let shared = restored.clone();
        restored.load(&path).unwrap();
        assert_eq!(restored.imei(), initial.imei());
        assert_eq!(shared.imei(), initial.imei());
        shared
            .get("https://mooc1.chaoxing.com/mobile", &[])
            .unwrap();
        restored
            .get_with_headers(
                "https://passport2.chaoxing.com/login",
                &[],
                &BTreeMap::from([("uSeR-aGeNt".into(), "Mozilla/5.0 desktop-fixture".into())]),
            )
            .unwrap();
        shared
            .get("https://mooc1.chaoxing.com/mobile-again", &[])
            .unwrap();
        let requests = server.join().unwrap();
        for (index, request) in requests.iter().enumerate() {
            let request = String::from_utf8_lossy(request);
            let headers: Vec<_> = request
                .lines()
                .filter_map(|line| line.split_once(':'))
                .filter(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
                .map(|(_, value)| value.trim())
                .collect();
            assert_eq!(
                headers,
                vec![if index == 1 {
                    "Mozilla/5.0 desktop-fixture"
                } else {
                    ua
                }]
            );
        }
        for invalid in [
            Value::Null,
            json!(format!("{ua}\r\nPRIVATE_HEADER: secret")),
            json!(ua.replace(&initial.imei(), "PRIVATE_BAD_IMEI")),
            json!(format!(
                "Dalvik/2.1.0 (Linux; U; Android {} (@Kalimdor)_{}",
                "x".repeat(1024),
                initial.imei()
            )),
        ] {
            fs::write(
                &path,
                json!({"ck":"UID=999; token=PRIVATE_REPLACEMENT", "user_agent":invalid})
                    .to_string(),
            )
            .unwrap();
            let error = restored.load(&path).unwrap_err().to_string();
            assert_eq!(error, "invalid session user agent");
            assert_eq!(shared.imei(), initial.imei());
            assert_eq!(shared.cookie_value("UID").as_deref(), Some("123"));
        }
        let other_ua = generate_mobile_ua();
        fs::write(
            &path,
            json!({"version":1,"cookies":"invalid","user_agent":other_ua}).to_string(),
        )
        .unwrap();
        assert!(restored.load(&path).is_err());
        assert_eq!(shared.imei(), initial.imei());
        assert_eq!(
            shared.cookie_value("token").as_deref(),
            Some("PRIVATE_COOKIE")
        );

        let legacy_reader = Session::new(2, 0, None).unwrap();
        let legacy_imei = legacy_reader.imei();
        let mut old_v1 = saved;
        old_v1.as_object_mut().unwrap().remove("user_agent");
        for old in [old_v1, json!({"ck":"UID=123"}), json!({"ck":{"UID":"123"}})] {
            fs::write(&path, old.to_string()).unwrap();
            legacy_reader.load(&path).unwrap();
            assert_eq!(legacy_reader.imei(), legacy_imei);
            assert_eq!(legacy_reader.cookie_value("UID").as_deref(), Some("123"));
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn transport_timeouts_bound_get_retries_and_never_replay_exam_start() {
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Instant;

        for no_redirect in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let server_stop = stop.clone();
            let server = std::thread::spawn(move || {
                let mut connections = Vec::new();
                loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(Duration::from_secs(10)))
                                .unwrap();
                            let mut request = Vec::new();
                            let mut buffer = [0; 4096];
                            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                                let count = stream.read(&mut buffer).unwrap();
                                assert!(count > 0);
                                request.extend_from_slice(&buffer[..count]);
                            }
                            assert!(request.starts_with(b"GET /exam-start?"));
                            // Keep each socket open without headers so the client deadline fires.
                            connections.push(stream);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if server_stop.load(Ordering::Relaxed) {
                                break connections.len();
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("local timeout fixture failed: {error}"),
                    }
                }
            });
            let session = Session::new(1, 2, Some(&base)).unwrap();
            let start = Instant::now();
            let params = [("token".into(), "PRIVATE_TIMEOUT_TOKEN".into())];
            let result = if no_redirect {
                session.get_no_redirect("https://mooc1.chaoxing.com/exam-start", &params)
            } else {
                session.get("https://mooc1.chaoxing.com/exam-start", &params)
            };
            stop.store(true, Ordering::Relaxed);
            let requests = server.join().unwrap();
            assert_eq!(requests, if no_redirect { 1 } else { 3 });
            assert!(start.elapsed() < Duration::from_secs(30));
            let error = result.err().unwrap().to_string();
            assert!(error.contains("outcome is unconfirmed"));
            assert!(!error.contains("PRIVATE_TIMEOUT_TOKEN"));
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let session = Session::new(1, 2, Some(&base)).unwrap();
        let started = Instant::now();
        assert!(
            session
                .get("https://mooc1.chaoxing.com/refused", &[])
                .is_err()
        );
        assert!(started.elapsed() >= Duration::from_millis(700));
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    #[test]
    fn legacy_session_load_is_scoped_and_expired_roundtrip_cookies_are_not_sent() {
        let path = std::env::temp_dir().join(format!(
            "cpass-legacy-test-{:016x}.json",
            rand::random::<u64>()
        ));
        for legacy in [
            json!({"ck":"UID=123; token=PRIVATE_LEGACY_TOKEN"}),
            json!({"ck":{"UID":"123","token":"PRIVATE_LEGACY_TOKEN"}}),
        ] {
            fs::write(&path, legacy.to_string()).unwrap();
            let (base, server) = test_server(vec![("", b"{}".to_vec())]);
            let session = Session::new(2, 0, Some(&base)).unwrap();
            session.load(&path).unwrap();
            assert_eq!(session.cookie_value("UID").as_deref(), Some("123"));
            assert_eq!(
                session.cookie_value("token").as_deref(),
                Some("PRIVATE_LEGACY_TOKEN")
            );
            let store = session.cookies.lock().unwrap();
            for other in ["https://example.org/", "http://mooc1.chaoxing.com/"] {
                assert_eq!(
                    store
                        .get_request_values(&Url::parse(other).unwrap())
                        .count(),
                    0
                );
            }
            drop(store);
            session
                .get("https://example.org/cookie-check", &[])
                .unwrap();
            let request = String::from_utf8(server.join().unwrap().remove(0)).unwrap();
            assert!(!request.to_ascii_lowercase().contains("\r\ncookie:"));
            assert!(!request.contains("PRIVATE_LEGACY_TOKEN"));
        }
        fs::remove_file(&path).unwrap();

        let (base, server) = test_server(vec![("", b"{}".to_vec())]);
        let session = Session::new(2, 0, Some(&base)).unwrap();
        let origin = Url::parse(&base).unwrap();
        {
            let mut store = session.cookies.lock().unwrap();
            store
                .parse("kept=active; Max-Age=3600; Path=/", &origin)
                .unwrap();
            store
                .parse("expiring=PRIVATE_EXPIRED_TOKEN; Max-Age=1; Path=/", &origin)
                .unwrap();
        }
        session.save(&path).unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("PRIVATE_EXPIRED_TOKEN")
        );
        std::thread::sleep(Duration::from_secs(2));
        let restored = Session::new(2, 0, Some(&base)).unwrap();
        restored.load(&path).unwrap();
        restored
            .get("https://mooc1.chaoxing.com/cookie-check", &[])
            .unwrap();
        let request = String::from_utf8(server.join().unwrap().remove(0)).unwrap();
        assert!(request.contains("kept=active"));
        assert!(!request.contains("expiring=") && !request.contains("PRIVATE_EXPIRED_TOKEN"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn response_challenges_report_manual_actions_without_response_secrets() {
        for (path, headers, body, action) in [
            (
                "antispiderShowVerify.ac",
                "Content-Type: text/html\r\n",
                "PRIVATE_CHALLENGE_BODY",
                "captcha challenge",
            ),
            (
                "course",
                "Content-Type: text/html\r\n",
                "<div class='grayBg'><a href='/knowledge/startface'>PRIVATE_CHALLENGE_BODY</a></div>",
                "face verification",
            ),
        ] {
            let (base, server) = test_server(vec![(headers, body.as_bytes().to_vec())]);
            let session = Session::new(2, 2, Some(&base)).unwrap();
            let error = session
                .get(
                    &format!("https://mooc1.chaoxing.com/{path}"),
                    &[("token".into(), "PRIVATE_QUERY_TOKEN".into())],
                )
                .err()
                .unwrap()
                .to_string();
            assert!(error.starts_with("action-required:") && error.contains(action));
            assert!(!error.contains("PRIVATE_"));
            assert_eq!(server.join().unwrap().len(), 1);
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (path, response) in [
                (
                    "/course",
                    "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                ),
                (
                    "/login",
                    "HTTP/1.1 200 OK\r\nContent-Length: 18\r\nConnection: close\r\n\r\nPRIVATE_LOGIN_BODY",
                ),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                }
                assert!(String::from_utf8_lossy(&request).starts_with(&format!("GET {path}")));
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let session = Session::new(2, 2, Some(&base)).unwrap();
        let error = session
            .get("https://mooc1.chaoxing.com/course", &[])
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("action-required: session expired"));
        assert!(!error.contains("PRIVATE_LOGIN_BODY"));
        server.join().unwrap();
    }

    #[test]
    fn signature_matches_python_golden() {
        let params = signature(
            123456,
            1234,
            1700000000123,
            1,
            2,
            &"a".repeat(32),
            &"b".repeat(8),
            0.5,
            100,
            200,
        );
        assert_eq!(params[0].1, "a191d87c930b2098eabbbbbbbb");
        assert_eq!(params[2].1, "(100|200)");
        assert_eq!(params[3].1, "1700000000123121");
        assert_eq!(
            signature(
                123456,
                0,
                1700000000123,
                1,
                2,
                &"a".repeat(32),
                &"b".repeat(8),
                0.5,
                100,
                200
            )[0]
            .1,
            "967044f05b9a21c49ebbbbbbbb"
        );
    }

    #[test]
    fn login_cipher_matches_openssl_golden() {
        assert_eq!(
            encrypt_login("13800000000").unwrap(),
            "ZBjZ8C7FsVyCJ12TKWWRSQ=="
        );
        assert_eq!(mobile_ua(), mobile_ua());
        assert_eq!(imei().len(), 32);
        let params = vec![
            ("id".into(), "123".into()),
            ("view".into(), "json".into()),
            ("_time".into(), "1700000000123".into()),
        ];
        assert_eq!(
            inf_enc_sign(&params).last().unwrap().1,
            "769b85674d5f3e38d1bb7ae3ea3064a8"
        );
        let special = vec![("a".into(), "~* 中".into())];
        assert_eq!(encode_params(&special), "a=~%2A+%E4%B8%AD");
        assert_eq!(
            inf_enc_sign(&special).last().unwrap().1,
            "225c1f7135d7a9db6a7ee608a7d25e0a"
        );
        let session = Session::new(2, 0, Some("http://127.0.0.1:9999")).unwrap();
        assert_eq!(
            session
                .with_params("https://mooc1.chaoxing.com/test?existing=x=y", &special)
                .unwrap()
                .query(),
            Some("existing=x=y&a=~%2A+%E4%B8%AD")
        );
    }

    #[test]
    fn cookies_remain_domain_scoped() {
        let session = Session::new(2, 0, Some("http://127.0.0.1:9999")).unwrap();
        session.import_cookie("UID=123; token=a=b").unwrap();
        assert_eq!(session.cookie_value("UID").as_deref(), Some("123"));
        let store = session.cookies.lock().unwrap();
        assert_eq!(
            store
                .get_request_values(&Url::parse("https://example.org/").unwrap())
                .count(),
            0
        );
        assert_eq!(
            store
                .get_request_values(&Url::parse("http://mooc1.chaoxing.com/").unwrap())
                .count(),
            0
        );
        drop(store);
        assert!(session.import_cookie("UID=123\r\nCookie: other").is_err());
    }

    #[test]
    fn session_roundtrip_is_scoped_and_owner_only() {
        let path = std::env::temp_dir().join(format!(
            "cpass-session-test-{:016x}.json",
            rand::random::<u64>()
        ));
        let session = Session::new(2, 0, None).unwrap();
        session.import_cookie("UID=123; token=a=b").unwrap();
        session.save(&path).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("password"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let restored = Session::new(2, 0, None).unwrap();
        restored.load(&path).unwrap();
        assert_eq!(restored.cookie_value("token").as_deref(), Some("a=b"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn multipart_upload_sends_the_original_jpeg_and_fields() {
        let mut image = vec![0xff, 0xd8, 0xff, 0xdb, 0x00, 0x43, 0x00];
        image.extend_from_slice(&[1; 64]);
        image.extend_from_slice(&[
            0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00,
        ]);
        for class in [0x00, 0x10] {
            image.extend_from_slice(&[0xff, 0xc4, 0x00, 0x14, class, 0x01]);
            image.extend_from_slice(&[0; 15]);
            image.push(0);
        }
        image.extend_from_slice(&[
            0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00, 0x3f, 0xff, 0xd9,
        ]);
        let path = std::env::temp_dir().join(format!(
            "cpass-jpeg-test-{:016x}.jpg",
            rand::random::<u64>()
        ));
        fs::write(&path, &image).unwrap();
        let (url, server) = test_server(vec![(
            "Content-Type: application/json\r\n",
            br#"{"result":true,"objectId":"fixture-image"}"#.to_vec(),
        )]);
        let session = Session::new(2, 5, Some(&url)).unwrap();
        let response = session
            .post_multipart(
                "https://pan-yz.chaoxing.com/upload",
                &[
                    ("uploadtype".into(), "face".into()),
                    ("puid".into(), "123".into()),
                ],
                &BTreeMap::from([("context".into(), "user-provided".into())]),
                "file",
                &path,
            )
            .unwrap();
        assert_eq!(response.json().unwrap()["objectId"], "fixture-image");
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        let text = String::from_utf8_lossy(request);
        assert!(text.starts_with("POST /upload?uploadtype=face&puid=123 "));
        assert!(text.contains("name=\"file\";") && text.contains("Content-Type: image/jpeg"));
        assert!(text.contains("name=\"context\"\r\n\r\nuser-provided"));
        assert!(request.windows(image.len()).any(|bytes| bytes == image));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn post_failure_and_redirect_do_not_replay_or_expose_secrets() {
        use std::net::TcpListener;
        for status in [503, 307] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 8192];
                let count = stream.read(&mut request).unwrap();
                assert!(String::from_utf8_lossy(&request[..count]).starts_with("POST /commit?"));
                stream.write_all(format!("HTTP/1.1 {status} fixture\r\nLocation: /retry\r\nContent-Length: 17\r\nConnection: close\r\n\r\nserver-secret-key").as_bytes()).unwrap();
                drop(stream);
                listener.set_nonblocking(true).unwrap();
                std::thread::sleep(Duration::from_millis(150));
                assert!(listener.accept().is_err(), "POST was replayed");
            });
            let session = Session::new(2, 5, Some(&base)).unwrap();
            let result = session.post_form(
                "https://mooc1.chaoxing.com/commit",
                &[("token".into(), "private-query-secret".into())],
                &BTreeMap::from([("password".into(), "private-body-secret".into())]),
            );
            if status == 503 {
                let error = result.err().unwrap().to_string();
                assert!(error.contains("503"));
                assert!(!error.contains("private") && !error.contains("server-secret"));
            } else {
                assert_eq!(result.unwrap().status, 307);
            }
            server.join().unwrap();
        }
    }
}
