use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{CpassError, Result};
use crate::secret::SecretSource;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub paths: AppPaths,
    pub login: LoginConfig,
    pub transport: TransportConfig,
    pub searchers: Vec<SearcherConfig>,
    pub notifications: Vec<NotificationConfig>,
    pub legacy_reference: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppPaths {
    pub session_dir: PathBuf,
    pub log_dir: PathBuf,
    pub export_dir: PathBuf,
    pub face_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LoginConfig {
    pub phone: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransportConfig {
    pub timeout_secs: u64,
    pub retries: u32,
    pub retry_delay_millis: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearcherConfig {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub values: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationConfig {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub values: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigOutput {
    pub config_path: PathBuf,
    pub selected_profile: Option<String>,
    pub automation_paths: AppPaths,
    pub config: AppConfig,
}

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    session_path: Option<PathBuf>,
    log_path: Option<PathBuf>,
    export_path: Option<PathBuf>,
    face_image_path: Option<PathBuf>,
    login: Option<RawLoginConfig>,
    transport: Option<RawTransportConfig>,
    #[serde(default)]
    searchers: Vec<RawSearcherConfig>,
    #[serde(default)]
    notifications: Vec<RawNotificationConfig>,
    #[serde(default)]
    profiles: BTreeMap<String, RawProfileConfig>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct RawSearcherConfig {
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    values: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct RawNotificationConfig {
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    values: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Default)]
struct RawProfileConfig {
    session_path: Option<PathBuf>,
    log_path: Option<PathBuf>,
    export_path: Option<PathBuf>,
    face_image_path: Option<PathBuf>,
    login: Option<RawLoginConfig>,
    transport: Option<RawTransportConfig>,
    searchers: Option<Vec<RawSearcherConfig>>,
    notifications: Option<Vec<RawNotificationConfig>>,
}

#[derive(Debug, Deserialize, Default)]
struct RawLoginConfig {
    phone: Option<String>,
    password: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawTransportConfig {
    timeout_secs: Option<u64>,
    retries: Option<u32>,
    retry_delay_millis: Option<u64>,
}

impl Default for AppPaths {
    fn default() -> Self {
        Self {
            session_dir: PathBuf::from("session"),
            log_dir: PathBuf::from("logs"),
            export_dir: PathBuf::from("export"),
            face_dir: PathBuf::from("faces"),
        }
    }
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 20,
            retries: 3,
            retry_delay_millis: 1_000,
        }
    }
}

impl AppConfig {
    pub fn load_from_path(path: impl AsRef<Path>, secrets: &dyn SecretSource) -> Result<Self> {
        Self::load_from_path_with_profile(path, secrets, None)
    }

    pub fn load_from_path_with_profile(
        path: impl AsRef<Path>,
        secrets: &dyn SecretSource,
        profile: Option<&str>,
    ) -> Result<Self> {
        let path = path.as_ref();
        let raw = if path.exists() {
            serde_yaml::from_str::<RawConfig>(&fs::read_to_string(path)?)?
        } else {
            RawConfig::default()
        };

        let mut config = AppConfig::from_raw(&raw);
        if let Some(profile_name) = normalize_selected_profile(profile)? {
            let selected_profile = raw.profiles.get(profile_name).ok_or_else(|| {
                CpassError::Config(format!(
                    "config profile '{profile_name}' was not found in {}",
                    path.display()
                ))
            })?;
            selected_profile.apply_to(&mut config);
        }

        if let Some(value) = secrets.get("CPASS_SESSION_DIR") {
            config.paths.session_dir = PathBuf::from(value);
        }
        if let Some(value) = secrets.get("CPASS_LOG_DIR") {
            config.paths.log_dir = PathBuf::from(value);
        }
        if let Some(value) = secrets.get("CPASS_EXPORT_DIR") {
            config.paths.export_dir = PathBuf::from(value);
        }
        if let Some(value) = secrets.get("CPASS_FACE_DIR") {
            config.paths.face_dir = PathBuf::from(value);
        }
        if let Some(value) = secrets.get("CPASS_PHONE") {
            config.login.phone = Some(value);
        }
        if let Some(value) = secrets.get("CPASS_PASSWORD") {
            config.login.password = Some(value);
        }
        if let Some(value) = secrets.get("CPASS_REQUEST_TIMEOUT_SECS") {
            config.transport.timeout_secs = value.parse().map_err(|_| {
                CpassError::Config("CPASS_REQUEST_TIMEOUT_SECS must be an integer".to_owned())
            })?;
        }
        if let Some(value) = secrets.get("CPASS_REQUEST_RETRIES") {
            config.transport.retries = value.parse().map_err(|_| {
                CpassError::Config("CPASS_REQUEST_RETRIES must be an integer".to_owned())
            })?;
        }
        if let Some(value) = secrets.get("CPASS_OPENAI_API_KEY") {
            for searcher in &mut config.searchers {
                if searcher.kind.eq_ignore_ascii_case("OpenAISearcher")
                    || searcher.kind.eq_ignore_ascii_case("openai")
                    || searcher.kind.eq_ignore_ascii_case("openai-compatible")
                {
                    searcher.values.insert(
                        "api_key".to_owned(),
                        serde_json::Value::String(value.clone()),
                    );
                }
            }
        }

        config.validate()?;
        Ok(config)
    }

    fn from_raw(raw: &RawConfig) -> Self {
        let mut config = AppConfig {
            paths: AppPaths {
                session_dir: raw
                    .session_path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("session")),
                log_dir: raw
                    .log_path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("logs")),
                export_dir: raw
                    .export_path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("export")),
                face_dir: raw
                    .face_image_path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("faces")),
            },
            login: LoginConfig::default(),
            transport: TransportConfig::default(),
            searchers: raw
                .searchers
                .iter()
                .cloned()
                .map(SearcherConfig::from)
                .collect(),
            notifications: raw
                .notifications
                .iter()
                .cloned()
                .map(NotificationConfig::from)
                .collect(),
            legacy_reference: true,
        };
        if let Some(login) = &raw.login {
            login.apply_to(&mut config.login);
        }
        if let Some(transport) = &raw.transport {
            transport.apply_to(&mut config.transport);
        }
        config
    }

    pub fn validate(&self) -> Result<()> {
        for (name, path) in [
            ("session_dir", &self.paths.session_dir),
            ("log_dir", &self.paths.log_dir),
            ("export_dir", &self.paths.export_dir),
            ("face_dir", &self.paths.face_dir),
        ] {
            if path.as_os_str().is_empty() {
                return Err(CpassError::Validation(format!("{name} must not be empty")));
            }
        }

        if self.transport.timeout_secs == 0 {
            return Err(CpassError::Validation(
                "transport.timeout_secs must be greater than 0".to_owned(),
            ));
        }
        if self.transport.retries > 10 {
            return Err(CpassError::Validation(
                "transport.retries must be between 0 and 10".to_owned(),
            ));
        }

        for notification in &self.notifications {
            validate_notification_config(notification)?;
        }

        Ok(())
    }
}

