use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use assert_cmd::Command;
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy")
        .canonicalize()
        .expect("fixture root to exist")
}

fn document_run_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy_run_document")
        .canonicalize()
        .expect("document run fixture root to exist")
}

fn live_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy_live")
        .canonicalize()
        .expect("live fixture root to exist")
}

fn rich_option_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy_rich_options")
        .canonicalize()
        .expect("rich option fixture root to exist")
}

fn qr_login_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/qr_login")
        .canonicalize()
        .expect("qr login fixture root to exist")
}

fn golden_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/golden")
        .canonicalize()
        .expect("golden root to exist")
}

fn json_searcher_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy/json_searcher_questions.json")
        .canonicalize()
        .expect("json searcher fixture to exist")
}

fn sqlite_searcher_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy/sqlite_searcher_questions.db")
        .canonicalize()
        .expect("sqlite searcher fixture to exist")
}

fn http_searcher_response_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy/http_searcher_response.json")
        .canonicalize()
        .expect("http searcher response fixture to exist")
}

fn rest_api_searcher_response_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy/rest_api_searcher_response.json")
        .canonicalize()
        .expect("rest api searcher response fixture to exist")
}

fn json_api_searcher_response_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy/json_api_searcher_response.json")
        .canonicalize()
        .expect("json api searcher response fixture to exist")
}

fn openai_searcher_response_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy/openai_searcher_response.json")
        .canonicalize()
        .expect("openai searcher response fixture to exist")
}

fn mqtt_notification_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/legacy/mqtt_publish_response.json")
        .canonicalize()
        .expect("mqtt notification fixture to exist")
}

fn read_golden(name: &str) -> serde_json::Value {
    serde_json::from_str(
        &fs::read_to_string(golden_root().join(name)).expect("golden file to be readable"),
    )
    .expect("golden json")
}

fn output_event_names(output: &serde_json::Value) -> Vec<String> {
    output["events"]
        .as_array()
        .expect("events array")
        .iter()
        .map(|event| event["event"].as_str().expect("event name").to_owned())
        .collect()
}

fn copy_fixture_dir(source: &Path, target: &Path, skipped_file_names: &[&str]) {
    fs::create_dir_all(target).expect("fixture target dir");
    for entry in fs::read_dir(source).expect("fixture dir to be readable") {
        let entry = entry.expect("fixture dir entry");
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("fixture file name");
        if skipped_file_names.contains(&file_name) {
            continue;
        }
        fs::copy(&path, target.join(file_name)).expect("fixture file copied");
    }
}

