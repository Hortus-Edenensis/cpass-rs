use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cpass_core::config::NotificationConfig;
use cpass_core::event::{CourseRunExecutionState, RunEvent, RunEventSink};
use cpass_core::{CpassError, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, ServerName},
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NotificationSummary {
    pub kind: String,
    pub level: String,
    pub title: String,
    pub body: String,
    pub warning_count: usize,
    pub retry_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationPlan {
    pub config: NotificationConfig,
    pub summary: NotificationSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GotifyConfig {
    base_url: Url,
    token: String,
    priority: i64,
    fixture_response_path: Option<PathBuf>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MqttConfig {
    broker_url: Url,
    host: String,
    port: u16,
    transport: MqttTransport,
    client_id: Option<String>,
    username: Option<String>,
    password: Option<String>,
    topic: String,
    qos: u8,
    retain: bool,
    fixture_response_path: Option<PathBuf>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MqttTransport {
    Tcp,
    Tls,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MqttLiveConfig {
    broker_url: Url,
    host: String,
    port: u16,
    transport: MqttTransport,
    client_id: Option<String>,
    username: Option<String>,
    password: Option<String>,
    topic: String,
    qos: u8,
    retain: bool,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MqttDeliveryMode {
    Fixture(PathBuf),
    Live(MqttLiveConfig),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct GotifyMessageRequest {
    title: String,
    message: String,
    priority: i64,
    extras: GotifyMessageExtras,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct GotifyMessageExtras {
    #[serde(rename = "client::display")]
    client_display: GotifyClientDisplay,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct GotifyClientDisplay {
    #[serde(rename = "contentType")]
    content_type: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct GotifyMessageResponse {
    id: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct MqttNotificationPayload {
    schema: String,
    kind: String,
    level: String,
    title: String,
    body: String,
    warning_count: usize,
    retry_count: usize,
    warnings: Vec<String>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MqttPublishRequest {
    topic: String,
    qos: u8,
    retain: bool,
    payload: String,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct MqttPublishResponse {
    status: String,
    topic: String,
    qos: u8,
    retain: bool,
}

const MQTT_EXTRA_ROOT_CERT_DER_ENV: &str = "CPASS_MQTT_EXTRA_ROOT_CERT_DER";

trait AsyncIo: AsyncRead + AsyncWrite {}

impl<T> AsyncIo for T where T: AsyncRead + AsyncWrite + ?Sized {}

type BoxedAsyncIo = Box<dyn AsyncIo + Send + Unpin>;

fn encode_mqtt_string(value: &str) -> Result<Vec<u8>> {
    let bytes = value.as_bytes();
    let length = u16::try_from(bytes.len()).map_err(|_| {
        CpassError::Validation(format!("mqtt string value exceeds {} bytes", u16::MAX))
    })?;

    let mut encoded = Vec::with_capacity(2 + bytes.len());
    encoded.extend_from_slice(&length.to_be_bytes());
    encoded.extend_from_slice(bytes);
    Ok(encoded)
}

fn encode_mqtt_remaining_length(remaining_length: usize) -> Result<Vec<u8>> {
    if remaining_length > 268_435_455 {
        return Err(CpassError::Validation(format!(
            "mqtt packet remaining length {remaining_length} exceeds MQTT 3.1.1 maximum"
        )));
    }

    let mut encoded = Vec::new();
    let mut value = remaining_length;
    loop {
        let mut byte = u8::try_from(value % 128).expect("mqtt remaining length chunk fits in u8");
        value /= 128;
        if value > 0 {
            byte |= 0x80;
        }
        encoded.push(byte);
        if value == 0 {
            break;
        }
    }

    Ok(encoded)
}

fn build_mqtt_packet(control_byte: u8, variable_header: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    let remaining_length = variable_header.len() + payload.len();
    let mut packet = Vec::with_capacity(1 + 4 + remaining_length);
    packet.push(control_byte);
    packet.extend(encode_mqtt_remaining_length(remaining_length)?);
    packet.extend_from_slice(variable_header);
    packet.extend_from_slice(payload);
    Ok(packet)
}

fn validate_packet_id(
    control: u8,
    expected_control: u8,
    payload: &[u8],
    expected_packet_id: u16,
    packet_name: &str,
) -> Result<()> {
    if control != expected_control {
        return Err(CpassError::UnexpectedResponse(format!(
            "expected mqtt {packet_name} packet control=0x{expected_control:02x}, got 0x{control:02x}"
        )));
    }

    if payload.len() != 2 {
        return Err(CpassError::UnexpectedResponse(format!(
            "expected mqtt {packet_name} packet payload length 2, got {}",
            payload.len()
        )));
    }

    let packet_id = u16::from_be_bytes([payload[0], payload[1]]);
    if packet_id != expected_packet_id {
        return Err(CpassError::UnexpectedResponse(format!(
            "expected mqtt {packet_name} packet id {expected_packet_id}, got {packet_id}"
        )));
    }

    Ok(())
}

#[async_trait]
trait GotifyBackend: Send + Sync {
    async fn send(&self, url: Url, request: &GotifyMessageRequest)
    -> Result<GotifyMessageResponse>;
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone)]
struct ReqwestGotifyBackend {
    client: reqwest::Client,
}

impl ReqwestGotifyBackend {
    fn new(timeout: Duration) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().timeout(timeout).build()?,
        })
    }
}

#[async_trait]
impl GotifyBackend for ReqwestGotifyBackend {
    async fn send(
        &self,
        url: Url,
        request: &GotifyMessageRequest,
    ) -> Result<GotifyMessageResponse> {
        let response = self
            .client
            .post(url)
            .json(request)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json::<GotifyMessageResponse>().await?)
    }
}

#[derive(Debug, Clone)]
struct FixtureGotifyBackend {
    response: GotifyMessageResponse,
}

impl FixtureGotifyBackend {
    fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let response = serde_json::from_str(&fs::read_to_string(path.as_ref())?)?;
        Ok(Self { response })
    }
}

#[async_trait]
impl GotifyBackend for FixtureGotifyBackend {
    async fn send(
        &self,
        _url: Url,
        _request: &GotifyMessageRequest,
    ) -> Result<GotifyMessageResponse> {
        Ok(self.response.clone())
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[async_trait]
trait MqttBackend: Send + Sync {
    async fn publish(&self, request: &MqttPublishRequest) -> Result<MqttPublishResponse>;
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone)]
struct FixtureMqttBackend {
    response: MqttPublishResponse,
}

#[cfg_attr(not(test), allow(dead_code))]
impl FixtureMqttBackend {
    fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let response = serde_json::from_str(&fs::read_to_string(path.as_ref())?)?;
        Ok(Self { response })
    }
}

#[async_trait]
impl MqttBackend for FixtureMqttBackend {
    async fn publish(&self, _request: &MqttPublishRequest) -> Result<MqttPublishResponse> {
        Ok(self.response.clone())
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone)]
struct LiveMqttBackend {
    config: MqttLiveConfig,
    timeout: Duration,
}

#[cfg_attr(not(test), allow(dead_code))]
impl LiveMqttBackend {
    const KEEP_ALIVE_SECS: u16 = 30;
    const PACKET_ID: u16 = 1;

    fn new(config: MqttLiveConfig, timeout: Duration) -> Self {
        Self { config, timeout }
    }

    async fn connect_stream(&self) -> Result<BoxedAsyncIo> {
        let tcp: TcpStream = self
            .with_timeout(
                "connect",
                TcpStream::connect((self.config.host.as_str(), self.config.port)),
            )
            .await?;
        tcp.set_nodelay(true)?;

        if matches!(self.config.transport, MqttTransport::Tcp) {
            return Ok(Box::new(tcp));
        }

        let root_store = load_mqtt_root_store()?;
        let tls_config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(tls_config));
        let server_name = ServerName::try_from(self.config.host.clone()).map_err(|error| {
            CpassError::Config(format!(
                "mqtt broker host '{}' is not a valid TLS server name: {error}",
                self.config.host
            ))
        })?;
        let tls_stream = self
            .with_timeout("tls handshake", connector.connect(server_name, tcp))
            .await?;
        Ok(Box::new(tls_stream))
    }

    async fn with_timeout<T>(
        &self,
        action: &str,
        future: impl std::future::Future<Output = std::io::Result<T>>,
    ) -> Result<T> {
        timeout(self.timeout, future)
            .await
            .map_err(|_| {
                CpassError::Config(format!(
                    "mqtt {action} timed out after {}s",
                    self.timeout.as_secs()
                ))
            })?
            .map_err(CpassError::Io)
    }

    fn build_connect_packet(&self) -> Result<Vec<u8>> {
        let client_id = self
            .config
            .client_id
            .clone()
            .unwrap_or_else(|| "cpass-rs".to_owned());
        let mut connect_flags = 0b0000_0010;
        let mut payload = encode_mqtt_string(&client_id)?;

        if let Some(username) = &self.config.username {
            connect_flags |= 0b1000_0000;
            payload.extend(encode_mqtt_string(username)?);
        }

        if let Some(password) = &self.config.password {
            connect_flags |= 0b0100_0000;
            payload.extend(encode_mqtt_string(password)?);
        }

        let mut variable_header = encode_mqtt_string("MQTT")?;
        variable_header.push(0x04);
        variable_header.push(connect_flags);
        variable_header.extend_from_slice(&Self::KEEP_ALIVE_SECS.to_be_bytes());

        build_mqtt_packet(0x10, &variable_header, &payload)
    }

    fn build_publish_packet(&self, request: &MqttPublishRequest) -> Result<Vec<u8>> {
        let mut variable_header = encode_mqtt_string(&request.topic)?;
        if request.qos > 0 {
            variable_header.extend_from_slice(&Self::PACKET_ID.to_be_bytes());
        }
        let control_byte = 0x30 | ((request.qos & 0b11) << 1) | u8::from(request.retain);
        build_mqtt_packet(control_byte, &variable_header, request.payload.as_bytes())
    }

    fn build_pubrel_packet(&self) -> Vec<u8> {
        vec![0x62, 0x02, 0x00, 0x01]
    }

    async fn write_packet(&self, stream: &mut BoxedAsyncIo, packet: &[u8]) -> Result<()> {
        self.with_timeout("write", stream.write_all(packet)).await?;
        self.with_timeout("flush", stream.flush()).await
    }

    async fn read_packet(&self, stream: &mut BoxedAsyncIo) -> Result<(u8, Vec<u8>)> {
        let mut control = [0_u8; 1];
        self.with_timeout("read packet header", stream.read_exact(&mut control))
            .await?;
        let remaining_length = self.read_remaining_length(stream).await?;
        let mut payload = vec![0_u8; remaining_length];
        self.with_timeout("read packet payload", stream.read_exact(&mut payload))
            .await?;
        Ok((control[0], payload))
    }

    async fn read_remaining_length(&self, stream: &mut BoxedAsyncIo) -> Result<usize> {
        let mut multiplier = 1_usize;
        let mut value = 0_usize;

        for _ in 0..4 {
            let mut byte = [0_u8; 1];
            self.with_timeout("read remaining length", stream.read_exact(&mut byte))
                .await?;
            value += usize::from(byte[0] & 0x7f) * multiplier;
            if byte[0] & 0x80 == 0 {
                return Ok(value);
            }
            multiplier *= 128;
        }

        Err(CpassError::UnexpectedResponse(
            "mqtt packet remaining length exceeded 4-byte encoding".to_owned(),
        ))
    }

    async fn expect_connack(&self, stream: &mut BoxedAsyncIo) -> Result<()> {
        let (control, payload) = self.read_packet(stream).await?;
        if control != 0x20 || payload.len() != 2 {
            return Err(CpassError::UnexpectedResponse(format!(
                "expected mqtt CONNACK packet, got control=0x{control:02x} len={}",
                payload.len()
            )));
        }
        if payload[1] != 0 {
            return Err(CpassError::UnexpectedResponse(format!(
                "mqtt broker rejected CONNECT with return code {}",
                payload[1]
            )));
        }
        Ok(())
    }

    async fn acknowledge_publish(
        &self,
        stream: &mut BoxedAsyncIo,
        qos: u8,
    ) -> Result<MqttPublishResponse> {
        match qos {
            0 => {}
            1 => {
                let (control, payload) = self.read_packet(stream).await?;
                validate_packet_id(control, 0x40, payload.as_slice(), Self::PACKET_ID, "PUBACK")?;
            }
            2 => {
                let (control, payload) = self.read_packet(stream).await?;
                validate_packet_id(control, 0x50, payload.as_slice(), Self::PACKET_ID, "PUBREC")?;
                self.write_packet(stream, &self.build_pubrel_packet())
                    .await?;
                let (control, payload) = self.read_packet(stream).await?;
                validate_packet_id(
                    control,
                    0x70,
                    payload.as_slice(),
                    Self::PACKET_ID,
                    "PUBCOMP",
                )?;
            }
            other => {
                return Err(CpassError::Validation(format!(
                    "mqtt publish qos '{other}' is not supported; expected 0, 1, or 2"
                )));
            }
        }

        Ok(MqttPublishResponse {
            status: "accepted".to_owned(),
            topic: self.config.topic.clone(),
            qos,
            retain: self.config.retain,
        })
    }

    async fn publish_with_stream(
        &self,
        mut stream: BoxedAsyncIo,
        request: &MqttPublishRequest,
    ) -> Result<MqttPublishResponse> {
        self.write_packet(&mut stream, &self.build_connect_packet()?)
            .await?;
        self.expect_connack(&mut stream).await?;
        self.write_packet(&mut stream, &self.build_publish_packet(request)?)
            .await?;
        let response = self.acknowledge_publish(&mut stream, request.qos).await?;
        self.write_packet(&mut stream, &[0xe0, 0x00]).await?;
        self.with_timeout("shutdown", stream.shutdown()).await?;
        Ok(response)
    }
}

#[async_trait]
impl MqttBackend for LiveMqttBackend {
    async fn publish(&self, request: &MqttPublishRequest) -> Result<MqttPublishResponse> {
        let stream = self.connect_stream().await?;
        self.publish_with_stream(stream, request).await
    }
}

fn load_mqtt_root_store() -> Result<RootCertStore> {
    let mut root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if let Some(path) = std::env::var_os(MQTT_EXTRA_ROOT_CERT_DER_ENV) {
        let path = PathBuf::from(path);
        let certificate = CertificateDer::from(fs::read(&path)?);
        let (added, _ignored) = root_store.add_parsable_certificates([certificate]);
        if added == 0 {
            return Err(CpassError::Config(format!(
                "mqtt extra root cert '{}' did not contain a parsable trust anchor",
                path.display()
            )));
        }
    }

    Ok(root_store)
}

#[derive(Clone)]
struct GotifyNotifier {
    config: GotifyConfig,
    backend: Arc<dyn GotifyBackend>,
}

impl std::fmt::Debug for GotifyNotifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GotifyNotifier")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl GotifyNotifier {
    fn from_config_with_base_dir(
        config: &NotificationConfig,
        config_base_dir: &Path,
        timeout: Duration,
    ) -> Result<Self> {
        let config = GotifyConfig::from_notification_config(config, config_base_dir)?;
        let backend: Arc<dyn GotifyBackend> = if let Some(path) = &config.fixture_response_path {
            Arc::new(FixtureGotifyBackend::from_path(path)?)
        } else {
            Arc::new(ReqwestGotifyBackend::new(timeout)?)
        };

        Ok(Self { config, backend })
    }

    async fn send_summary(&self, summary: &NotificationSummary) -> Result<GotifyMessageResponse> {
        let request = self.build_request(summary);
        self.backend.send(self.message_url(), &request).await
    }

    fn build_request(&self, summary: &NotificationSummary) -> GotifyMessageRequest {
        GotifyMessageRequest {
            title: summary.title.clone(),
            message: render_gotify_message(summary),
            priority: self.config.priority,
            extras: GotifyMessageExtras {
                client_display: GotifyClientDisplay {
                    content_type: "text/plain".to_owned(),
                },
            },
        }
    }

    fn message_url(&self) -> Url {
        let mut url = self.config.base_url.clone();
        url.query_pairs_mut()
            .append_pair("token", self.config.token.as_str());
        url
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone)]
struct MqttNotifier {
    config: MqttConfig,
    backend: Arc<dyn MqttBackend>,
}

impl std::fmt::Debug for MqttNotifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MqttNotifier")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl MqttNotifier {
    fn from_config_with_base_dir(
        config: &NotificationConfig,
        config_base_dir: &Path,
        timeout: Duration,
    ) -> Result<Self> {
        let config = MqttConfig::from_notification_config(config, config_base_dir)?;
        let backend: Arc<dyn MqttBackend> = match config.delivery_mode() {
            MqttDeliveryMode::Fixture(path) => Arc::new(FixtureMqttBackend::from_path(path)?),
            MqttDeliveryMode::Live(live_config) => {
                Arc::new(LiveMqttBackend::new(live_config, timeout))
            }
        };

        Ok(Self { config, backend })
    }

    async fn send_summary(&self, summary: &NotificationSummary) -> Result<MqttPublishResponse> {
        let request = self.build_request(summary)?;
        self.backend.publish(&request).await
    }

    fn build_request(&self, summary: &NotificationSummary) -> Result<MqttPublishRequest> {
        Ok(MqttPublishRequest {
            topic: self.config.topic.clone(),
            qos: self.config.qos,
            retain: self.config.retain,
            payload: render_mqtt_payload(summary)?,
        })
    }
}

impl GotifyConfig {
    fn from_notification_config(
        config: &NotificationConfig,
        config_base_dir: &Path,
    ) -> Result<Self> {
        let base_url = Url::parse(&required_notification_string_field(config, "base_url")?)?;
        let token = required_notification_string_field(config, "token")?;
        let priority = optional_notification_i64_field(config, "priority")?.unwrap_or(0);
        let fixture_response_path =
            resolve_notification_path(config, config_base_dir, "fixture_response_path")?;

        Ok(Self {
            base_url,
            token,
            priority,
            fixture_response_path,
        })
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl MqttConfig {
    fn from_notification_config(
        config: &NotificationConfig,
        config_base_dir: &Path,
    ) -> Result<Self> {
        let broker_url = Url::parse(&required_notification_string_field(config, "broker_url")?)?;
        let host = broker_url
            .host_str()
            .ok_or_else(|| {
                CpassError::Validation(format!(
                    "notification type '{}' field 'broker_url' must include a host",
                    config.kind
                ))
            })?
            .to_owned();
        let transport = mqtt_transport_from_scheme(config, broker_url.scheme())?;
        let port = broker_url.port().unwrap_or(match transport {
            MqttTransport::Tcp => 1883,
            MqttTransport::Tls => 8883,
        });
        let topic = required_notification_string_field(config, "topic")?;
        let client_id = optional_notification_string_field(config, "client_id")?;
        let username = optional_notification_string_field(config, "username")?;
        let password = optional_notification_string_field(config, "password")?;
        if password.is_some() && username.is_none() {
            return Err(CpassError::Validation(format!(
                "notification type '{}' field 'password' requires 'username' to also be set",
                config.kind
            )));
        }
        if !broker_url.username().is_empty() || broker_url.password().is_some() {
            return Err(CpassError::Validation(format!(
                "notification type '{}' field 'broker_url' must not include inline credentials; use 'username'/'password' fields instead",
                config.kind
            )));
        }
        if matches!(broker_url.path(), path if !path.is_empty() && path != "/") {
            return Err(CpassError::Validation(format!(
                "notification type '{}' field 'broker_url' must not include a path",
                config.kind
            )));
        }
        if broker_url.query().is_some() || broker_url.fragment().is_some() {
            return Err(CpassError::Validation(format!(
                "notification type '{}' field 'broker_url' must not include query or fragment components",
                config.kind
            )));
        }
        let qos = optional_notification_u64_field(config, "qos")?.unwrap_or(0);
        let retain = optional_notification_bool_field(config, "retain")?.unwrap_or(false);
        let fixture_response_path =
            resolve_notification_path(config, config_base_dir, "fixture_response_path")?;

        Ok(Self {
            broker_url,
            host,
            port,
            transport,
            client_id,
            username,
            password,
            topic,
            qos: qos as u8,
            retain,
            fixture_response_path,
        })
    }

    fn delivery_mode(&self) -> MqttDeliveryMode {
        match &self.fixture_response_path {
            Some(path) => MqttDeliveryMode::Fixture(path.clone()),
            None => MqttDeliveryMode::Live(self.live_config()),
        }
    }

    fn live_config(&self) -> MqttLiveConfig {
        MqttLiveConfig {
            broker_url: self.broker_url.clone(),
            host: self.host.clone(),
            port: self.port,
            transport: self.transport,
            client_id: self.client_id.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            topic: self.topic.clone(),
            qos: self.qos,
            retain: self.retain,
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn mqtt_transport_from_scheme(config: &NotificationConfig, scheme: &str) -> Result<MqttTransport> {
    match scheme {
        "mqtt" => Ok(MqttTransport::Tcp),
        "mqtts" => Ok(MqttTransport::Tls),
        _ => Err(CpassError::Validation(format!(
            "notification type '{}' field 'broker_url' must use one of [mqtt, mqtts], got '{}'",
            config.kind, scheme
        ))),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
const MQTT_NOTIFICATION_SCHEMA: &str = "cpass.notification.v1";

#[cfg_attr(not(test), allow(dead_code))]
impl MqttNotificationPayload {
    fn from_summary(summary: &NotificationSummary) -> Self {
        Self {
            schema: MQTT_NOTIFICATION_SCHEMA.to_owned(),
            kind: summary.kind.clone(),
            level: summary.level.clone(),
            title: summary.title.clone(),
            body: summary.body.clone(),
            warning_count: summary.warning_count,
            retry_count: summary.retry_count,
            warnings: summary.warnings.clone(),
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn resolve_notification_path(
    config: &NotificationConfig,
    config_base_dir: &Path,
    field_name: &str,
) -> Result<Option<PathBuf>> {
    let Some(path) = optional_notification_string_field(config, field_name)? else {
        return Ok(None);
    };

    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(Some(path))
    } else {
        Ok(Some(config_base_dir.join(path)))
    }
}

fn required_notification_string_field(
    config: &NotificationConfig,
    field_name: &str,
) -> Result<String> {
    optional_notification_string_field(config, field_name)?.ok_or_else(|| {
        CpassError::Validation(format!(
            "notification type '{}' requires string field '{}'",
            config.kind, field_name
        ))
    })
}

fn optional_notification_string_field(
    config: &NotificationConfig,
    field_name: &str,
) -> Result<Option<String>> {
    match config.values.get(field_name) {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                return Err(CpassError::Validation(format!(
                    "notification type '{}' field '{}' must not be empty",
                    config.kind, field_name
                )));
            }
            Ok(Some(value.to_owned()))
        }
        Some(_) => Err(CpassError::Validation(format!(
            "notification type '{}' field '{}' must be a string",
            config.kind, field_name
        ))),
    }
}

fn optional_notification_i64_field(
    config: &NotificationConfig,
    field_name: &str,
) -> Result<Option<i64>> {
    match config.values.get(field_name) {
        None => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or_else(|| {
            CpassError::Validation(format!(
                "notification type '{}' field '{}' must be an integer",
                config.kind, field_name
            ))
        }),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn optional_notification_u64_field(
    config: &NotificationConfig,
    field_name: &str,
) -> Result<Option<u64>> {
    match config.values.get(field_name) {
        None => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            CpassError::Validation(format!(
                "notification type '{}' field '{}' must be an unsigned integer",
                config.kind, field_name
            ))
        }),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn optional_notification_bool_field(
    config: &NotificationConfig,
    field_name: &str,
) -> Result<Option<bool>> {
    match config.values.get(field_name) {
        None => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or_else(|| {
            CpassError::Validation(format!(
                "notification type '{}' field '{}' must be a boolean",
                config.kind, field_name
            ))
        }),
    }
}

pub struct FanoutRunEventSink {
    sinks: Vec<Arc<dyn RunEventSink>>,
}

impl FanoutRunEventSink {
    #[must_use]
    pub fn new(sinks: Vec<Arc<dyn RunEventSink>>) -> Self {
        Self { sinks }
    }
}

impl RunEventSink for FanoutRunEventSink {
    fn emit(&self, event: RunEvent) {
        for sink in &self.sinks {
            sink.emit(event.clone());
        }
    }
}

#[derive(Default, Clone)]
pub struct NotificationSummaryCollector {
    state: Arc<Mutex<NotificationSummaryState>>,
}

impl NotificationSummaryCollector {
    #[must_use]
    pub fn summary(&self) -> Option<NotificationSummary> {
        self.state
            .lock()
            .expect("notification summary collector poisoned")
            .render_summary()
    }
}

impl RunEventSink for NotificationSummaryCollector {
    fn emit(&self, event: RunEvent) {
        self.state
            .lock()
            .expect("notification summary collector poisoned")
            .apply(event);
    }
}

pub struct NotificationCoordinator {
    configs: Vec<NotificationConfig>,
    collector: Option<Arc<NotificationSummaryCollector>>,
}

impl NotificationCoordinator {
    #[must_use]
    pub fn new(configs: &[NotificationConfig]) -> Self {
        Self {
            configs: configs.to_vec(),
            collector: (!configs.is_empty())
                .then(|| Arc::new(NotificationSummaryCollector::default())),
        }
    }

    #[must_use]
    pub fn wrap_sink(&self, primary: Arc<dyn RunEventSink>) -> Arc<dyn RunEventSink> {
        match &self.collector {
            Some(collector) => Arc::new(FanoutRunEventSink::new(vec![primary, collector.clone()])),
            None => primary,
        }
    }

    #[must_use]
    pub fn plans(&self) -> Vec<NotificationPlan> {
        let Some(collector) = &self.collector else {
            return Vec::new();
        };
        let Some(summary) = collector.summary() else {
            return Vec::new();
        };

        self.configs
            .iter()
            .cloned()
            .map(|config| NotificationPlan {
                config,
                summary: summary.clone(),
            })
            .collect()
    }
}

#[derive(Default)]
struct NotificationSummaryState {
    warnings: Vec<String>,
    retry_count: usize,
    outcome: Option<NotificationOutcome>,
}

impl NotificationSummaryState {
    fn apply(&mut self, event: RunEvent) {
        match event {
            RunEvent::DoctorFinished { checks } => {
                self.outcome = Some(NotificationOutcome::DoctorFinished { checks });
            }
            RunEvent::ConfigValidated { path, searchers } => {
                self.outcome = Some(NotificationOutcome::ConfigValidated { path, searchers });
            }
            RunEvent::LoginSucceeded { phone, puid } => {
                self.outcome = Some(NotificationOutcome::LoginSucceeded { phone, puid });
            }
            RunEvent::CourseRunExecutionFinished {
                course_id,
                state,
                total_entries,
                completed_entries,
                blocked_entries,
            } => {
                self.outcome = Some(NotificationOutcome::CourseRunFinished {
                    course_id,
                    state,
                    total_entries,
                    completed_entries,
                    blocked_entries,
                });
            }
            RunEvent::ExamRunPlanningFinished {
                course_id,
                exam_id,
                questions,
            } => {
                self.outcome = Some(NotificationOutcome::ExamRunPlanningFinished {
                    course_id,
                    exam_id,
                    questions,
                });
            }
            RunEvent::RetryScheduled { .. } => {
                self.retry_count += 1;
            }
            RunEvent::Warning { message } => {
                self.warnings.push(message);
            }
            RunEvent::CommandNotImplemented { command } => {
                self.outcome = Some(NotificationOutcome::CommandNotImplemented {
                    command: command.to_owned(),
                });
            }
            _ => {}
        }
    }

    fn render_summary(&self) -> Option<NotificationSummary> {
        self.outcome
            .as_ref()
            .map(|outcome| outcome.render(&self.warnings, self.retry_count))
    }
}

enum NotificationOutcome {
    DoctorFinished {
        checks: usize,
    },
    ConfigValidated {
        path: String,
        searchers: usize,
    },
    LoginSucceeded {
        phone: String,
        puid: u64,
    },
    CourseRunFinished {
        course_id: u64,
        state: CourseRunExecutionState,
        total_entries: usize,
        completed_entries: usize,
        blocked_entries: usize,
    },
    ExamRunPlanningFinished {
        course_id: u64,
        exam_id: u64,
        questions: usize,
    },
    CommandNotImplemented {
        command: String,
    },
}

impl NotificationOutcome {
    fn render(&self, warnings: &[String], retry_count: usize) -> NotificationSummary {
        match self {
            Self::DoctorFinished { checks } => NotificationSummary {
                kind: "doctor".to_owned(),
                level: "success".to_owned(),
                title: "cpass doctor completed".to_owned(),
                body: format!("checks={checks}"),
                warning_count: warnings.len(),
                retry_count,
                warnings: warnings.to_vec(),
            },
            Self::ConfigValidated { path, searchers } => NotificationSummary {
                kind: "config_validate".to_owned(),
                level: "success".to_owned(),
                title: "cpass config validate completed".to_owned(),
                body: format!("path={path} searchers={searchers}"),
                warning_count: warnings.len(),
                retry_count,
                warnings: warnings.to_vec(),
            },
            Self::LoginSucceeded { phone, puid } => NotificationSummary {
                kind: "login".to_owned(),
                level: "success".to_owned(),
                title: format!("cpass login succeeded for {phone}"),
                body: format!("puid={puid}"),
                warning_count: warnings.len(),
                retry_count,
                warnings: warnings.to_vec(),
            },
            Self::CourseRunFinished {
                course_id,
                state,
                total_entries,
                completed_entries,
                blocked_entries,
            } => NotificationSummary {
                kind: "course_run".to_owned(),
                level: match state {
                    CourseRunExecutionState::Completed => "success",
                    CourseRunExecutionState::Blocked => "warning",
                    CourseRunExecutionState::Pending | CourseRunExecutionState::Running => "info",
                }
                .to_owned(),
                title: format!("cpass run {} for course {course_id}", state.as_str()),
                body: format!(
                    "total_entries={total_entries} completed_entries={completed_entries} blocked_entries={blocked_entries}"
                ),
                warning_count: warnings.len(),
                retry_count,
                warnings: warnings.to_vec(),
            },
            Self::ExamRunPlanningFinished {
                course_id,
                exam_id,
                questions,
            } => NotificationSummary {
                kind: "exam_run_planning".to_owned(),
                level: "success".to_owned(),
                title: format!("cpass exam plan ready for exam {exam_id}"),
                body: format!("course_id={course_id} questions={questions}"),
                warning_count: warnings.len(),
                retry_count,
                warnings: warnings.to_vec(),
            },
            Self::CommandNotImplemented { command } => NotificationSummary {
                kind: "command_not_implemented".to_owned(),
                level: "warning".to_owned(),
                title: format!("cpass command not implemented: {command}"),
                body: "core emitted a command_not_implemented lifecycle event".to_owned(),
                warning_count: warnings.len(),
                retry_count,
                warnings: warnings.to_vec(),
            },
        }
    }
}

fn render_gotify_message(summary: &NotificationSummary) -> String {
    let mut lines = vec![
        format!("kind={}", summary.kind),
        format!("level={}", summary.level),
        summary.body.clone(),
        format!("warning_count={}", summary.warning_count),
        format!("retry_count={}", summary.retry_count),
    ];

    if !summary.warnings.is_empty() {
        lines.push(String::new());
        lines.push("warnings:".to_owned());
        lines.extend(
            summary
                .warnings
                .iter()
                .map(|warning| format!("- {warning}")),
        );
    }

    lines.join("\n")
}

fn render_mqtt_payload(summary: &NotificationSummary) -> Result<String> {
    Ok(serde_json::to_string(
        &MqttNotificationPayload::from_summary(summary),
    )?)
}

pub async fn dispatch_notification_plans(
    plans: &[NotificationPlan],
    config_base_dir: &Path,
    timeout: Duration,
) {
    for plan in plans {
        let result = dispatch_notification_plan(plan, config_base_dir, timeout).await;

        if let Err(error) = result {
            let kind = plan.config.kind.to_ascii_lowercase();
            eprintln!("warning: {kind} notification delivery failed: {error}");
        }
    }
}

async fn dispatch_notification_plan(
    plan: &NotificationPlan,
    config_base_dir: &Path,
    timeout: Duration,
) -> Result<()> {
    if plan.config.kind.eq_ignore_ascii_case("gotify") {
        let notifier =
            GotifyNotifier::from_config_with_base_dir(&plan.config, config_base_dir, timeout)?;
        let _response = notifier.send_summary(&plan.summary).await?;
        return Ok(());
    }

    if plan.config.kind.eq_ignore_ascii_case("mqtt") {
        let notifier =
            MqttNotifier::from_config_with_base_dir(&plan.config, config_base_dir, timeout)?;
        let _response = notifier.send_summary(&plan.summary).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use std::io::Cursor;

    use super::{
        FanoutRunEventSink, GotifyNotifier, LiveMqttBackend, MQTT_EXTRA_ROOT_CERT_DER_ENV,
        MqttConfig, MqttDeliveryMode, MqttLiveConfig, MqttNotifier, MqttPublishRequest,
        MqttTransport, NotificationCoordinator, NotificationPlan, NotificationSummary,
        NotificationSummaryCollector, dispatch_notification_plan, dispatch_notification_plans,
        load_mqtt_root_store, render_gotify_message, render_mqtt_payload,
    };
    use crate::run_output::RunEventBuffer;
    use cpass_core::config::NotificationConfig;
    use cpass_core::event::{CourseRunExecutionState, RunEvent, RunEventSink};
    use reqwest::Url;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, duplex};
    use tokio::task::JoinHandle;
    use tokio_rustls::TlsAcceptor;
    use tokio_rustls::TlsConnector;
    use tokio_rustls::rustls::{
        ClientConfig, RootCertStore, ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
    };
    #[test]
    fn collector_renders_course_run_summary_with_warnings() {
        let collector = NotificationSummaryCollector::default();
        collector.emit(RunEvent::Warning {
            message: "live entries remain blocked".to_owned(),
        });
        collector.emit(RunEvent::CourseRunExecutionFinished {
            course_id: 1001,
            state: CourseRunExecutionState::Blocked,
            total_entries: 4,
            completed_entries: 2,
            blocked_entries: 2,
        });

        let summary = collector.summary().expect("course run summary");
        assert_eq!(summary.kind, "course_run");
        assert_eq!(summary.level, "warning");
        assert_eq!(summary.title, "cpass run blocked for course 1001");
        assert_eq!(
            summary.body,
            "total_entries=4 completed_entries=2 blocked_entries=2"
        );
        assert_eq!(summary.warning_count, 1);
        assert_eq!(summary.warnings, vec!["live entries remain blocked"]);
    }

    #[test]
    fn collector_renders_login_summary() {
        let collector = NotificationSummaryCollector::default();
        collector.emit(RunEvent::LoginSucceeded {
            phone: "13800138000".to_owned(),
            puid: 42,
        });

        let summary = collector.summary().expect("login summary");
        assert_eq!(summary.kind, "login");
        assert_eq!(summary.level, "success");
        assert_eq!(summary.title, "cpass login succeeded for 13800138000");
        assert_eq!(summary.body, "puid=42");
    }

    #[test]
    fn fanout_sink_forwards_events_to_each_subscriber() {
        let left = Arc::new(RunEventBuffer::default());
        let right = Arc::new(RunEventBuffer::default());
        let sink = FanoutRunEventSink::new(vec![
            left.clone() as Arc<dyn RunEventSink>,
            right.clone() as Arc<dyn RunEventSink>,
        ]);

        sink.emit(RunEvent::DoctorStarted);
        sink.emit(RunEvent::DoctorFinished { checks: 3 });

        assert_eq!(left.snapshot(), right.snapshot());
        assert_eq!(left.snapshot().len(), 2);
    }

    #[test]
    fn coordinator_builds_one_plan_per_notification_config() {
        let coordinator = NotificationCoordinator::new(&[
            NotificationConfig {
                kind: "gotify".to_owned(),
                values: BTreeMap::from([
                    (
                        "base_url".to_owned(),
                        serde_json::Value::String("https://gotify.example/message".to_owned()),
                    ),
                    (
                        "token".to_owned(),
                        serde_json::Value::String("demo-token".to_owned()),
                    ),
                ]),
            },
            NotificationConfig {
                kind: "mqtt".to_owned(),
                values: BTreeMap::from([
                    (
                        "broker_url".to_owned(),
                        serde_json::Value::String("mqtts://broker.example:8883".to_owned()),
                    ),
                    (
                        "topic".to_owned(),
                        serde_json::Value::String("cpass/status".to_owned()),
                    ),
                ]),
            },
        ]);
        let primary = Arc::new(RunEventBuffer::default());
        let sink = coordinator.wrap_sink(primary);

        sink.emit(RunEvent::RetryScheduled {
            url: "https://api.example.com".to_owned(),
            attempt: 2,
            reason: "gateway timeout".to_owned(),
        });
        sink.emit(RunEvent::CourseRunExecutionFinished {
            course_id: 1001,
            state: CourseRunExecutionState::Completed,
            total_entries: 2,
            completed_entries: 2,
            blocked_entries: 0,
        });

        let plans = coordinator.plans();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].config.kind, "gotify");
        assert_eq!(plans[1].config.kind, "mqtt");
        assert_eq!(plans[0].summary.kind, "course_run");
        assert_eq!(plans[0].summary.level, "success");
        assert_eq!(plans[0].summary.retry_count, 1);
    }

    #[test]
    fn gotify_message_rendering_keeps_summary_metadata_and_warning_lines() {
        let message = render_gotify_message(&NotificationSummary {
            kind: "course_run".to_owned(),
            level: "warning".to_owned(),
            title: "cpass run blocked".to_owned(),
            body: "total_entries=4 completed_entries=2 blocked_entries=2".to_owned(),
            warning_count: 2,
            retry_count: 1,
            warnings: vec![
                "live entries remain blocked".to_owned(),
                "chapter work stays fail-closed".to_owned(),
            ],
        });

        assert_eq!(
            message,
            "kind=course_run\nlevel=warning\ntotal_entries=4 completed_entries=2 blocked_entries=2\nwarning_count=2\nretry_count=1\n\nwarnings:\n- live entries remain blocked\n- chapter work stays fail-closed"
        );
    }

    #[test]
    fn mqtt_payload_rendering_keeps_summary_metadata_as_json() {
        let payload = render_mqtt_payload(&NotificationSummary {
            kind: "course_run".to_owned(),
            level: "warning".to_owned(),
            title: "cpass run blocked".to_owned(),
            body: "total_entries=4 completed_entries=2 blocked_entries=2".to_owned(),
            warning_count: 2,
            retry_count: 1,
            warnings: vec![
                "live entries remain blocked".to_owned(),
                "chapter work stays fail-closed".to_owned(),
            ],
        })
        .expect("mqtt payload");

        assert_eq!(
            payload,
            "{\"schema\":\"cpass.notification.v1\",\"kind\":\"course_run\",\"level\":\"warning\",\"title\":\"cpass run blocked\",\"body\":\"total_entries=4 completed_entries=2 blocked_entries=2\",\"warning_count\":2,\"retry_count\":1,\"warnings\":[\"live entries remain blocked\",\"chapter work stays fail-closed\"]}"
        );
    }

    #[test]
    fn mqtt_config_preserves_live_delivery_fields_without_fixture_path() {
        let config = NotificationConfig {
            kind: "mqtt".to_owned(),
            values: BTreeMap::from([
                (
                    "broker_url".to_owned(),
                    serde_json::Value::String("mqtts://broker.example".to_owned()),
                ),
                (
                    "topic".to_owned(),
                    serde_json::Value::String("cpass/status".to_owned()),
                ),
                (
                    "client_id".to_owned(),
                    serde_json::Value::String("cpass-rs".to_owned()),
                ),
                (
                    "username".to_owned(),
                    serde_json::Value::String("demo-user".to_owned()),
                ),
                (
                    "password".to_owned(),
                    serde_json::Value::String("demo-password".to_owned()),
                ),
                ("qos".to_owned(), serde_json::Value::Number(2.into())),
                ("retain".to_owned(), serde_json::Value::Bool(true)),
            ]),
        };

        let parsed =
            MqttConfig::from_notification_config(&config, Path::new(".")).expect("mqtt config");

        assert_eq!(parsed.topic, "cpass/status");
        assert_eq!(parsed.qos, 2);
        assert!(parsed.retain);

        match parsed.delivery_mode() {
            MqttDeliveryMode::Live(live) => {
                assert_eq!(live.broker_url.scheme(), "mqtts");
                assert_eq!(live.broker_url.host_str(), Some("broker.example"));
                assert_eq!(live.host, "broker.example");
                assert_eq!(live.port, 8883);
                assert_eq!(live.transport, MqttTransport::Tls);
                assert_eq!(live.client_id.as_deref(), Some("cpass-rs"));
                assert_eq!(live.username.as_deref(), Some("demo-user"));
                assert_eq!(live.password.as_deref(), Some("demo-password"));
                assert_eq!(live.topic, "cpass/status");
                assert_eq!(live.qos, 2);
                assert!(live.retain);
            }
            MqttDeliveryMode::Fixture(path) => {
                panic!(
                    "expected live delivery mode, got fixture path {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn mqtt_config_selects_fixture_delivery_mode_when_fixture_path_is_present() {
        let config = NotificationConfig {
            kind: "mqtt".to_owned(),
            values: BTreeMap::from([
                (
                    "broker_url".to_owned(),
                    serde_json::Value::String("mqtt://broker.example:1883".to_owned()),
                ),
                (
                    "topic".to_owned(),
                    serde_json::Value::String("cpass/status".to_owned()),
                ),
                (
                    "fixture_response_path".to_owned(),
                    serde_json::Value::String("mqtt/response.json".to_owned()),
                ),
            ]),
        };

        let config_base_dir = Path::new("/tmp/cpass-config");
        let parsed = MqttConfig::from_notification_config(&config, config_base_dir)
            .expect("fixture-backed mqtt config");

        assert_eq!(parsed.broker_url.scheme(), "mqtt");
        assert_eq!(parsed.broker_url.host_str(), Some("broker.example"));

        match parsed.delivery_mode() {
            MqttDeliveryMode::Fixture(path) => {
                assert_eq!(path, config_base_dir.join("mqtt/response.json"));
            }
            MqttDeliveryMode::Live(live) => {
                panic!(
                    "expected fixture delivery mode, got live broker {}",
                    live.broker_url
                );
            }
        }
    }

    #[test]
    fn mqtt_config_defaults_plain_broker_port_for_live_delivery_mode() {
        let config = NotificationConfig {
            kind: "mqtt".to_owned(),
            values: BTreeMap::from([
                (
                    "broker_url".to_owned(),
                    serde_json::Value::String("mqtt://broker.example".to_owned()),
                ),
                (
                    "topic".to_owned(),
                    serde_json::Value::String("cpass/status".to_owned()),
                ),
            ]),
        };

        let parsed =
            MqttConfig::from_notification_config(&config, Path::new(".")).expect("mqtt config");

        match parsed.delivery_mode() {
            MqttDeliveryMode::Live(live) => {
                assert_eq!(live.host, "broker.example");
                assert_eq!(live.port, 1883);
                assert_eq!(live.transport, MqttTransport::Tcp);
                assert_eq!(live.topic, "cpass/status");
                assert_eq!(live.qos, 0);
                assert!(!live.retain);
            }
            MqttDeliveryMode::Fixture(path) => {
                panic!(
                    "expected live delivery mode, got fixture path {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn mqtt_notifier_supports_live_backend_without_fixture_response_path() {
        let config = NotificationConfig {
            kind: "mqtt".to_owned(),
            values: BTreeMap::from([
                (
                    "broker_url".to_owned(),
                    serde_json::Value::String("mqtts://broker.example".to_owned()),
                ),
                (
                    "topic".to_owned(),
                    serde_json::Value::String("cpass/status".to_owned()),
                ),
                (
                    "client_id".to_owned(),
                    serde_json::Value::String("cpass-rs".to_owned()),
                ),
                (
                    "username".to_owned(),
                    serde_json::Value::String("demo-user".to_owned()),
                ),
                (
                    "password".to_owned(),
                    serde_json::Value::String("demo-password".to_owned()),
                ),
            ]),
        };

        let notifier = MqttNotifier::from_config_with_base_dir(
            &config,
            Path::new("."),
            Duration::from_secs(5),
        )
        .expect("live mqtt notifier");

        assert_eq!(notifier.config.transport, MqttTransport::Tls);
        assert_eq!(notifier.config.port, 8883);
        assert_eq!(notifier.config.client_id.as_deref(), Some("cpass-rs"));
        assert_eq!(notifier.config.username.as_deref(), Some("demo-user"));
        assert_eq!(notifier.config.password.as_deref(), Some("demo-password"));
        assert!(notifier.config.fixture_response_path.is_none());
    }

    #[test]
    fn live_mqtt_backend_builds_connect_packet_with_credentials() {
        let backend = LiveMqttBackend::new(
            MqttLiveConfig {
                broker_url: Url::parse("mqtts://broker.example").expect("broker url"),
                host: "broker.example".to_owned(),
                port: 8883,
                transport: MqttTransport::Tls,
                client_id: Some("cpass-rs-tests".to_owned()),
                username: Some("demo-user".to_owned()),
                password: Some("demo-password".to_owned()),
                topic: "cpass/status".to_owned(),
                qos: 2,
                retain: true,
            },
            Duration::from_secs(5),
        );

        let packet = backend.build_connect_packet().expect("connect packet");
        let (control, payload) = decode_test_mqtt_packet(&packet);
        assert_eq!(control, 0x10);

        let mut cursor = Cursor::new(payload.as_slice());
        assert_eq!(read_test_mqtt_string(&mut cursor), "MQTT");

        let mut header = [0_u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut header).expect("connect header");
        assert_eq!(header[0], 0x04);
        assert_eq!(header[1], 0b1100_0010);
        assert_eq!(u16::from_be_bytes([header[2], header[3]]), 30);

        assert_eq!(read_test_mqtt_string(&mut cursor), "cpass-rs-tests");
        assert_eq!(read_test_mqtt_string(&mut cursor), "demo-user");
        assert_eq!(read_test_mqtt_string(&mut cursor), "demo-password");
    }

    #[test]
    fn live_mqtt_backend_builds_publish_packet_with_qos_two_and_retain() {
        let backend = LiveMqttBackend::new(
            MqttLiveConfig {
                broker_url: Url::parse("mqtt://broker.example:1883").expect("broker url"),
                host: "broker.example".to_owned(),
                port: 1883,
                transport: MqttTransport::Tcp,
                client_id: Some("cpass-rs-tests".to_owned()),
                username: None,
                password: None,
                topic: "cpass/status".to_owned(),
                qos: 2,
                retain: true,
            },
            Duration::from_secs(5),
        );
        let request = MqttPublishRequest {
            topic: "cpass/status".to_owned(),
            qos: 2,
            retain: true,
            payload: "{\"schema\":\"cpass.notification.v1\"}".to_owned(),
        };

        let packet = backend
            .build_publish_packet(&request)
            .expect("publish packet");
        let (control, payload) = decode_test_mqtt_packet(&packet);
        assert_eq!(control, 0x35);

        let mut cursor = Cursor::new(payload.as_slice());
        assert_eq!(read_test_mqtt_string(&mut cursor), "cpass/status");

        let mut packet_id = [0_u8; 2];
        std::io::Read::read_exact(&mut cursor, &mut packet_id).expect("packet id");
        assert_eq!(u16::from_be_bytes(packet_id), 1);

        let mut body = Vec::new();
        std::io::Read::read_to_end(&mut cursor, &mut body).expect("payload body");
        assert_eq!(
            String::from_utf8(body).expect("utf8 payload"),
            "{\"schema\":\"cpass.notification.v1\"}"
        );
    }

    #[test]
    fn load_mqtt_root_store_adds_extra_root_certificate_from_env() {
        let _guard = mqtt_env_lock().lock().expect("mqtt env lock");
        unsafe {
            std::env::remove_var(MQTT_EXTRA_ROOT_CERT_DER_ENV);
        }
        let baseline = load_mqtt_root_store().expect("baseline root store");
        let baseline_len = baseline.roots.len();

        unsafe {
            std::env::set_var(MQTT_EXTRA_ROOT_CERT_DER_ENV, mqtt_tls_root_fixture_path());
        }
        let with_extra = load_mqtt_root_store().expect("root store with extra cert");
        unsafe {
            std::env::remove_var(MQTT_EXTRA_ROOT_CERT_DER_ENV);
        }

        assert_eq!(with_extra.roots.len(), baseline_len + 1);
    }

    #[tokio::test]
    async fn live_mqtt_backend_publishes_over_plain_duplex_session() {
        let backend = test_live_mqtt_backend(MqttTransport::Tcp);
        let request = test_publish_request();
        let (client, server) = duplex(4096);
        let broker_task = spawn_test_broker(server);

        let response = backend
            .publish_with_stream(Box::new(client), &request)
            .await
            .expect("plain mqtt publish");
        let observation = broker_task.await.expect("plain broker task");

        assert_eq!(response.status, "accepted");
        assert_eq!(response.topic, "cpass/status");
        assert_eq!(response.qos, 2);
        assert!(response.retain);
        assert_eq!(observation.client_id, "cpass-rs-tests");
        assert_eq!(observation.username.as_deref(), Some("demo-user"));
        assert_eq!(observation.password.as_deref(), Some("demo-password"));
        assert_eq!(observation.topic, "cpass/status");
        assert_eq!(observation.qos, 2);
        assert!(observation.retain);
        assert_eq!(observation.payload, request.payload);
    }

    #[tokio::test]
    async fn live_mqtt_backend_publishes_over_tls_duplex_session() {
        let backend = test_live_mqtt_backend(MqttTransport::Tls);
        let request = test_publish_request();
        let (client, server) = duplex(4096);
        let acceptor = test_tls_acceptor();
        let connector = test_tls_connector();
        let server_name = ServerName::try_from("localhost").expect("localhost tls name");
        let broker_task = tokio::spawn(async move {
            let tls_stream = acceptor.accept(server).await.expect("tls accept");
            drive_test_broker_session(tls_stream)
                .await
                .expect("tls broker session")
        });
        let client_tls = connector
            .connect(server_name, client)
            .await
            .expect("tls connect");

        let response = backend
            .publish_with_stream(Box::new(client_tls), &request)
            .await
            .expect("tls mqtt publish");
        let observation = broker_task.await.expect("tls broker task");

        assert_eq!(response.status, "accepted");
        assert_eq!(response.topic, "cpass/status");
        assert_eq!(response.qos, 2);
        assert!(response.retain);
        assert_eq!(observation.client_id, "cpass-rs-tests");
        assert_eq!(observation.username.as_deref(), Some("demo-user"));
        assert_eq!(observation.password.as_deref(), Some("demo-password"));
        assert_eq!(observation.topic, "cpass/status");
        assert_eq!(observation.qos, 2);
        assert!(observation.retain);
        assert_eq!(observation.payload, request.payload);
    }

    #[tokio::test]
    async fn gotify_notifier_replays_fixture_response_from_config_dir() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_gotify_fixture(temp.path());
        let config = NotificationConfig {
            kind: "gotify".to_owned(),
            values: BTreeMap::from([
                (
                    "base_url".to_owned(),
                    serde_json::Value::String("https://gotify.example/message".to_owned()),
                ),
                (
                    "token".to_owned(),
                    serde_json::Value::String("demo-token".to_owned()),
                ),
                ("priority".to_owned(), serde_json::Value::Number(5.into())),
                (
                    "fixture_response_path".to_owned(),
                    serde_json::Value::String("gotify_message_response.json".to_owned()),
                ),
            ]),
        };
        let notifier =
            GotifyNotifier::from_config_with_base_dir(&config, temp.path(), Duration::from_secs(5))
                .expect("fixture-backed notifier");

        let response = notifier
            .send_summary(&NotificationSummary {
                kind: "login".to_owned(),
                level: "success".to_owned(),
                title: "cpass login succeeded".to_owned(),
                body: "puid=42".to_owned(),
                warning_count: 0,
                retry_count: 0,
                warnings: Vec::new(),
            })
            .await
            .expect("fixture response");

        assert_eq!(response.id, 1);
        assert_eq!(
            notifier.message_url().as_str(),
            "https://gotify.example/message?token=demo-token"
        );
        assert_eq!(
            notifier
                .build_request(&NotificationSummary {
                    kind: "login".to_owned(),
                    level: "success".to_owned(),
                    title: "cpass login succeeded".to_owned(),
                    body: "puid=42".to_owned(),
                    warning_count: 0,
                    retry_count: 0,
                    warnings: Vec::new(),
                })
                .priority,
            5
        );
    }

    #[tokio::test]
    async fn mqtt_notifier_replays_fixture_response_from_config_dir() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_mqtt_fixture(temp.path());
        let config = NotificationConfig {
            kind: "mqtt".to_owned(),
            values: BTreeMap::from([
                (
                    "broker_url".to_owned(),
                    serde_json::Value::String("mqtt://broker.example:1883".to_owned()),
                ),
                (
                    "topic".to_owned(),
                    serde_json::Value::String("cpass/status".to_owned()),
                ),
                (
                    "client_id".to_owned(),
                    serde_json::Value::String("cpass-rs".to_owned()),
                ),
                ("qos".to_owned(), serde_json::Value::Number(1.into())),
                ("retain".to_owned(), serde_json::Value::Bool(true)),
                (
                    "fixture_response_path".to_owned(),
                    serde_json::Value::String("mqtt_publish_response.json".to_owned()),
                ),
            ]),
        };
        let notifier =
            MqttNotifier::from_config_with_base_dir(&config, temp.path(), Duration::from_secs(5))
                .expect("fixture-backed mqtt notifier");
        let summary = NotificationSummary {
            kind: "doctor".to_owned(),
            level: "success".to_owned(),
            title: "cpass doctor completed".to_owned(),
            body: "checks=4".to_owned(),
            warning_count: 0,
            retry_count: 0,
            warnings: Vec::new(),
        };

        let response = notifier
            .send_summary(&summary)
            .await
            .expect("mqtt fixture response");
        let request = notifier
            .build_request(&summary)
            .expect("mqtt publish request");

        assert_eq!(
            request.payload,
            "{\"schema\":\"cpass.notification.v1\",\"kind\":\"doctor\",\"level\":\"success\",\"title\":\"cpass doctor completed\",\"body\":\"checks=4\",\"warning_count\":0,\"retry_count\":0,\"warnings\":[]}"
        );
        assert_eq!(request.topic, "cpass/status");
        assert_eq!(request.qos, 1);
        assert!(request.retain);
        assert_eq!(response.status, "accepted");
        assert_eq!(response.topic, "cpass/status");
        assert_eq!(response.qos, 1);
        assert!(response.retain);
    }

    #[tokio::test]
    async fn dispatch_notification_plan_replays_fixture_backed_mqtt_delivery() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_mqtt_fixture(temp.path());
        let plan = NotificationPlan {
            config: NotificationConfig {
                kind: "mqtt".to_owned(),
                values: BTreeMap::from([
                    (
                        "broker_url".to_owned(),
                        serde_json::Value::String("mqtt://broker.example:1883".to_owned()),
                    ),
                    (
                        "topic".to_owned(),
                        serde_json::Value::String("cpass/status".to_owned()),
                    ),
                    ("qos".to_owned(), serde_json::Value::Number(1.into())),
                    ("retain".to_owned(), serde_json::Value::Bool(true)),
                    (
                        "fixture_response_path".to_owned(),
                        serde_json::Value::String("mqtt_publish_response.json".to_owned()),
                    ),
                ]),
            },
            summary: NotificationSummary {
                kind: "doctor".to_owned(),
                level: "success".to_owned(),
                title: "cpass doctor completed".to_owned(),
                body: "checks=4".to_owned(),
                warning_count: 0,
                retry_count: 0,
                warnings: Vec::new(),
            },
        };

        dispatch_notification_plan(&plan, temp.path(), Duration::from_secs(5))
            .await
            .expect("fixture-backed mqtt dispatch");
    }

    #[tokio::test]
    async fn dispatch_notification_plans_accepts_gotify_and_mqtt_backends() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_gotify_fixture(temp.path());
        write_mqtt_fixture(temp.path());
        let plans = vec![
            NotificationPlan {
                config: NotificationConfig {
                    kind: "gotify".to_owned(),
                    values: BTreeMap::from([
                        (
                            "base_url".to_owned(),
                            serde_json::Value::String("https://gotify.example/message".to_owned()),
                        ),
                        (
                            "token".to_owned(),
                            serde_json::Value::String("demo-token".to_owned()),
                        ),
                        (
                            "fixture_response_path".to_owned(),
                            serde_json::Value::String("gotify_message_response.json".to_owned()),
                        ),
                    ]),
                },
                summary: NotificationSummary {
                    kind: "doctor".to_owned(),
                    level: "success".to_owned(),
                    title: "cpass doctor completed".to_owned(),
                    body: "checks=4".to_owned(),
                    warning_count: 0,
                    retry_count: 0,
                    warnings: Vec::new(),
                },
            },
            NotificationPlan {
                config: NotificationConfig {
                    kind: "mqtt".to_owned(),
                    values: BTreeMap::from([
                        (
                            "broker_url".to_owned(),
                            serde_json::Value::String("mqtt://broker.example:1883".to_owned()),
                        ),
                        (
                            "topic".to_owned(),
                            serde_json::Value::String("cpass/status".to_owned()),
                        ),
                        (
                            "fixture_response_path".to_owned(),
                            serde_json::Value::String("mqtt_publish_response.json".to_owned()),
                        ),
                    ]),
                },
                summary: NotificationSummary {
                    kind: "doctor".to_owned(),
                    level: "success".to_owned(),
                    title: "cpass doctor completed".to_owned(),
                    body: "checks=4".to_owned(),
                    warning_count: 0,
                    retry_count: 0,
                    warnings: Vec::new(),
                },
            },
        ];

        dispatch_notification_plans(&plans, temp.path(), Duration::from_secs(5)).await;
    }

    fn decode_test_mqtt_packet(packet: &[u8]) -> (u8, Vec<u8>) {
        let control = packet[0];
        let mut multiplier = 1_usize;
        let mut remaining_length = 0_usize;
        let mut offset = 1_usize;

        loop {
            let byte = packet[offset];
            remaining_length += usize::from(byte & 0x7f) * multiplier;
            offset += 1;
            if byte & 0x80 == 0 {
                break;
            }
            multiplier *= 128;
        }

        (control, packet[offset..offset + remaining_length].to_vec())
    }

    fn read_test_mqtt_string(cursor: &mut Cursor<&[u8]>) -> String {
        let mut length = [0_u8; 2];
        std::io::Read::read_exact(cursor, &mut length).expect("mqtt string length");
        let length = usize::from(u16::from_be_bytes(length));
        let mut value = vec![0_u8; length];
        std::io::Read::read_exact(cursor, &mut value).expect("mqtt string bytes");
        String::from_utf8(value).expect("utf8 mqtt string")
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBrokerObservation {
        client_id: String,
        username: Option<String>,
        password: Option<String>,
        topic: String,
        qos: u8,
        retain: bool,
        payload: String,
    }

    fn mqtt_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn mqtt_tls_root_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mqtt_tls/localhost-root.der")
            .canonicalize()
            .expect("mqtt tls root fixture")
    }

    fn mqtt_tls_cert_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mqtt_tls/localhost-cert.der")
            .canonicalize()
            .expect("mqtt tls cert fixture")
    }

    fn mqtt_tls_key_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mqtt_tls/localhost-key.der")
            .canonicalize()
            .expect("mqtt tls key fixture")
    }

    fn test_live_mqtt_backend(transport: MqttTransport) -> LiveMqttBackend {
        let broker_url = match transport {
            MqttTransport::Tcp => "mqtt://localhost:1883",
            MqttTransport::Tls => "mqtts://localhost:8883",
        };

        LiveMqttBackend::new(
            MqttLiveConfig {
                broker_url: Url::parse(broker_url).expect("broker url"),
                host: "localhost".to_owned(),
                port: match transport {
                    MqttTransport::Tcp => 1883,
                    MqttTransport::Tls => 8883,
                },
                transport,
                client_id: Some("cpass-rs-tests".to_owned()),
                username: Some("demo-user".to_owned()),
                password: Some("demo-password".to_owned()),
                topic: "cpass/status".to_owned(),
                qos: 2,
                retain: true,
            },
            Duration::from_secs(5),
        )
    }

    fn test_publish_request() -> MqttPublishRequest {
        MqttPublishRequest {
            topic: "cpass/status".to_owned(),
            qos: 2,
            retain: true,
            payload: "{\"schema\":\"cpass.notification.v1\",\"kind\":\"config_validate\"}"
                .to_owned(),
        }
    }

    fn test_tls_acceptor() -> TlsAcceptor {
        let cert = fs::read(mqtt_tls_cert_fixture_path()).expect("mqtt tls cert");
        let key = fs::read(mqtt_tls_key_fixture_path()).expect("mqtt tls key");
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert)],
                PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key)),
            )
            .expect("tls server cert");
        TlsAcceptor::from(Arc::new(config))
    }

    fn test_tls_connector() -> TlsConnector {
        let mut roots = RootCertStore::empty();
        let cert =
            CertificateDer::from(fs::read(mqtt_tls_root_fixture_path()).expect("mqtt tls root"));
        let (added, _ignored) = roots.add_parsable_certificates([cert]);
        assert_eq!(added, 1, "expected one parsable mqtt test root cert");
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        TlsConnector::from(Arc::new(config))
    }

    fn spawn_test_broker<S>(stream: S) -> JoinHandle<TestBrokerObservation>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            drive_test_broker_session(stream)
                .await
                .expect("test broker session")
        })
    }

    async fn drive_test_broker_session<S>(mut stream: S) -> std::io::Result<TestBrokerObservation>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let (control, payload) = read_async_test_mqtt_packet(&mut stream).await?;
        assert_eq!(control, 0x10, "expected CONNECT packet");
        let connect = decode_test_connect_payload(&payload);

        write_async_test_mqtt_packet(&mut stream, &[0x20, 0x02, 0x00, 0x00]).await?;

        let (control, payload) = read_async_test_mqtt_packet(&mut stream).await?;
        let publish = decode_test_publish_payload(control, &payload);
        assert_eq!(publish.qos, 2, "expected qos2 publish");
        assert!(publish.retain, "expected retained publish");
        write_async_test_mqtt_packet(&mut stream, &[0x50, 0x02, 0x00, 0x01]).await?;

        let (control, payload) = read_async_test_mqtt_packet(&mut stream).await?;
        assert_eq!(control, 0x62, "expected PUBREL packet");
        assert_eq!(
            payload.as_slice(),
            &[0x00, 0x01],
            "expected PUBREL packet id"
        );
        write_async_test_mqtt_packet(&mut stream, &[0x70, 0x02, 0x00, 0x01]).await?;

        let (control, payload) = read_async_test_mqtt_packet(&mut stream).await?;
        assert_eq!(control, 0xe0, "expected DISCONNECT packet");
        assert!(payload.is_empty(), "expected empty DISCONNECT payload");

        Ok(TestBrokerObservation {
            client_id: connect.client_id,
            username: connect.username,
            password: connect.password,
            topic: publish.topic,
            qos: publish.qos,
            retain: publish.retain,
            payload: publish.payload,
        })
    }

    async fn read_async_test_mqtt_packet<S>(stream: &mut S) -> std::io::Result<(u8, Vec<u8>)>
    where
        S: AsyncRead + Unpin,
    {
        let mut control = [0_u8; 1];
        stream.read_exact(&mut control).await?;
        let remaining_length = read_async_test_remaining_length(stream).await?;
        let mut payload = vec![0_u8; remaining_length];
        stream.read_exact(&mut payload).await?;
        Ok((control[0], payload))
    }

    async fn read_async_test_remaining_length<S>(stream: &mut S) -> std::io::Result<usize>
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

    async fn write_async_test_mqtt_packet<S>(stream: &mut S, packet: &[u8]) -> std::io::Result<()>
    where
        S: AsyncWrite + Unpin,
    {
        stream.write_all(packet).await?;
        stream.flush().await
    }

    fn decode_test_connect_payload(payload: &[u8]) -> TestBrokerObservation {
        let mut cursor = Cursor::new(payload);
        assert_eq!(read_test_mqtt_string(&mut cursor), "MQTT");

        let mut header = [0_u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut header).expect("connect header");
        assert_eq!(header[0], 0x04);
        assert_eq!(header[1], 0b1100_0010);
        assert_eq!(u16::from_be_bytes([header[2], header[3]]), 30);

        TestBrokerObservation {
            client_id: read_test_mqtt_string(&mut cursor),
            username: Some(read_test_mqtt_string(&mut cursor)),
            password: Some(read_test_mqtt_string(&mut cursor)),
            topic: String::new(),
            qos: 0,
            retain: false,
            payload: String::new(),
        }
    }

    fn decode_test_publish_payload(control: u8, payload: &[u8]) -> TestBrokerObservation {
        let qos = (control & 0b0000_0110) >> 1;
        let retain = control & 0b0000_0001 != 0;
        let mut cursor = Cursor::new(payload);
        let topic = read_test_mqtt_string(&mut cursor);

        let mut packet_id = [0_u8; 2];
        std::io::Read::read_exact(&mut cursor, &mut packet_id).expect("packet id");
        assert_eq!(u16::from_be_bytes(packet_id), 1);

        let mut body = Vec::new();
        std::io::Read::read_to_end(&mut cursor, &mut body).expect("payload body");

        TestBrokerObservation {
            client_id: String::new(),
            username: None,
            password: None,
            topic,
            qos,
            retain,
            payload: String::from_utf8(body).expect("utf8 payload"),
        }
    }

    fn write_gotify_fixture(root: &Path) {
        fs::write(
            root.join("gotify_message_response.json"),
            "{\n  \"id\": 1\n}\n",
        )
        .expect("fixture written");
    }

    fn write_mqtt_fixture(root: &Path) {
        fs::write(
            root.join("mqtt_publish_response.json"),
            "{\n  \"status\": \"accepted\",\n  \"topic\": \"cpass/status\",\n  \"qos\": 1,\n  \"retain\": true\n}\n",
        )
        .expect("fixture written");
    }
}
