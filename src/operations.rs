use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Command,
    Config,
    Login,
    Parse,
    Resolve,
    Course,
    Work,
    Exam,
    Media,
    Notify,
    Ocr,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Started,
    Succeeded,
    Failed,
    Incomplete,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Configuration,
    Network,
    Receipt,
    Timeout,
    Incomplete,
    Operation,
}

/// Free-form responses and account fields never enter the diagnostic format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub version: [u16; 3],
    pub timestamp: u64,
    pub stage: Stage,
    pub outcome: Outcome,
    pub error: Option<ErrorCode>,
    pub matched: usize,
    pub submitted: usize,
    pub incomplete: usize,
}

impl Event {
    pub fn new(stage: Stage, outcome: Outcome, error: Option<ErrorCode>) -> Self {
        Self {
            version: [
                env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or_default(),
                env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or_default(),
                env!("CARGO_PKG_VERSION_PATCH").parse().unwrap_or_default(),
            ],
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            stage,
            outcome,
            error,
            matched: 0,
            submitted: 0,
            incomplete: 0,
        }
    }
}

fn private_file(path: &Path, append: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true);
    if append {
        ensure!(
            !fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()),
            "日志目标不能是符号链接"
        );
        options.append(true).create(true);
    } else {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|_| anyhow!("无法打开诊断输出"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| anyhow!("无法限制诊断输出权限"))?;
    }
    Ok(file)
}

pub fn append_log(path: &Path, event: &Event) -> Result<()> {
    let mut line = serde_json::to_vec(event)?;
    line.push(b'\n');
    private_file(path, true)?
        .write_all(&line)
        .map_err(|_| anyhow!("无法写入诊断日志"))
}

pub fn diagnostic_bundle(log: &Path, output: &Path) -> Result<()> {
    let input = File::open(log).map_err(|_| anyhow!("无法读取诊断日志"))?;
    ensure!(
        input.metadata()?.len() <= 4 * 1024 * 1024,
        "诊断日志超过 4 MiB"
    );
    let mut data = String::new();
    input.take(4 * 1024 * 1024 + 1).read_to_string(&mut data)?;
    ensure!(data.len() <= 4 * 1024 * 1024, "诊断日志超过 4 MiB");
    let mut events = Vec::new();
    let mut discarded = 0;
    for line in data.lines() {
        match serde_json::from_str::<Event>(line) {
            Ok(event) => events.push(event),
            Err(_) => discarded += 1,
        }
    }
    let bundle = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "events": events,
        "discarded_lines": discarded,
    });
    let mut file = private_file(output, false)?;
    file.write_all(&serde_json::to_vec_pretty(&bundle)?)?;
    file.sync_all()?;
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub gotify: Option<GotifyConfig>,
    pub mqtt: Option<MqttConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GotifyConfig {
    pub url: String,
    pub token_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttConfig {
    pub broker: String,
    pub topic: String,
    #[serde(default)]
    pub username_env: Option<String>,
    #[serde(default)]
    pub password_env: Option<String>,
    #[serde(default)]
    pub allow_plaintext: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NotificationReceipt {
    pub provider: &'static str,
    pub accepted: bool,
    pub error: Option<ErrorCode>,
}

pub fn notify(config: &NotificationConfig, event: &Event) -> Vec<NotificationReceipt> {
    if !config.enabled {
        return vec![];
    }
    let mut receipts = Vec::new();
    let mut record = |provider, result: std::result::Result<(), ErrorCode>| {
        receipts.push(NotificationReceipt {
            provider,
            accepted: result.is_ok(),
            error: result.err(),
        });
    };
    if let Some(gotify) = &config.gotify {
        record("gotify", send_gotify(gotify, event));
    }
    if let Some(mqtt) = &config.mqtt {
        record("mqtt", send_mqtt(mqtt, event));
    }
    receipts
}

fn secret(name: &str) -> std::result::Result<String, ErrorCode> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(ErrorCode::Configuration);
    }
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or(ErrorCode::Configuration)
}