fn write_session_record(path: &Path, phone: &str, saved_at: &str) {
    let value = serde_json::json!({
        "schema": "cpass.session.v1",
        "account": {
            "puid": 114514,
            "name": format!("Session {phone}"),
            "phone": phone,
            "school": "Fixture University",
            "sex": serde_json::Value::Null,
            "student_id": serde_json::Value::Null,
        },
        "cookies": {
            "hosts": {
                "passport2.chaoxing.com": "_uid=114514; vc3=legacy-cookie; uf=legacy-token;",
                "sso.chaoxing.com": "_uid=114514; vc3=legacy-cookie; uf=legacy-token;",
                "mooc1-api.chaoxing.com": "_uid=114514; vc3=legacy-cookie; uf=legacy-token;",
                "mooc1.chaoxing.com": "_uid=114514; vc3=legacy-cookie; uf=legacy-token;"
            }
        },
        "saved_at": saved_at,
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("session parent dir");
    }
    fs::write(
        path,
        serde_json::to_string_pretty(&value).expect("session json"),
    )
    .expect("session fixture written");
}

fn write_test_config(root: &Path) -> PathBuf {
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers: []\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_notifications(root: &Path) -> PathBuf {
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers: []\nnotifications:\n  - type: gotify\n    base_url: \"https://gotify.example/message\"\n    token: \"demo-token\"\n    priority: 5\n    fixture_response_path: \"gotify_message_response.json\"\n  - type: mqtt\n    broker_url: \"mqtts://broker.example:8883\"\n    topic: \"cpass/status\"\n    qos: 1\n    retain: false\n    fixture_response_path: \"mqtt_publish_response.json\"\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_live_mqtt_notification(root: &Path, broker_url: &str) -> PathBuf {
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers: []\nnotifications:\n  - type: mqtt\n    broker_url: \"{broker_url}\"\n    topic: \"cpass/status\"\n    client_id: \"cpass-rs\"\n    username: \"demo-user\"\n    password: \"demo-password\"\n    qos: 2\n    retain: true\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
        broker_url = broker_url,
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_profiles(root: &Path) -> PathBuf {
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nlogin:\n  phone: \"13800138000\"\nsearchers: []\nprofiles:\n  automation:\n    session_path: \"{automation_session}\"\n    export_path: \"{automation_export}\"\n    login:\n      phone: \"13900139000\"\n    transport:\n      retries: 5\n    searchers:\n      - type: json\n        file_path: \"questions.json\"\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
        automation_session = root.join("automation-session").display(),
        automation_export = root.join("automation-export").display(),
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_relative_profile_paths(root: &Path) -> PathBuf {
    let config = "session_path: \"session\"\nlog_path: \"logs\"\nexport_path: \"export\"\nface_image_path: \"faces\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers: []\nprofiles:\n  automation:\n    session_path: \"session/automation\"\n    log_path: \"logs/automation\"\n    export_path: \"export/automation\"\n    face_image_path: \"faces/automation\"\n"
        .to_owned();
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_json_searcher(root: &Path, file_name: &str) -> PathBuf {
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers:\n  - type: json\n    file_path: \"{file_name}\"\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
        file_name = file_name,
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_sqlite_searcher(root: &Path, file_name: &str) -> PathBuf {
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers:\n  - type: sqlite\n    file_path: \"{file_name}\"\n    table: \"question\"\n    req_field: \"question\"\n    rsp_field: \"answer\"\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
        file_name = file_name,
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_http_searcher(root: &Path, response_file_name: &str) -> PathBuf {
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers:\n  - type: http\n    url: \"http://127.0.0.1:8088/question/search\"\n    payload_mode: \"form\"\n    method: \"GET\"\n    q_field: \"title\"\n    o_field: \"options\"\n    headers:\n      X-Searcher: \"http\"\n    ext_params:\n      tenant: \"demo\"\n    a_field: \"$.data.answer\"\n    fixture_response_path: \"{response_file_name}\"\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
        response_file_name = response_file_name,
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_rest_api_searcher(root: &Path, response_file_name: &str) -> PathBuf {
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers:\n  - type: restApiSearcher\n    url: \"http://127.0.0.1:8088/question/search\"\n    method: \"POST\"\n    q_field: \"title\"\n    o_field: \"options\"\n    headers:\n      Authorization: \"Bearer integration-token\"\n    ext_params:\n      tenant: \"demo\"\n    a_field: \"$.data.answer\"\n    fixture_response_path: \"{response_file_name}\"\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
        response_file_name = response_file_name,
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_json_api_searcher(root: &Path, response_file_name: &str) -> PathBuf {
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers:\n  - type: JsonApiSearcher\n    url: \"http://127.0.0.1:8088/question/search\"\n    headers:\n      X-Token: \"integration-token\"\n    ext_params:\n      tenant: \"demo\"\n    a_field: \"$.data.answer\"\n    fixture_response_path: \"{response_file_name}\"\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
        response_file_name = response_file_name,
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn write_test_config_with_openai_searcher(
    root: &Path,
    searcher_type: &str,
    response_file_name: &str,
) -> PathBuf {
    let prompt_line = if searcher_type.eq_ignore_ascii_case("OpenAISearcher") {
        "    prompt: \"题干：{question}\\n选项：{options}\\n全文：{search_text}\"\n"
    } else {
        "    prompt_template: \"题干：{question}\\n选项：{options}\\n全文：{search_text}\"\n"
    };
    let config = format!(
        "session_path: \"{session}\"\nlog_path: \"{logs}\"\nexport_path: \"{export}\"\nface_image_path: \"{faces}\"\ntransport:\n  timeout_secs: 20\n  retries: 3\n  retry_delay_millis: 1000\nsearchers:\n  - type: {searcher_type}\n    base_url: \"https://api.example.com/v1\"\n    model: \"gpt-compatible\"\n    system_prompt: \"只回答答案\"\n{prompt_line}    fixture_response_path: \"{response_file_name}\"\n",
        session = root.join("session").display(),
        logs = root.join("logs").display(),
        export = root.join("export").display(),
        faces = root.join("faces").display(),
        searcher_type = searcher_type,
        prompt_line = prompt_line,
        response_file_name = response_file_name,
    );
    let path = root.join("config.yml");
    fs::write(&path, config).expect("config written");
    path
}

fn bootstrap_workspace() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_path = write_test_config(temp.path());
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

fn bootstrap_workspace_without_sessions() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_path = write_test_config(temp.path());
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    (temp, config_path)
}

fn bootstrap_workspace_with_notifications() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_path = write_test_config_with_notifications(temp.path());
    fs::copy(
        fixture_root().join("gotify_message_response.json"),
        temp.path().join("gotify_message_response.json"),
    )
    .expect("gotify fixture copied");
    fs::copy(
        mqtt_notification_fixture(),
        temp.path().join("mqtt_publish_response.json"),
    )
    .expect("mqtt fixture copied");
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

fn bootstrap_workspace_with_profiles() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_path = write_test_config_with_profiles(temp.path());
    (temp, config_path)
}

fn bootstrap_workspace_with_multiple_sessions() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_path = write_test_config(temp.path());
    write_session_record(
        &temp.path().join("session/13800138000.json"),
        "13800138000",
        "2024-01-01T00:00:00Z",
    );
    write_session_record(
        &temp.path().join("session/13900139000.json"),
        "13900139000",
        "2025-01-01T00:00:00Z",
    );
    (temp, config_path)
}

fn bootstrap_workspace_with_profile_session_selection() -> (TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_path = write_test_config_with_profiles(temp.path());
    write_session_record(
        &temp.path().join("automation-session/13900139000.json"),
        "13900139000",
        "2024-01-01T00:00:00Z",
    );
    write_session_record(
        &temp.path().join("automation-session/14000140000.json"),
        "14000140000",
        "2025-01-01T00:00:00Z",
    );
    let fixture_path = temp.path().join("fixture-no-account-info");
    copy_fixture_dir(&fixture_root(), &fixture_path, &["account_info.json"]);
    (temp, config_path, fixture_path)
}

fn bootstrap_workspace_with_relative_profile_paths() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_path = write_test_config_with_relative_profile_paths(temp.path());
    (temp, config_path)
}

fn bootstrap_workspace_with_json_searcher() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let questions_path = temp.path().join("questions.json");
    fs::write(
        &questions_path,
        fs::read_to_string(json_searcher_fixture()).expect("json searcher fixture"),
    )
    .expect("json searcher copied");
    let config_path = write_test_config_with_json_searcher(temp.path(), "questions.json");
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

fn bootstrap_workspace_with_missing_mqtt_notification_fixture() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_path = write_test_config_with_notifications(temp.path());
    fs::copy(
        fixture_root().join("gotify_message_response.json"),
        temp.path().join("gotify_message_response.json"),
    )
    .expect("gotify fixture copied");
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

fn bootstrap_workspace_with_live_mqtt_notification(broker_url: &str) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_path = write_test_config_with_live_mqtt_notification(temp.path(), broker_url);
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

fn mqtt_tls_root_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/mqtt_tls/ca.cert.der")
        .canonicalize()
        .expect("mqtt tls root fixture to exist")
}

fn mqtt_tls_cert_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/mqtt_tls/server.cert.der")
        .canonicalize()
        .expect("mqtt tls cert fixture to exist")
}

fn mqtt_tls_key_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/mqtt_tls/server.key.der")
        .canonicalize()
        .expect("mqtt tls key fixture to exist")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalMqttBrokerMode {
    Plain,
    Tls,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrokerObservation {
    client_id: String,
    username: Option<String>,
    password: Option<String>,
    topic: String,
    qos: u8,
    retain: bool,
    payload: String,
}

struct LocalMqttBroker {
    broker_url: String,
    observation_rx: mpsc::Receiver<BrokerObservation>,
    join_handle: thread::JoinHandle<()>,
}

impl LocalMqttBroker {
    fn spawn(mode: LocalMqttBrokerMode) -> Self {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (observation_tx, observation_rx) = mpsc::channel();
        let join_handle = thread::spawn(move || {
            let runtime = RuntimeBuilder::new_current_thread()
                .enable_all()
                .build()
                .expect("broker runtime");
            runtime.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("broker listener");
                let port = listener.local_addr().expect("broker local addr").port();
                ready_tx.send(port).expect("broker ready");
                let (tcp, _peer) = listener.accept().await.expect("broker accept");

                let observation = match mode {
                    LocalMqttBrokerMode::Plain => drive_local_mqtt_broker_session(tcp)
                        .await
                        .expect("plain mqtt session"),
                    LocalMqttBrokerMode::Tls => {
                        let acceptor = local_mqtt_tls_acceptor();
                        let tls_stream = acceptor.accept(tcp).await.expect("tls accept");
                        drive_local_mqtt_broker_session(tls_stream)
                            .await
                            .expect("tls mqtt session")
                    }
                };

                observation_tx
                    .send(observation)
                    .expect("broker observation");
            });
        });

        let port = ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("broker port");
        let scheme = match mode {
            LocalMqttBrokerMode::Plain => "mqtt",
            LocalMqttBrokerMode::Tls => "mqtts",
        };

        Self {
            broker_url: format!("{scheme}://localhost:{port}"),
            observation_rx,
            join_handle,
        }
    }

    fn broker_url(&self) -> &str {
        &self.broker_url
    }

    fn finish(self) -> BrokerObservation {
        let observation = self
            .observation_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("broker observation");
        self.join_handle.join().expect("broker thread");
        observation
    }
}

fn local_mqtt_tls_acceptor() -> TlsAcceptor {
    let cert = fs::read(mqtt_tls_cert_fixture()).expect("mqtt tls cert");
    let key = fs::read(mqtt_tls_key_fixture()).expect("mqtt tls key");
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert)],
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key)),
        )
        .expect("mqtt tls server cert");
    TlsAcceptor::from(Arc::new(server_config))
}