impl From<RawSearcherConfig> for SearcherConfig {
    fn from(value: RawSearcherConfig) -> Self {
        Self {
            kind: value.kind,
            values: value.values,
        }
    }
}

impl From<RawNotificationConfig> for NotificationConfig {
    fn from(value: RawNotificationConfig) -> Self {
        Self {
            kind: value.kind,
            values: value.values,
        }
    }
}

impl RawProfileConfig {
    fn apply_to(&self, config: &mut AppConfig) {
        if let Some(path) = &self.session_path {
            config.paths.session_dir = path.clone();
        }
        if let Some(path) = &self.log_path {
            config.paths.log_dir = path.clone();
        }
        if let Some(path) = &self.export_path {
            config.paths.export_dir = path.clone();
        }
        if let Some(path) = &self.face_image_path {
            config.paths.face_dir = path.clone();
        }
        if let Some(login) = &self.login {
            login.apply_to(&mut config.login);
        }
        if let Some(transport) = &self.transport {
            transport.apply_to(&mut config.transport);
        }
        if let Some(searchers) = &self.searchers {
            config.searchers = searchers
                .iter()
                .cloned()
                .map(SearcherConfig::from)
                .collect();
        }
        if let Some(notifications) = &self.notifications {
            config.notifications = notifications
                .iter()
                .cloned()
                .map(NotificationConfig::from)
                .collect();
        }
    }
}

impl RawLoginConfig {
    fn apply_to(&self, login: &mut LoginConfig) {
        if let Some(phone) = &self.phone {
            login.phone = Some(phone.clone());
        }
        if let Some(password) = &self.password {
            login.password = Some(password.clone());
        }
    }
}

impl RawTransportConfig {
    fn apply_to(&self, transport: &mut TransportConfig) {
        if let Some(timeout_secs) = self.timeout_secs {
            transport.timeout_secs = timeout_secs;
        }
        if let Some(retries) = self.retries {
            transport.retries = retries;
        }
        if let Some(retry_delay_millis) = self.retry_delay_millis {
            transport.retry_delay_millis = retry_delay_millis;
        }
    }
}