fn gotify_url(raw: &str) -> std::result::Result<reqwest::Url, ErrorCode> {
    let mut url = reqwest::Url::parse(raw).map_err(|_| ErrorCode::Configuration)?;
    let loopback = url.host_str().is_some_and(|h| {
        h == "localhost"
            || h.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !((url.scheme() == "https") || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ErrorCode::Configuration);
    }
    let path = format!("{}/message", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

fn send_gotify(config: &GotifyConfig, event: &Event) -> std::result::Result<(), ErrorCode> {
    let token = secret(&config.token_env)?;
    gotify_request(gotify_url(&config.url)?, &token, event)
}

fn gotify_request(
    url: reqwest::Url,
    token: &str,
    event: &Event,
) -> std::result::Result<(), ErrorCode> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ErrorCode::Network)?;
    let response = client
        .post(url)
        .header("X-Gotify-Key", token)
        .json(&serde_json::json!({
            "title": "cpass",
            "message": serde_json::to_string(event).map_err(|_| ErrorCode::Operation)?,
            "priority": 5,
        }))
        .send()
        .map_err(|_| ErrorCode::Network)?;
    if !response.status().is_success() {
        return Err(ErrorCode::Receipt);
    }
    let mut body = Vec::new();
    response
        .take(65537)
        .read_to_end(&mut body)
        .map_err(|_| ErrorCode::Network)?;
    if body.len() > 65536 {
        return Err(ErrorCode::Receipt);
    }
    let receipt: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| ErrorCode::Receipt)?;
    if receipt
        .get("id")
        .and_then(|v| v.as_u64())
        .is_some_and(|id| id > 0)
    {
        Ok(())
    } else {
        Err(ErrorCode::Receipt)
    }
}

fn mqtt_string(bytes: &mut Vec<u8>, value: &str) -> std::result::Result<(), ErrorCode> {
    if value.len() > u16::MAX as usize || value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(ErrorCode::Configuration);
    }
    bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn mqtt_packet(
    stream: &mut TcpStream,
    kind: u8,
    data: &[u8],
) -> std::result::Result<(), ErrorCode> {
    let mut header = vec![kind];
    let mut length = data.len();
    loop {
        let byte = (length % 128) as u8;
        length /= 128;
        header.push(byte | if length > 0 { 128 } else { 0 });
        if length == 0 {
            break;
        }
    }
    stream
        .write_all(&header)
        .and_then(|_| stream.write_all(data))
        .map_err(|_| ErrorCode::Network)
}