async fn drive_local_mqtt_broker_session<S>(mut stream: S) -> std::io::Result<BrokerObservation>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (control, payload) = read_mqtt_packet(&mut stream).await?;
    assert_eq!(control, 0x10, "expected CONNECT packet");
    let connect = decode_connect_packet(&payload);

    write_mqtt_packet(&mut stream, &[0x20, 0x02, 0x00, 0x00]).await?;

    let (control, payload) = read_mqtt_packet(&mut stream).await?;
    let publish = decode_publish_packet(control, &payload);

    match publish.qos {
        0 => {}
        1 => write_mqtt_packet(&mut stream, &[0x40, 0x02, 0x00, 0x01]).await?,
        2 => {
            write_mqtt_packet(&mut stream, &[0x50, 0x02, 0x00, 0x01]).await?;
            let (control, payload) = read_mqtt_packet(&mut stream).await?;
            assert_eq!(control, 0x62, "expected PUBREL packet");
            assert_eq!(
                payload.as_slice(),
                &[0x00, 0x01],
                "expected PUBREL packet id"
            );
            write_mqtt_packet(&mut stream, &[0x70, 0x02, 0x00, 0x01]).await?;
        }
        other => panic!("unexpected qos {other}"),
    }

    let (control, payload) = read_mqtt_packet(&mut stream).await?;
    assert_eq!(control, 0xe0, "expected DISCONNECT packet");
    assert!(payload.is_empty(), "expected empty DISCONNECT payload");

    Ok(BrokerObservation {
        client_id: connect.client_id,
        username: connect.username,
        password: connect.password,
        topic: publish.topic,
        qos: publish.qos,
        retain: publish.retain,
        payload: publish.payload,
    })
}

async fn read_mqtt_packet<S>(stream: &mut S) -> std::io::Result<(u8, Vec<u8>)>
where
    S: AsyncRead + Unpin,
{
    let mut control = [0_u8; 1];
    stream.read_exact(&mut control).await?;
    let remaining_length = read_mqtt_remaining_length(stream).await?;
    let mut payload = vec![0_u8; remaining_length];
    stream.read_exact(&mut payload).await?;
    Ok((control[0], payload))
}

async fn read_mqtt_remaining_length<S>(stream: &mut S) -> std::io::Result<usize>
where
    S: AsyncRead + Unpin,
{
    let mut multiplier = 1_usize;
    let mut value = 0_usize;

    for _ in 0..4 {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await?;
        value += usize::from(byte[0] & 0x7f) * multiplier;
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
        multiplier *= 128;
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid mqtt remaining length encoding",
    ))
}

async fn write_mqtt_packet<S>(stream: &mut S, packet: &[u8]) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(packet).await?;
    stream.flush().await
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodedConnectPacket {
    client_id: String,
    username: Option<String>,
    password: Option<String>,
}

fn decode_connect_packet(payload: &[u8]) -> DecodedConnectPacket {
    let mut cursor = Cursor::new(payload);
    assert_eq!(read_mqtt_string(&mut cursor), "MQTT");
    assert_eq!(read_u8(&mut cursor), 0x04, "expected MQTT protocol level 4");
    let flags = read_u8(&mut cursor);
    assert_eq!(read_u16(&mut cursor), 30, "expected keepalive=30");

    let client_id = read_mqtt_string(&mut cursor);
    let username = if flags & 0b1000_0000 != 0 {
        Some(read_mqtt_string(&mut cursor))
    } else {
        None
    };
    let password = if flags & 0b0100_0000 != 0 {
        Some(read_mqtt_string(&mut cursor))
    } else {
        None
    };
    assert_eq!(
        cursor.position() as usize,
        payload.len(),
        "expected full CONNECT payload"
    );

    DecodedConnectPacket {
        client_id,
        username,
        password,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodedPublishPacket {
    topic: String,
    qos: u8,
    retain: bool,
    payload: String,
}

fn decode_publish_packet(control: u8, payload: &[u8]) -> DecodedPublishPacket {
    let qos = (control & 0b0000_0110) >> 1;
    let retain = control & 0b0000_0001 != 0;
    let mut cursor = Cursor::new(payload);
    let topic = read_mqtt_string(&mut cursor);
    if qos > 0 {
        assert_eq!(read_u16(&mut cursor), 1, "expected MQTT packet id 1");
    }
    let mut payload_bytes = Vec::new();
    Read::read_to_end(&mut cursor, &mut payload_bytes).expect("mqtt publish payload");

    DecodedPublishPacket {
        topic,
        qos,
        retain,
        payload: String::from_utf8(payload_bytes).expect("utf8 mqtt payload"),
    }
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> u8 {
    let mut buf = [0_u8; 1];
    Read::read_exact(cursor, &mut buf).expect("mqtt u8");
    buf[0]
}

fn read_u16(cursor: &mut Cursor<&[u8]>) -> u16 {
    let mut buf = [0_u8; 2];
    Read::read_exact(cursor, &mut buf).expect("mqtt u16");
    u16::from_be_bytes(buf)
}

fn read_mqtt_string(cursor: &mut Cursor<&[u8]>) -> String {
    let length = read_u16(cursor) as usize;
    let mut bytes = vec![0_u8; length];
    Read::read_exact(cursor, &mut bytes).expect("mqtt string");
    String::from_utf8(bytes).expect("utf8 mqtt string")
}

fn expected_config_validate_mqtt_payload(config_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "schema": "cpass.notification.v1",
        "kind": "config_validate",
        "level": "success",
        "title": "cpass config validate completed",
        "body": format!("path={} searchers=0", config_path.display()),
        "warning_count": 0,
        "retry_count": 0,
        "warnings": [],
    })
}

fn bootstrap_workspace_with_sqlite_searcher() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    fs::copy(sqlite_searcher_fixture(), temp.path().join("questions.db"))
        .expect("sqlite searcher copied");
    let config_path = write_test_config_with_sqlite_searcher(temp.path(), "questions.db");
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

fn bootstrap_workspace_with_http_searcher() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    fs::copy(
        http_searcher_response_fixture(),
        temp.path().join("http_searcher_response.json"),
    )
    .expect("http response fixture copied");
    let config_path =
        write_test_config_with_http_searcher(temp.path(), "http_searcher_response.json");
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

fn bootstrap_workspace_with_rest_api_searcher() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    fs::copy(
        rest_api_searcher_response_fixture(),
        temp.path().join("rest_api_searcher_response.json"),
    )
    .expect("rest api response fixture copied");
    let config_path =
        write_test_config_with_rest_api_searcher(temp.path(), "rest_api_searcher_response.json");
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

fn bootstrap_workspace_with_json_api_searcher() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    fs::copy(
        json_api_searcher_response_fixture(),
        temp.path().join("json_api_searcher_response.json"),
    )
    .expect("json api response fixture copied");
    let config_path =
        write_test_config_with_json_api_searcher(temp.path(), "json_api_searcher_response.json");
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

fn bootstrap_workspace_with_openai_searcher(searcher_type: &str) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    fs::copy(
        openai_searcher_response_fixture(),
        temp.path().join("openai_searcher_response.json"),
    )
    .expect("openai response fixture copied");
    let config_path = write_test_config_with_openai_searcher(
        temp.path(),
        searcher_type,
        "openai_searcher_response.json",
    );
    fs::create_dir_all(temp.path().join("session")).expect("session dir");
    fs::copy(
        fixture_root().join("legacy_session.json"),
        temp.path().join("session/13800138000.json"),
    )
    .expect("session fixture copied");
    (temp, config_path)
}

#[test]
fn lists_courses_with_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "courses",
            "list",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output, read_golden("courses_list.json"));
}

#[test]
fn rejects_ambiguous_saved_sessions_without_non_interactive_selector() {
    let (_temp, config_path) = bootstrap_workspace_with_multiple_sessions();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "courses",
            "list",
        ])
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("multiple saved sessions found"),
        "expected ambiguous session error, got: {stderr}"
    );
    assert!(
        stderr.contains("--phone") && stderr.contains("login.phone/CPASS_PHONE"),
        "expected actionable selector hint, got: {stderr}"
    );
}

#[test]
fn shows_course_with_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "courses",
            "show",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output, read_golden("courses_show_1001.json"));
}