fn normalize_selected_profile(profile: Option<&str>) -> Result<Option<&str>> {
    match profile.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => Ok(Some(value)),
    }
}

fn validate_notification_config(config: &NotificationConfig) -> Result<()> {
    if config.kind.eq_ignore_ascii_case("gotify") {
        validate_notification_url(
            &required_notification_string_field(config, "base_url")?,
            &["http", "https"],
            config,
            "base_url",
        )?;
        let _ = required_notification_string_field(config, "token")?;
        let _ = optional_notification_string_field(config, "fixture_response_path")?;
        if let Some(priority) = optional_notification_i64_field(config, "priority")?
            && !(-2..=10).contains(&priority)
        {
            return Err(CpassError::Validation(format!(
                "notification type '{}' field 'priority' must be between -2 and 10",
                config.kind
            )));
        }

        return Ok(());
    }

    if config.kind.eq_ignore_ascii_case("mqtt") {
        validate_notification_url(
            &required_notification_string_field(config, "broker_url")?,
            &["mqtt", "mqtts"],
            config,
            "broker_url",
        )?;
        let _ = required_notification_string_field(config, "topic")?;
        let _ = optional_notification_string_field(config, "client_id")?;
        let _ = optional_notification_string_field(config, "username")?;
        let _ = optional_notification_string_field(config, "password")?;
        let _ = optional_notification_string_field(config, "fixture_response_path")?;
        let _ = optional_notification_bool_field(config, "retain")?;
        if let Some(qos) = optional_notification_u64_field(config, "qos")?
            && qos > 2
        {
            return Err(CpassError::Validation(format!(
                "notification type '{}' field 'qos' must be between 0 and 2",
                config.kind
            )));
        }

        return Ok(());
    }

    Err(CpassError::Validation(format!(
        "notification type '{}' is not supported; expected gotify or mqtt",
        config.kind
    )))
}