fn send_mqtt(config: &MqttConfig, event: &Event) -> std::result::Result<(), ErrorCode> {
    let url = reqwest::Url::parse(&config.broker).map_err(|_| ErrorCode::Configuration)?;
    if url.scheme() != "mqtt"
        || !config.allow_plaintext
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
        || config.topic.is_empty()
        || config.topic.contains(['#', '+'])
    {
        return Err(ErrorCode::Configuration);
    }
    let username = config.username_env.as_deref().map(secret).transpose()?;
    let password = config.password_env.as_deref().map(secret).transpose()?;
    if password.is_some() && username.is_none() {
        return Err(ErrorCode::Configuration);
    }
    let mut connect = vec![0, 4, b'M', b'Q', b'T', b'T', 4];
    connect.push(
        2 | if username.is_some() { 128 } else { 0 } | if password.is_some() { 64 } else { 0 },
    );
    connect.extend_from_slice(&[0, 30]);
    mqtt_string(
        &mut connect,
        &format!("cpass{:016x}", rand::random::<u64>()),
    )?;
    if let Some(username) = &username {
        mqtt_string(&mut connect, username)?;
    }
    if let Some(password) = &password {
        mqtt_string(&mut connect, password)?;
    }
    let mut publish = Vec::new();
    mqtt_string(&mut publish, &config.topic)?;
    publish.extend_from_slice(&[0, 1]);
    publish.extend_from_slice(&serde_json::to_vec(event).map_err(|_| ErrorCode::Operation)?);
    let host = url
        .host_str()
        .ok_or(ErrorCode::Configuration)?
        .trim_matches(['[', ']']);
    let loopback = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if (username.is_some() || password.is_some()) && !loopback {
        return Err(ErrorCode::Configuration);
    }
    let mut stream = (host, url.port().unwrap_or(1883))
        .to_socket_addrs()
        .map_err(|_| ErrorCode::Network)?
        .filter(|address| (username.is_none() && password.is_none()) || address.ip().is_loopback())
        .take(4)
        .find_map(|address| TcpStream::connect_timeout(&address, Duration::from_secs(3)).ok())
        .ok_or(ErrorCode::Network)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| ErrorCode::Network)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| ErrorCode::Network)?;
    mqtt_packet(&mut stream, 0x10, &connect)?;
    let mut reply = [0u8; 4];
    stream
        .read_exact(&mut reply)
        .map_err(|_| ErrorCode::Network)?;
    if reply != [0x20, 2, 0, 0] {
        return Err(ErrorCode::Receipt);
    }
    mqtt_packet(&mut stream, 0x32, &publish)?;
    stream
        .read_exact(&mut reply)
        .map_err(|_| ErrorCode::Network)?;
    if reply != [0x40, 2, 0, 1] {
        return Err(ErrorCode::Receipt);
    }
    let _ = mqtt_packet(&mut stream, 0xe0, &[]);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OcrConfig {
    pub executable: PathBuf,
    pub language: String,
    pub timeout_secs: u64,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            executable: "tesseract".into(),
            language: "eng".into(),
            timeout_secs: 10,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OcrHint {
    pub text: String,
    pub requires_confirmation: bool,
}

struct TemporaryOutput(PathBuf);
impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn ocr_hint(config: &OcrConfig, image: &Path) -> Result<OcrHint> {
    ensure!(
        (1..=30).contains(&config.timeout_secs),
        "OCR 超时须在 1 至 30 秒之间"
    );
    ensure!(
        !config.language.is_empty()
            && config.language.len() <= 64
            && config
                .language
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'+'),
        "OCR 语言参数无效"
    );
    let metadata = fs::symlink_metadata(image).map_err(|_| anyhow!("无法读取验证码图片"))?;
    ensure!(
        metadata.is_file() && metadata.len() <= 5 * 1024 * 1024,
        "验证码图片须为不超过 5 MiB 的普通文件"
    );
    let mut header = [0u8; 8];
    File::open(image)
        .map_err(|_| anyhow!("无法读取验证码图片"))?
        .read_exact(&mut header)
        .map_err(|_| anyhow!("验证码图片无效"))?;
    ensure!(
        header == *b"\x89PNG\r\n\x1a\n" || header.starts_with(&[0xff, 0xd8, 0xff]),
        "验证码图片须为 PNG 或 JPEG"
    );
    let image = fs::canonicalize(image).map_err(|_| anyhow!("无法读取验证码图片"))?;
    let output = TemporaryOutput(
        std::env::temp_dir().join(format!("cpass-ocr-{:016x}.tmp", rand::random::<u64>())),
    );
    let file = private_file(&output.0, false)?;
    let mut child = Command::new(&config.executable)
        .arg(image)
        .arg("stdout")
        .args(["-l", &config.language, "--psm", "7"])
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| anyhow!("无法启动本地 OCR，请安装 tesseract 或检查 executable"))?;
    let deadline = Instant::now() + Duration::from_secs(config.timeout_secs);
    loop {
        if fs::metadata(&output.0)
            .map(|m| m.len() > 16384)
            .unwrap_or(true)
            || Instant::now() >= deadline
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!("OCR 超时或输出超限"));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                ensure!(status.success(), "本地 OCR 识别失败");
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!("无法读取 OCR 进程状态"));
            }
        }
    }
    let mut data = String::new();
    File::open(&output.0)?
        .take(16385)
        .read_to_string(&mut data)?;
    ensure!(data.len() <= 16384, "OCR 输出超限");
    let text: String = data.split_whitespace().collect();
    ensure!(
        !text.is_empty() && text.len() <= 32 && text.bytes().all(|b| b.is_ascii_alphanumeric()),
        "OCR 未得到可用的字母数字候选，请人工识别"
    );
    Ok(OcrHint {
        text,
        requires_confirmation: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("cpass-ops-test-{:016x}", rand::random::<u64>()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn event() -> Event {
        Event::new(Stage::Work, Outcome::Failed, Some(ErrorCode::Incomplete))
    }

    #[test]
    fn logs_and_diagnostics_exclude_arbitrary_account_fields() {
        let dir = Scratch::new();
        let log = dir.0.join("events.jsonl");
        let output = dir.0.join("diagnostics.json");
        append_log(&log, &event()).unwrap();
        let mut secret_event = serde_json::to_value(event()).unwrap();
        secret_event["cookie"] = "SECRET_COOKIE".into();
        let mut file = OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(file, "{secret_event}").unwrap();
        writeln!(file, "password=SECRET_PASSWORD; 姓名手机号face").unwrap();
        diagnostic_bundle(&log, &output).unwrap();
        let raw = fs::read_to_string(&output).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["events"].as_array().unwrap().len(), 1);
        assert_eq!(json["discarded_lines"], 2);
        assert!(!raw.contains("SECRET"));
        assert!(diagnostic_bundle(&log, &output).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(output).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    fn mqtt_read(stream: &mut TcpStream) -> (u8, Vec<u8>) {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut kind = [0];
        stream.read_exact(&mut kind).unwrap();
        let mut length = 0;
        let mut multiplier = 1;
        loop {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            length += (byte[0] & 127) as usize * multiplier;
            if byte[0] & 128 == 0 {
                break;
            }
            multiplier *= 128;
            assert!(multiplier <= 128 * 128 * 128);
        }
        assert!(length < 65536);
        let mut data = vec![0; length];
        stream.read_exact(&mut data).unwrap();
        (kind[0], data)
    }

    fn mqtt_fixture(ack: [u8; 4]) -> (MqttConfig, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = MqttConfig {
            broker: format!("mqtt://{}", listener.local_addr().unwrap()),
            topic: "cpass/events".into(),
            username_env: None,
            password_env: None,
            allow_plaintext: true,
        };
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (kind, connect) = mqtt_read(&mut stream);
            assert_eq!(kind, 0x10);
            assert_eq!(&connect[..10], &[0, 4, b'M', b'Q', b'T', b'T', 4, 2, 0, 30]);
            stream.write_all(&[0x20, 2, 0, 0]).unwrap();
            let (kind, publish) = mqtt_read(&mut stream);
            assert_eq!(kind, 0x32);
            let topic_len = u16::from_be_bytes([publish[0], publish[1]]) as usize;
            assert_eq!(&publish[2..2 + topic_len], b"cpass/events");
            assert_eq!(&publish[2 + topic_len..4 + topic_len], &[0, 1]);
            let event: Event = serde_json::from_slice(&publish[4 + topic_len..]).unwrap();
            assert_eq!(event.error, Some(ErrorCode::Incomplete));
            stream.write_all(&ack).unwrap();
            if ack == [0x40, 2, 0, 1] {
                assert_eq!(mqtt_read(&mut stream), (0xe0, vec![]));
            }
        });
        (config, server)
    }

    #[test]
    fn mqtt_qos_one_requires_matching_puback_and_failures_are_isolated() {
        let (mqtt, server) = mqtt_fixture([0x40, 2, 0, 1]);
        let config = NotificationConfig {
            enabled: true,
            gotify: Some(GotifyConfig {
                url: "https://example.invalid".into(),
                token_env: "INVALID ENV NAME".into(),
            }),
            mqtt: Some(mqtt),
        };
        let receipts = notify(&config, &event());
        assert_eq!(receipts.len(), 2);
        assert!(!receipts[0].accepted);
        assert!(receipts[1].accepted);
        server.join().unwrap();
        let (mqtt, server) = mqtt_fixture([0x40, 2, 0, 2]);
        assert_eq!(send_mqtt(&mqtt, &event()), Err(ErrorCode::Receipt));
        server.join().unwrap();
        assert!(notify(&NotificationConfig::default(), &event()).is_empty());
    }

    #[test]
    fn notification_security_configuration_is_explicit() {
        assert!(gotify_url("http://example.invalid").is_err());
        assert!(gotify_url("https://user:secret@example.invalid").is_err());
        assert!(gotify_url("https://example.invalid?token=secret").is_err());
        let mut mqtt = MqttConfig {
            broker: "mqtt://127.0.0.1:1".into(),
            topic: "events".into(),
            username_env: None,
            password_env: None,
            allow_plaintext: false,
        };
        assert_eq!(send_mqtt(&mqtt, &event()), Err(ErrorCode::Configuration));
        mqtt.allow_plaintext = true;
        mqtt.broker = "mqtts://127.0.0.1:1".into();
        assert_eq!(send_mqtt(&mqtt, &event()), Err(ErrorCode::Configuration));
        mqtt.broker = "mqtt://127.0.0.1:1".into();
        mqtt.topic = "events/#".into();
        assert_eq!(send_mqtt(&mqtt, &event()), Err(ErrorCode::Configuration));
    }

    #[test]
    fn gotify_local_http_requires_receipt_and_uses_header_secret() {
        for (body, expected) in [
            ("{\"id\":42}", Ok(())),
            ("{\"message\":\"SECRET\"}", Err(ErrorCode::Receipt)),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = gotify_url(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 16384);
                }
                let header = String::from_utf8(request).unwrap();
                assert!(header.starts_with("POST /message HTTP/1.1\r\n"));
                assert!(
                    header
                        .to_ascii_lowercase()
                        .contains("x-gotify-key: fixture-secret\r\n")
                );
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut payload = vec![0; length];
                stream.read_exact(&mut payload).unwrap();
                assert!(!String::from_utf8_lossy(&payload).contains("fixture-secret"));
                let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
                serde_json::from_str::<Event>(value["message"].as_str().unwrap()).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            });
            assert_eq!(gotify_request(url, "fixture-secret", &event()), expected);
            server.join().unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_ocr_process_is_bounded_and_only_produces_a_manual_hint() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Scratch::new();
        let image = dir.0.join("captcha.png");
        fs::write(&image, b"\x89PNG\r\n\x1a\nfixture").unwrap();
        let executable = dir.0.join("ocr-fixture");
        fs::write(&executable, "#!/bin/sh\nprintf 'A1b2\\n'\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let config = OcrConfig {
            executable: executable.clone(),
            timeout_secs: 1,
            ..OcrConfig::default()
        };
        let hint = ocr_hint(&config, &image).unwrap();
        assert_eq!(hint.text, "A1b2");
        assert!(hint.requires_confirmation);
        fs::write(&executable, "#!/bin/sh\nexec sleep 5\n").unwrap();
        let started = Instant::now();
        assert!(
            ocr_hint(&config, &image)
                .unwrap_err()
                .to_string()
                .contains("超时")
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        fs::write(&image, b"not a real image").unwrap();
        assert!(ocr_hint(&config, &image).is_err());
    }
}