#[test]
fn shows_course_with_course_index_selector() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "courses",
            "show",
            "--course-index",
            "0",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output, read_golden("courses_show_1001.json"));
}

#[test]
fn scans_tasks_with_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "tasks",
            "scan",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(
        output["chapters"][0]["task_points"][0]["iframe_data"],
        "{\"objectid\":\"video-001\"}"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][1]["iframe_data"],
        "{\"workid\":\"work-001\",\"_jobid\":\"job-001\"}"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["attachment_metadata"]["attachment"]["attachment_type"],
        "video"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["attachment_metadata"]["attachment"]["playback_rate"],
        "0.9"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["attachment_metadata"]["video_status"]["duration_secs"],
        602
    );
    assert_eq!(
        output["chapters"][1]["task_points"][0]["attachment_metadata"]["attachment"]["attachment_type"],
        "document"
    );
    assert_eq!(
        output["chapters"][1]["task_points"][0]["attachment_metadata"]["attachment"]["file_type"],
        "pdf"
    );
    assert_eq!(output, read_golden("tasks_scan.json"));
}

#[test]
fn scans_live_tasks_with_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            live_fixture_root().to_str().expect("utf8 path"),
            "tasks",
            "scan",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["chapters"][0]["chapter_id"], 13);
    assert_eq!(
        output["chapters"][0]["task_points"][0]["module"],
        "insertlive"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["resource_id"],
        "live-course-001"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["attachment_metadata"]["attachment"]["attachment_type"],
        "live"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["attachment_metadata"]["attachment"]["stream_name"],
        "zhibo_12345"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["attachment_metadata"]["attachment"]["vdo_id"],
        "vdo-live-001"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["attachment_metadata"]["attachment"]["live_id"],
        "live-course-001"
    );
    assert_eq!(output, read_golden("tasks_scan_live.json"));
}