fn validate_notification_url(
    url: &str,
    expected_schemes: &[&str],
    config: &NotificationConfig,
    field_name: &str,
) -> Result<()> {
    let parsed = Url::parse(url).map_err(|error| {
        CpassError::Validation(format!(
            "notification type '{}' field '{}' must be a valid absolute URL: {error}",
            config.kind, field_name
        ))
    })?;

    if expected_schemes.contains(&parsed.scheme()) {
        Ok(())
    } else {
        Err(CpassError::Validation(format!(
            "notification type '{}' field '{}' must use one of [{}], got '{}'",
            config.kind,
            field_name,
            expected_schemes.join(", "),
            parsed.scheme()
        )))
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

#[cfg(test)]
mod tests {
    use super::AppConfig;
    use crate::secret::SecretSource;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct TestSecrets {
        values: BTreeMap<String, String>,
    }

    impl SecretSource for TestSecrets {
        fn get(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }
    }

    #[test]
    fn loads_defaults_when_config_is_missing() {
        let config = AppConfig::load_from_path(
            PathBuf::from("definitely-missing-config.yml"),
            &TestSecrets::default(),
        )
        .expect("config to load");
        assert_eq!(config.paths.session_dir, PathBuf::from("session"));
        assert_eq!(config.transport.timeout_secs, 20);
        assert!(config.notifications.is_empty());
    }

    #[test]
    fn applies_environment_overrides() {
        let mut secrets = TestSecrets::default();
        secrets
            .values
            .insert("CPASS_SESSION_DIR".to_owned(), "tmp/sessions".to_owned());
        secrets
            .values
            .insert("CPASS_REQUEST_RETRIES".to_owned(), "5".to_owned());
        let config =
            AppConfig::load_from_path(PathBuf::from("definitely-missing-config.yml"), &secrets)
                .expect("config to load");
        assert_eq!(config.paths.session_dir, PathBuf::from("tmp/sessions"));
        assert_eq!(config.transport.retries, 5);
    }

    #[test]
    fn loads_selected_profile_with_partial_overrides() {
        let path = write_temp_config(
            r#"
session_path: "session/base"
log_path: "logs/base"
export_path: "export/base"
face_image_path: "faces/base"
login:
  phone: "13800138000"
transport:
  timeout_secs: 30
  retries: 2
searchers:
  - type: json
    file_path: "base.json"
notifications:
  - type: gotify
    base_url: "https://gotify.example/message"
    token: "base-token"
profiles:
  automation:
    session_path: "session/automation"
    login:
      password: "profile-password"
    transport:
      retries: 5
    searchers:
      - type: sqlite
        file_path: "profile.db"
"#,
        );

        let config = AppConfig::load_from_path_with_profile(
            &path,
            &TestSecrets::default(),
            Some("automation"),
        )
        .expect("config with selected profile");

        assert_eq!(
            config.paths.session_dir,
            PathBuf::from("session/automation")
        );
        assert_eq!(config.paths.log_dir, PathBuf::from("logs/base"));
        assert_eq!(config.login.phone.as_deref(), Some("13800138000"));
        assert_eq!(config.login.password.as_deref(), Some("profile-password"));
        assert_eq!(config.transport.timeout_secs, 30);
        assert_eq!(config.transport.retries, 5);
        assert_eq!(config.searchers.len(), 1);
        assert_eq!(config.searchers[0].kind, "sqlite");
        assert_eq!(
            config.searchers[0].values.get("file_path"),
            Some(&serde_json::Value::String("profile.db".to_owned()))
        );
        assert_eq!(config.notifications.len(), 1);
        assert_eq!(config.notifications[0].kind, "gotify");

        remove_temp_config(&path);
    }

    #[test]
    fn rejects_unknown_selected_profile() {
        let path = write_temp_config(
            r#"
profiles:
  automation:
    session_path: "session/automation"
"#,
        );

        let error =
            AppConfig::load_from_path_with_profile(&path, &TestSecrets::default(), Some("missing"))
                .expect_err("missing profile");

        assert!(
            error
                .to_string()
                .contains("config profile 'missing' was not found")
        );

        remove_temp_config(&path);
    }

    #[test]
    fn loads_notification_configs() {
        let path = write_temp_config(
            r#"
session_path: "session/"
log_path: "logs/"
export_path: "export/"
face_image_path: "faces/"
notifications:
  - type: gotify
    base_url: "https://gotify.example/message"
    token: "demo-token"
    priority: 5
  - type: mqtt
    broker_url: "mqtts://broker.example:8883"
    topic: "cpass/status"
    client_id: "cpass-rs"
    qos: 1
    retain: true
"#,
        );

        let config = AppConfig::load_from_path(&path, &TestSecrets::default())
            .expect("config with notifications");

        assert_eq!(config.notifications.len(), 2);
        assert_eq!(config.notifications[0].kind, "gotify");
        assert_eq!(config.notifications[1].kind, "mqtt");

        remove_temp_config(&path);
    }

    #[test]
    fn rejects_unknown_notification_types() {
        let path = write_temp_config(
            r#"
notifications:
  - type: webhook
    url: "https://example.com/hook"
"#,
        );

        let error = AppConfig::load_from_path(&path, &TestSecrets::default())
            .expect_err("unknown notification type");

        assert!(
            error
                .to_string()
                .contains("notification type 'webhook' is not supported")
        );

        remove_temp_config(&path);
    }

    #[test]
    fn rejects_invalid_mqtt_notification_qos() {
        let path = write_temp_config(
            r#"
notifications:
  - type: mqtt
    broker_url: "mqtt://broker.example"
    topic: "cpass/status"
    qos: 3
"#,
        );

        let error = AppConfig::load_from_path(&path, &TestSecrets::default())
            .expect_err("invalid mqtt qos");

        assert!(
            error
                .to_string()
                .contains("notification type 'mqtt' field 'qos' must be between 0 and 2")
        );

        remove_temp_config(&path);
    }

    #[test]
    fn rejects_non_string_mqtt_fixture_response_path() {
        let path = write_temp_config(
            r#"
notifications:
  - type: mqtt
    broker_url: "mqtt://broker.example"
    topic: "cpass/status"
    fixture_response_path: true
"#,
        );

        let error = AppConfig::load_from_path(&path, &TestSecrets::default())
            .expect_err("invalid mqtt fixture path");

        assert!(
            error.to_string().contains(
                "notification type 'mqtt' field 'fixture_response_path' must be a string"
            )
        );

        remove_temp_config(&path);
    }

    #[test]
    fn rejects_non_string_gotify_fixture_response_path() {
        let path = write_temp_config(
            r#"
notifications:
  - type: gotify
    base_url: "https://gotify.example/message"
    token: "demo-token"
    fixture_response_path: true
"#,
        );

        let error = AppConfig::load_from_path(&path, &TestSecrets::default())
            .expect_err("invalid gotify fixture path");

        assert!(
            error.to_string().contains(
                "notification type 'gotify' field 'fixture_response_path' must be a string"
            )
        );

        remove_temp_config(&path);
    }

    fn write_temp_config(contents: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "cpass-config-test-{}-{unique}.yml",
            std::process::id()
        ));
        fs::write(&path, contents).expect("write temp config");
        path
    }

    fn remove_temp_config(path: &PathBuf) {
        let _ = fs::remove_file(path);
    }
}