#[test]
fn plans_course_run_with_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["course"]["course_id"], 1001);
    assert_eq!(output["total_task_points"], 3);
    assert_eq!(output["events"].as_array().map(Vec::len), Some(11));
    assert_eq!(output["events"][0]["event"], "course_run_planning_started");
    assert_eq!(output["events"][0]["course_id"], 1001);
    assert!(output["events"][0]["course_index"].is_null());
    assert_eq!(output["events"][1]["event"], "course_run_planning_finished");
    assert_eq!(output["events"][1]["course_id"], 1001);
    assert_eq!(output["events"][1]["chapters"], 2);
    assert_eq!(output["events"][1]["task_points"], 3);
    assert_eq!(output["events"][2]["event"], "course_run_execution_started");
    assert_eq!(output["events"][2]["course_id"], 1001);
    assert_eq!(output["events"][2]["total_entries"], 3);
    assert_eq!(
        output["events"][3],
        serde_json::json!({
            "event": "course_run_queue_entry_state_changed",
            "course_id": 1001,
            "queue_index": 0,
            "state": "running"
        })
    );
    assert_eq!(
        output["events"][4],
        serde_json::json!({
            "event": "course_run_video_progress_reported",
            "course_id": 1001,
            "queue_index": 0,
            "playing_time_secs": 602,
            "duration_secs": 602,
            "is_passed": true
        })
    );
    assert_eq!(
        output["events"][5],
        serde_json::json!({
            "event": "course_run_queue_entry_state_changed",
            "course_id": 1001,
            "queue_index": 0,
            "state": "completed"
        })
    );
    assert_eq!(
        output["events"][6],
        serde_json::json!({
            "event": "course_run_queue_entry_state_changed",
            "course_id": 1001,
            "queue_index": 1,
            "state": "running"
        })
    );
    assert_eq!(
        output["events"][7]["event"],
        "course_run_chapter_work_snapshot_fetched"
    );
    assert_eq!(output["events"][7]["course_id"], 1001);
    assert_eq!(output["events"][7]["queue_index"], 1);
    assert_eq!(output["events"][7]["snapshot"]["work_answer_id"], 99001);
    assert_eq!(output["events"][7]["snapshot"]["total_question_num"], 3);
    assert_eq!(
        output["events"][7]["snapshot"]["questions"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    assert_eq!(output["events"][8]["event"], "warning");
    assert!(
        output["events"][8]["message"]
            .as_str()
            .expect("warning message")
            .contains("queue entry 1 fetched chapter-work runtime snapshot 99001 with 3 questions")
    );
    assert_eq!(
        output["events"][9],
        serde_json::json!({
            "event": "course_run_queue_entry_state_changed",
            "course_id": 1001,
            "queue_index": 1,
            "state": "blocked"
        })
    );
    assert_eq!(
        output["events"][10]["event"],
        "course_run_execution_finished"
    );
    assert_eq!(output["events"][10]["state"], "blocked");
    assert_eq!(output["events"][10]["completed_entries"], 1);
    assert_eq!(output["events"][10]["blocked_entries"], 1);
    assert_eq!(output["execution_queue"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        output["execution_preflight"]["all_entries_registered"],
        true
    );
    assert_eq!(output["execution_preflight"]["total_entries"], 3);
    assert_eq!(output["execution_preflight"]["registered_entries"], 3);
    assert_eq!(output["execution_preflight"]["blocked_entries"], 0);
    assert_eq!(
        output["execution_preflight"]["queue"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    assert_eq!(output["execution_preflight"]["queue"][0]["queue_index"], 0);
    assert_eq!(
        output["execution_preflight"]["queue"][0]["resolution"]["status"],
        "registered"
    );
    assert_eq!(
        output["execution_preflight"]["queue"][0]["resolution"]["registration"]["key"],
        "video"
    );
    assert_eq!(output["execution_queue"][0]["queue_index"], 0);
    assert_eq!(output["execution_queue"][0]["chapter_id"], 11);
    assert_eq!(
        output["chapters"][0]["task_points"][0]["task_kind"]["kind"],
        "video"
    );
    assert_eq!(output["execution_queue"][0]["task_kind"]["kind"], "video");
    assert_eq!(
        output["chapters"][0]["task_points"][1]["task_kind"]["kind"],
        "chapter_work"
    );
    assert_eq!(output["execution_queue"][1]["chapter_id"], 11);
    assert_eq!(
        output["execution_queue"][1]["task_kind"]["kind"],
        "chapter_work"
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["attachment_metadata"]["video_status"]["duration_secs"],
        602
    );
    assert_eq!(
        output["chapters"][0]["task_points"][0]["attachment_metadata"]["attachment"]["playback_rate"],
        "0.9"
    );
    assert_eq!(
        output["execution_queue"][0]["attachment_metadata"]["video_status"]["duration_secs"],
        602
    );
    assert_eq!(
        output["execution_queue"][0]["attachment_metadata"]["attachment"]["playback_rate"],
        "0.9"
    );
    assert_eq!(
        output["execution_queue"][0]["iframe_data"],
        "{\"objectid\":\"video-001\"}"
    );
    assert_eq!(
        output["execution_queue"][1]["iframe_data"],
        "{\"workid\":\"work-001\",\"_jobid\":\"job-001\"}"
    );
    assert_eq!(
        output["chapters"][1]["task_points"][0]["attachment_metadata"]["attachment"]["file_type"],
        "pdf"
    );
    assert_eq!(output["execution_queue"][2]["chapter_id"], 12);
    assert_eq!(
        output["execution_queue"][2]["attachment_metadata"]["attachment"]["file_type"],
        "pdf"
    );
    assert_eq!(output["execution_preflight"]["queue"][2]["queue_index"], 2);
    assert_eq!(
        output["execution_preflight"]["queue"][2]["resolution"]["registration"]["key"],
        "document"
    );
    assert_eq!(output["execution_result"]["course_id"], 1001);
    assert_eq!(output["execution_result"]["state"], "blocked");
    assert_eq!(output["execution_result"]["total_entries"], 3);
    assert_eq!(output["execution_result"]["completed_entries"], 1);
    assert_eq!(output["execution_result"]["blocked_entries"], 1);
    assert_eq!(
        output["execution_result"]["queue"].as_array().map(Vec::len),
        Some(3)
    );
    assert_eq!(output["execution_result"]["queue"][0]["queue_index"], 0);
    assert_eq!(output["execution_result"]["queue"][0]["state"], "completed");
    assert_eq!(output["execution_result"]["queue"][1]["queue_index"], 1);
    assert_eq!(output["execution_result"]["queue"][1]["state"], "blocked");
    assert_eq!(output["execution_result"]["queue"][2]["queue_index"], 2);
    assert_eq!(output["execution_result"]["queue"][2]["state"], "pending");
    assert_eq!(output, read_golden("run_course_plan_1001.json"));
}

#[test]
fn plans_course_run_with_notification_config_keeps_json_output_stable() {
    let (_temp, config_path) = bootstrap_workspace_with_notifications();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["events"].as_array().map(Vec::len), Some(11));
    assert_eq!(output["execution_result"]["state"], "blocked");
    assert_eq!(output, read_golden("run_course_plan_1001.json"));
}

#[test]
fn validates_config_with_gotify_fixture_notification_config() {
    let (_temp, config_path) = bootstrap_workspace_with_notifications();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "config",
            "validate",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(
        output["config"]["notifications"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(
        output["config"]["notifications"][0]["values"]["fixture_response_path"],
        serde_json::Value::String("gotify_message_response.json".to_owned())
    );
    assert_eq!(output["config"]["notifications"][1]["type"], "mqtt");
    assert_eq!(
        output["config"]["notifications"][1]["values"]["fixture_response_path"],
        serde_json::Value::String("mqtt_publish_response.json".to_owned())
    );
}

#[test]
fn config_validate_warns_when_mqtt_fixture_delivery_fails() {
    let (_temp, config_path) = bootstrap_workspace_with_missing_mqtt_notification_fixture();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "config",
            "validate",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);

    assert_eq!(
        output["config"]["notifications"].as_array().map(Vec::len),
        Some(2)
    );
    assert!(
        stderr.contains("warning: mqtt notification delivery failed:"),
        "expected mqtt warning in stderr, got: {stderr}"
    );
}

#[test]
fn config_validate_warns_when_live_mqtt_delivery_fails() {
    let (_temp, config_path) =
        bootstrap_workspace_with_live_mqtt_notification("mqtt://127.0.0.1:1");
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "config",
            "validate",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);

    assert_eq!(
        output["config"]["notifications"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(output["config"]["notifications"][0]["type"], "mqtt");
    assert_eq!(
        output["config"]["notifications"][0]["values"]["client_id"],
        serde_json::Value::String("cpass-rs".to_owned())
    );
    assert_eq!(
        output["config"]["notifications"][0]["values"]["qos"],
        serde_json::Value::Number(2.into())
    );
    assert_eq!(
        output["config"]["notifications"][0]["values"]["retain"],
        serde_json::Value::Bool(true)
    );
    assert!(
        stderr.contains("warning: mqtt notification delivery failed:"),
        "expected live mqtt warning in stderr, got: {stderr}"
    );
}

#[test]
#[ignore = "requires loopback listener support for the local mqtt broker harness"]
fn config_validate_dispatches_live_mqtt_notification_to_local_broker() {
    let broker = LocalMqttBroker::spawn(LocalMqttBrokerMode::Plain);
    let (_temp, config_path) = bootstrap_workspace_with_live_mqtt_notification(broker.broker_url());
    let expected_payload = expected_config_validate_mqtt_payload(&config_path);
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "config",
            "validate",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);

    assert_eq!(
        output["config"]["notifications"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(output["config"]["notifications"][0]["type"], "mqtt");
    assert_eq!(
        output["config"]["notifications"][0]["values"]["broker_url"],
        serde_json::Value::String(broker.broker_url().to_owned())
    );
    assert!(
        !stderr.contains("warning: mqtt notification delivery failed:"),
        "did not expect mqtt warning in stderr, got: {stderr}"
    );

    let observation = broker.finish();
    assert_eq!(observation.client_id, "cpass-rs");
    assert_eq!(observation.username.as_deref(), Some("demo-user"));
    assert_eq!(observation.password.as_deref(), Some("demo-password"));
    assert_eq!(observation.topic, "cpass/status");
    assert_eq!(observation.qos, 2);
    assert!(observation.retain);
    let actual_payload: serde_json::Value =
        serde_json::from_str(&observation.payload).expect("valid mqtt payload json");
    assert_eq!(actual_payload, expected_payload);
}

#[test]
#[ignore = "requires loopback listener support for the local mqtt broker harness"]
fn config_validate_dispatches_live_mqtts_notification_to_local_tls_broker() {
    let broker = LocalMqttBroker::spawn(LocalMqttBrokerMode::Tls);
    let (_temp, config_path) = bootstrap_workspace_with_live_mqtt_notification(broker.broker_url());
    let expected_payload = expected_config_validate_mqtt_payload(&config_path);
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .env("CPASS_MQTT_EXTRA_ROOT_CERT_DER", mqtt_tls_root_fixture())
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "config",
            "validate",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);

    assert_eq!(
        output["config"]["notifications"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(output["config"]["notifications"][0]["type"], "mqtt");
    assert_eq!(
        output["config"]["notifications"][0]["values"]["broker_url"],
        serde_json::Value::String(broker.broker_url().to_owned())
    );
    assert!(
        !stderr.contains("warning: mqtt notification delivery failed:"),
        "did not expect mqtt warning in stderr, got: {stderr}"
    );

    let observation = broker.finish();
    assert_eq!(observation.client_id, "cpass-rs");
    assert_eq!(observation.username.as_deref(), Some("demo-user"));
    assert_eq!(observation.password.as_deref(), Some("demo-password"));
    assert_eq!(observation.topic, "cpass/status");
    assert_eq!(observation.qos, 2);
    assert!(observation.retain);
    let actual_payload: serde_json::Value =
        serde_json::from_str(&observation.payload).expect("valid mqtt payload json");
    assert_eq!(actual_payload, expected_payload);
}

#[test]
fn config_validate_selects_named_profile_from_flag() {
    let (temp, config_path) = bootstrap_workspace_with_profiles();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--profile",
            "automation",
            "config",
            "validate",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(
        output["selected_profile"],
        serde_json::Value::String("automation".to_owned())
    );
    assert_eq!(
        output["config"]["paths"]["session_dir"],
        serde_json::Value::String(temp.path().join("automation-session").display().to_string())
    );
    assert_eq!(
        output["config"]["paths"]["export_dir"],
        serde_json::Value::String(temp.path().join("automation-export").display().to_string())
    );
    assert_eq!(
        output["config"]["login"]["phone"],
        serde_json::Value::String("13900139000".to_owned())
    );
    assert_eq!(output["config"]["transport"]["retries"], 5);
    assert_eq!(
        output["config"]["searchers"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(output["config"]["searchers"][0]["type"], "json");
}

#[test]
fn config_validate_selects_named_profile_from_env() {
    let (temp, config_path) = bootstrap_workspace_with_profiles();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .env("CPASS_PROFILE", "automation")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "config",
            "validate",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(
        output["selected_profile"],
        serde_json::Value::String("automation".to_owned())
    );
    assert_eq!(
        output["config"]["paths"]["session_dir"],
        serde_json::Value::String(temp.path().join("automation-session").display().to_string())
    );
    assert_eq!(
        output["config"]["login"]["phone"],
        serde_json::Value::String("13900139000".to_owned())
    );
}

#[test]
fn config_validate_surfaces_normalized_automation_paths() {
    let (temp, config_path) = bootstrap_workspace_with_relative_profile_paths();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--profile",
            "automation",
            "config",
            "validate",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(
        output["selected_profile"],
        serde_json::Value::String("automation".to_owned())
    );
    assert_eq!(
        output["config"]["paths"]["session_dir"],
        serde_json::Value::String("session/automation".to_owned())
    );
    assert_eq!(
        output["config"]["paths"]["log_dir"],
        serde_json::Value::String("logs/automation".to_owned())
    );
    assert_eq!(
        output["automation_paths"]["session_dir"],
        serde_json::Value::String(temp.path().join("session/automation").display().to_string())
    );
    assert_eq!(
        output["automation_paths"]["log_dir"],
        serde_json::Value::String(temp.path().join("logs/automation").display().to_string())
    );
    assert_eq!(
        output["automation_paths"]["export_dir"],
        serde_json::Value::String(temp.path().join("export/automation").display().to_string())
    );
    assert_eq!(
        output["automation_paths"]["face_dir"],
        serde_json::Value::String(temp.path().join("faces/automation").display().to_string())
    );
}

#[test]
fn doctor_surfaces_selected_profile_and_normalized_automation_paths() {
    let (temp, config_path) = bootstrap_workspace_with_relative_profile_paths();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--profile",
            "automation",
            "doctor",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(
        output["selected_profile"],
        serde_json::Value::String("automation".to_owned())
    );
    assert_eq!(
        output["session_dir"],
        serde_json::Value::String("session/automation".to_owned())
    );
    assert_eq!(
        output["log_dir"],
        serde_json::Value::String("logs/automation".to_owned())
    );
    assert_eq!(
        output["automation_paths"]["session_dir"],
        serde_json::Value::String(temp.path().join("session/automation").display().to_string())
    );
    assert_eq!(
        output["automation_paths"]["log_dir"],
        serde_json::Value::String(temp.path().join("logs/automation").display().to_string())
    );
    assert_eq!(
        output["automation_paths"]["export_dir"],
        serde_json::Value::String(temp.path().join("export/automation").display().to_string())
    );
    assert_eq!(
        output["automation_paths"]["face_dir"],
        serde_json::Value::String(temp.path().join("faces/automation").display().to_string())
    );
}

#[test]
fn plans_course_run_with_rich_option_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            rich_option_fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["course"]["course_id"], 1001);
    assert_eq!(output["total_task_points"], 1);
    assert_eq!(
        output["execution_queue"][0]["task_kind"]["kind"],
        "chapter_work"
    );
    assert_eq!(output["events"].as_array().map(Vec::len), Some(8));

    let snapshot_event = output["events"]
        .as_array()
        .expect("events array")
        .iter()
        .find(|event| event["event"] == "course_run_chapter_work_snapshot_fetched")
        .expect("chapter-work snapshot event");
    assert_eq!(
        snapshot_event["snapshot"]["questions"][0]["options"][0]["rich_content"]["image_urls"][0],
        serde_json::Value::String("https://static.example/work-a.png".to_owned())
    );
    assert_eq!(
        snapshot_event["snapshot"]["questions"][0]["options"][1]["value"],
        serde_json::Value::String(String::new())
    );
    assert_eq!(
        snapshot_event["snapshot"]["questions"][0]["options"][1]["rich_content"]["image_urls"][0],
        serde_json::Value::String("https://static.example/work-b.png".to_owned())
    );
    assert!(snapshot_event["snapshot"]["questions"][0]["options"][2]["rich_content"].is_null());
    assert_eq!(
        output,
        read_golden("run_course_plan_rich_options_1001.json")
    );
}

#[test]
fn plans_live_course_run_with_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            live_fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["course"]["course_id"], 1001);
    assert_eq!(output["total_task_points"], 1);
    assert_eq!(
        output["chapters"][0]["task_points"][0]["task_kind"]["kind"],
        "live"
    );
    assert_eq!(
        output["execution_queue"][0]["attachment_metadata"]["attachment"]["live_id"],
        "live-course-001"
    );
    assert_eq!(
        output["execution_queue"][0]["attachment_metadata"]["attachment"]["stream_name"],
        "zhibo_12345"
    );
    assert_eq!(
        output["execution_preflight"]["all_entries_registered"],
        true
    );
    assert_eq!(output["execution_preflight"]["registered_entries"], 1);
    assert_eq!(output["execution_preflight"]["blocked_entries"], 0);
    assert_eq!(
        output["execution_preflight"]["queue"][0]["resolution"]["status"],
        "registered"
    );
    assert_eq!(
        output["execution_preflight"]["queue"][0]["resolution"]["registration"]["key"],
        "live"
    );
    assert_eq!(output["execution_result"]["state"], "blocked");
    assert_eq!(output["execution_result"]["blocked_entries"], 1);
    assert_eq!(output["execution_result"]["queue"][0]["state"], "blocked");
    assert_eq!(
        output["execution_result"]["queue"][0]["resolution"]["status"],
        "registered"
    );
    assert_eq!(
        output["execution_result"]["queue"][0]["resolution"]["registration"]["key"],
        "live"
    );
    assert_eq!(output["events"].as_array().map(Vec::len), Some(8));
    assert_eq!(
        output["events"][3]["event"],
        "course_run_queue_entry_state_changed"
    );
    assert_eq!(output["events"][3]["state"], "running");
    assert_eq!(
        output["events"][4]["event"],
        "course_run_live_progress_reported"
    );
    assert_eq!(output["events"][4]["success"], true);
    assert_eq!(output["events"][5]["event"], "warning");
    assert!(
        output["events"][5]["message"]
            .as_str()
            .expect("warning message")
            .contains("acknowledged the reviewed live progress-report route")
    );
    assert_eq!(
        output["events"][6]["event"],
        "course_run_queue_entry_state_changed"
    );
    assert_eq!(output["events"][6]["state"], "blocked");
    assert_eq!(
        output["events"][7]["event"],
        "course_run_execution_finished"
    );
    assert_eq!(output, read_golden("run_course_plan_live_1001.json"));
}

#[test]
fn plans_course_run_with_document_executor_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            document_run_fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["course"]["course_id"], 1001);
    assert_eq!(output["total_task_points"], 3);
    assert_eq!(output["events"].as_array().map(Vec::len), Some(13));
    assert_eq!(
        output["events"][4]["event"],
        "course_run_video_progress_reported"
    );
    assert_eq!(
        output["events"][7],
        serde_json::json!({
            "event": "course_run_document_progress_reported",
            "course_id": 1001,
            "queue_index": 1,
            "success": true
        })
    );
    assert_eq!(output["events"][10]["event"], "warning");
    assert!(
        output["events"][10]["message"]
            .as_str()
            .expect("warning message")
            .contains("queue entry 2 chapter-work runtime snapshot fetch failed")
    );
    assert_eq!(output["execution_result"]["state"], "blocked");
    assert_eq!(output["execution_result"]["completed_entries"], 2);
    assert_eq!(output["execution_result"]["blocked_entries"], 1);
    assert_eq!(output["execution_result"]["queue"][0]["state"], "completed");
    assert_eq!(output["execution_result"]["queue"][1]["state"], "completed");
    assert_eq!(output["execution_result"]["queue"][2]["state"], "blocked");
    assert_eq!(output, read_golden("run_course_plan_document_1001.json"));
}

#[test]
fn plans_course_run_with_json_searcher_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace_with_json_searcher();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["events"].as_array().map(Vec::len), Some(12));
    assert_eq!(
        output["events"][8],
        serde_json::json!({
            "event": "course_run_chapter_work_candidate_selection_prepared",
            "course_id": 1001,
            "queue_index": 1,
            "work_answer_id": 99001,
            "total_questions": 3,
            "selected_questions": 3
        })
    );
    assert_eq!(output["events"][9]["event"], "warning");
    assert!(
        output["events"][9]["message"]
            .as_str()
            .expect("warning message")
            .contains("prepared chapter-work candidate selections for 3/3 questions")
    );
    assert_eq!(output["events"][10]["state"], "blocked");
    assert_eq!(
        output["events"][11]["event"],
        "course_run_execution_finished"
    );
    assert_eq!(
        output,
        read_golden("run_course_plan_json_searcher_1001.json")
    );
}

#[test]
fn plans_course_run_with_sqlite_searcher_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace_with_sqlite_searcher();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["events"].as_array().map(Vec::len), Some(12));
    assert_eq!(
        output["events"][8],
        serde_json::json!({
            "event": "course_run_chapter_work_candidate_selection_prepared",
            "course_id": 1001,
            "queue_index": 1,
            "work_answer_id": 99001,
            "total_questions": 3,
            "selected_questions": 3
        })
    );
    assert_eq!(output["events"][9]["event"], "warning");
    assert!(
        output["events"][9]["message"]
            .as_str()
            .expect("warning message")
            .contains("prepared chapter-work candidate selections for 3/3 questions")
    );
    assert_eq!(output["events"][10]["state"], "blocked");
    assert_eq!(
        output["events"][11]["event"],
        "course_run_execution_finished"
    );
    assert_eq!(
        output,
        read_golden("run_course_plan_sqlite_searcher_1001.json")
    );
}

#[test]
fn plans_course_run_with_http_searcher_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace_with_http_searcher();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["events"].as_array().map(Vec::len), Some(12));
    assert_eq!(
        output["events"][8],
        serde_json::json!({
            "event": "course_run_chapter_work_candidate_selection_prepared",
            "course_id": 1001,
            "queue_index": 1,
            "work_answer_id": 99001,
            "total_questions": 3,
            "selected_questions": 3
        })
    );
    assert_eq!(output["events"][9]["event"], "warning");
    assert_eq!(output["events"][10]["state"], "blocked");
    assert_eq!(
        output["events"][11]["event"],
        "course_run_execution_finished"
    );
    assert_eq!(
        output,
        read_golden("run_course_plan_json_searcher_1001.json")
    );
}

#[test]
fn plans_course_run_with_legacy_rest_api_searcher_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace_with_rest_api_searcher();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["events"].as_array().map(Vec::len), Some(12));
    assert_eq!(
        output,
        read_golden("run_course_plan_json_searcher_1001.json")
    );
}

#[test]
fn plans_course_run_with_legacy_json_api_searcher_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace_with_json_api_searcher();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["events"].as_array().map(Vec::len), Some(12));
    assert_eq!(
        output,
        read_golden("run_course_plan_json_searcher_1001.json")
    );
}

#[test]
fn plans_course_run_with_openai_compatible_searcher_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace_with_openai_searcher("openai-compatible");
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .env("CPASS_OPENAI_API_KEY", "sk-offline-openai-compatible")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["events"].as_array().map(Vec::len), Some(12));
    assert_eq!(
        output,
        read_golden("run_course_plan_json_searcher_1001.json")
    );
}

#[test]
fn plans_course_run_with_legacy_openai_searcher_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace_with_openai_searcher("OpenAISearcher");
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .env("CPASS_OPENAI_API_KEY", "sk-offline-legacy-openai")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["events"].as_array().map(Vec::len), Some(12));
    assert_eq!(
        output,
        read_golden("run_course_plan_json_searcher_1001.json")
    );
}

#[test]
fn emits_fixture_run_events_in_order_for_chapter_work_snapshot_path() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(
        output_event_names(&output),
        vec![
            "course_run_planning_started",
            "course_run_planning_finished",
            "course_run_execution_started",
            "course_run_queue_entry_state_changed",
            "course_run_video_progress_reported",
            "course_run_queue_entry_state_changed",
            "course_run_queue_entry_state_changed",
            "course_run_chapter_work_snapshot_fetched",
            "warning",
            "course_run_queue_entry_state_changed",
            "course_run_execution_finished",
        ]
    );
    assert_eq!(output["events"][3]["queue_index"], 0);
    assert_eq!(output["events"][6]["queue_index"], 1);
    assert_eq!(output["events"][9]["state"], "blocked");
    assert_eq!(output["events"][10]["blocked_entries"], 1);
}

#[test]
fn emits_fixture_run_events_in_order_for_document_then_blocked_path() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            document_run_fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(
        output_event_names(&output),
        vec![
            "course_run_planning_started",
            "course_run_planning_finished",
            "course_run_execution_started",
            "course_run_queue_entry_state_changed",
            "course_run_video_progress_reported",
            "course_run_queue_entry_state_changed",
            "course_run_queue_entry_state_changed",
            "course_run_document_progress_reported",
            "course_run_queue_entry_state_changed",
            "course_run_queue_entry_state_changed",
            "warning",
            "course_run_queue_entry_state_changed",
            "course_run_execution_finished",
        ]
    );
    assert_eq!(output["events"][7]["queue_index"], 1);
    assert_eq!(output["events"][8]["state"], "completed");
    assert_eq!(output["events"][9]["queue_index"], 2);
    assert_eq!(output["events"][11]["state"], "blocked");
    assert_eq!(output["events"][12]["completed_entries"], 2);
}

#[test]
fn renders_course_run_tui_snapshot_with_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
            "--tui",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(stdout.contains("cpass run TUI"));
    assert!(stdout.contains("Planning: finished for course 1001 (2 chapters, 3 task points)"));
    assert!(stdout.contains("Execution: blocked for course 1001 (3 total entries)"));
    assert!(stdout.contains("Queue: 1 completed, 1 blocked, 1 pending"));
    assert!(stdout.contains("video queue[0] reported 602/602s (is_passed=true)"));
    assert!(stdout.contains("chapter_work queue[1] fetched snapshot 99001 (3 questions)"));
    assert!(
        stdout
            .contains("queue entry 1 fetched chapter-work runtime snapshot 99001 with 3 questions")
    );
}

#[test]
fn renders_course_run_tui_snapshot_with_document_executor_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            document_run_fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
            "--tui",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(stdout.contains("cpass run TUI"));
    assert!(stdout.contains("Execution: blocked for course 1001 (3 total entries)"));
    assert!(stdout.contains("Queue: 2 completed, 1 blocked, 0 pending"));
    assert!(stdout.contains("document queue[1] reported success=true"));
    assert!(stdout.contains("queue entry 2 chapter-work runtime snapshot fetch failed"));
}

#[test]
fn launches_default_tui_from_top_level_invocation() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .write_stdin("0\n")
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            document_run_fixture_root().to_str().expect("utf8 path"),
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(stdout.contains("cpass interactive launcher"));
    assert!(stdout.contains("Version:"));
    assert!(stdout.contains("课程列表:"));
    assert!(stdout.contains("输入课程序号 / 课程名 / course_id"));
    assert!(stdout.contains("cpass run TUI"));
    assert!(stdout.contains("Planning: finished for course 1001 (2 chapters, 3 task points)"));
    assert!(stdout.contains("Execution: blocked for course 1001 (3 total entries)"));
}

#[test]
fn top_level_launcher_supports_qr_login_flow() {
    let (temp, config_path) = bootstrap_workspace_without_sessions();
    let fixture_path = temp.path().join("fixture-qr-login");
    copy_fixture_dir(&document_run_fixture_root(), &fixture_path, &[]);
    copy_fixture_dir(&qr_login_fixture_root(), &fixture_path, &[]);
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .write_stdin("\n0\n")
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_path.to_str().expect("utf8 path"),
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(stdout.contains("请输入手机号，留空为二维码登录"));
    assert!(stdout.contains("请使用学习通扫码登录："));
    assert!(stdout.contains("二维码内容 URL:"));
    assert!(stdout.contains("等待扫码确认..."));
    assert!(stdout.contains("cpass run TUI"));
}

#[test]
fn default_tui_rejects_invalid_course_selection() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .write_stdin("99\n")
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
        ])
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("course selection `99` did not match any course"),
        "expected invalid selection error, got: {stderr}"
    );
}

#[test]
fn top_level_launcher_prompts_for_session_selection_when_multiple_sessions_exist() {
    let (_temp, config_path) = bootstrap_workspace_with_multiple_sessions();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .write_stdin("1\n0\n")
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            document_run_fixture_root().to_str().expect("utf8 path"),
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(stdout.contains("可用会话:"));
    assert!(stdout.contains("[0] 13900139000"));
    assert!(stdout.contains("[1] 13800138000"));
    assert!(stdout.contains("选择会话序号"));
    assert!(stdout.contains("课程列表:"));
    assert!(stdout.contains("cpass run TUI"));
}

#[test]
fn rejects_tui_json_combination() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "run",
            "--course-id",
            "1001",
            "--tui",
        ])
        .assert()
        .failure();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(stderr.contains("--tui cannot be combined with --json"));
}

#[test]
fn exports_exam_catalog_with_fixture_transport() {
    let (temp, config_path) = bootstrap_workspace();
    let output_path = temp.path().join("export/exams.json");
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "exam",
            "export",
            "--course-id",
            "1001",
            "--output",
            output_path.to_str().expect("utf8 path"),
        ])
        .assert()
        .success();

    let mut output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert!(output_path.exists());
    assert_eq!(output["exams"].as_array().map(Vec::len), Some(6));
    assert_eq!(
        output["exams"][2]["meta"]["blocked_message"],
        "考试尚未开始"
    );
    assert_eq!(
        output["exams"][5]["meta"]["blocked_message"],
        "该试卷只允许在电脑考试客户端考试,完成考试后可在手机端查看"
    );
    output["generated_at"] = serde_json::Value::String("<normalized>".to_owned());
    output["output_path"] = serde_json::Value::String("<normalized>".to_owned());
    assert_eq!(output, read_golden("exam_export.json"));

    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(output_path).expect("manifest")).expect("json");
    manifest["generated_at"] = serde_json::Value::String("<normalized>".to_owned());
    manifest["output_path"] = serde_json::Value::String("<normalized>".to_owned());
    assert_eq!(manifest, read_golden("exam_export.json"));
}

#[test]
fn exports_exam_catalog_with_profile_selected_session_phone() {
    let (temp, config_path, fixture_path) = bootstrap_workspace_with_profile_session_selection();
    let output_path = temp.path().join("automation-export/exams.json");
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--profile",
            "automation",
            "--fixture-dir",
            fixture_path.to_str().expect("utf8 path"),
            "exam",
            "export",
            "--course-id",
            "1001",
            "--output",
            output_path.to_str().expect("utf8 path"),
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output["account"]["phone"], "13900139000");
    assert_eq!(output["course"]["course_id"], 1001);
    assert_eq!(output["exams"].as_array().map(Vec::len), Some(6));

    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(output_path).expect("manifest")).expect("json");
    assert_eq!(manifest["account"]["phone"], "13900139000");
}

#[test]
fn shows_exam_snapshot_with_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "exam",
            "show",
            "--course-id",
            "1001",
            "--exam-id",
            "555001",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output, read_golden("exam_show_555001.json"));
}

#[test]
fn shows_exam_snapshot_with_index_selectors() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "exam",
            "show",
            "--course-index",
            "0",
            "--exam-index",
            "0",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output, read_golden("exam_show_555001.json"));
}

#[test]
fn shows_blocked_exam_snapshot_with_fixture_transport() {
    let (_temp, config_path) = bootstrap_workspace();
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "exam",
            "show",
            "--course-id",
            "1001",
            "--exam-id",
            "555003",
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert_eq!(output, read_golden("exam_show_555003.json"));
}

#[test]
fn exports_exam_preview_inventory_with_fixture_transport() {
    let (temp, config_path) = bootstrap_workspace();
    let output_path = temp.path().join("export/exam-preview.json");
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            fixture_root().to_str().expect("utf8 path"),
            "exam",
            "preview",
            "export",
            "--course-id",
            "1001",
            "--exam-id",
            "555001",
            "--output",
            output_path.to_str().expect("utf8 path"),
        ])
        .assert()
        .success();

    let mut output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert!(output_path.exists());
    output["generated_at"] = serde_json::Value::String("<normalized>".to_owned());
    output["output_path"] = serde_json::Value::String("<normalized>".to_owned());
    assert_eq!(output, read_golden("exam_preview_export.json"));

    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(output_path).expect("manifest")).expect("json");
    manifest["generated_at"] = serde_json::Value::String("<normalized>".to_owned());
    manifest["output_path"] = serde_json::Value::String("<normalized>".to_owned());
    assert_eq!(manifest, read_golden("exam_preview_export.json"));
}

#[test]
fn exports_exam_preview_inventory_with_rich_option_fixture_transport() {
    let (temp, config_path) = bootstrap_workspace();
    let output_path = temp.path().join("export/exam-preview-rich.json");
    let assert = Command::cargo_bin("cpass")
        .expect("binary")
        .args([
            "--json",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--fixture-dir",
            rich_option_fixture_root().to_str().expect("utf8 path"),
            "exam",
            "preview",
            "export",
            "--course-id",
            "1001",
            "--exam-id",
            "555001",
            "--output",
            output_path.to_str().expect("utf8 path"),
        ])
        .assert()
        .success();

    let mut output: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json output");
    assert!(output_path.exists());
    assert_eq!(output["questions"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        output["questions"][0]["options"][0]["rich_content"]["image_urls"][0],
        serde_json::Value::String("https://static.example/a.png".to_owned())
    );
    assert_eq!(
        output["questions"][0]["options"][1]["rich_content"]["image_urls"][0],
        serde_json::Value::String("https://static.example/b.png".to_owned())
    );
    output["generated_at"] = serde_json::Value::String("<normalized>".to_owned());
    output["output_path"] = serde_json::Value::String("<normalized>".to_owned());
    assert_eq!(output, read_golden("exam_preview_export_rich_options.json"));

    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(output_path).expect("manifest")).expect("json");
    manifest["generated_at"] = serde_json::Value::String("<normalized>".to_owned());
    manifest["output_path"] = serde_json::Value::String("<normalized>".to_owned());
    assert_eq!(
        manifest,
        read_golden("exam_preview_export_rich_options.json")
    );
}
