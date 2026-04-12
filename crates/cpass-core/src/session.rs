use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{CpassError, Result};
use crate::models::{AccountProfile, CookieSnapshot};

pub trait SessionStore: Send + Sync {
    fn save(&self, record: &SessionRecord) -> Result<PathBuf>;
    fn load_latest(&self) -> Result<SessionRecord>;
    fn load_by_phone(&self, phone: &str) -> Result<SessionRecord>;
    fn list(&self) -> Result<Vec<SessionRecord>>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub schema: String,
    pub account: AccountProfile,
    pub cookies: CookieSnapshot,
    pub saved_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct FileSessionStore {
    root: PathBuf,
}

impl FileSessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn session_path(&self, phone: &str) -> PathBuf {
        self.root.join(format!("{phone}.json"))
    }

    fn ensure_root(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        Ok(())
    }

    fn parse_legacy(path: &Path, value: &serde_json::Value) -> Result<SessionRecord> {
        let phone = value
            .get("phone")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                CpassError::UnexpectedResponse(format!(
                    "legacy session missing phone in {}",
                    path.display()
                ))
            })?;
        let puid = value
            .get("puid")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                CpassError::UnexpectedResponse(format!(
                    "legacy session missing puid in {}",
                    path.display()
                ))
            })?;
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let ck = value
            .get("ck")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                CpassError::UnexpectedResponse(format!(
                    "legacy session missing ck in {}",
                    path.display()
                ))
            })?;

        let mut snapshot = CookieSnapshot::empty();
        let hosts = [
            "passport2.chaoxing.com",
            "sso.chaoxing.com",
            "mooc1-api.chaoxing.com",
            "mooc1.chaoxing.com",
        ];
        for host in hosts {
            snapshot.insert(host, ck.to_owned());
        }

        Ok(SessionRecord {
            schema: "legacy.cxkitty.session.v0".to_owned(),
            account: AccountProfile {
                puid,
                name,
                phone: phone.to_owned(),
                school: "unknown".to_owned(),
                sex: None,
                student_id: None,
            },
            cookies: snapshot,
            saved_at: Utc::now(),
        })
    }

    pub fn load_path(path: impl AsRef<Path>) -> Result<SessionRecord> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&content)?;
        if value.get("schema").is_some() {
            Ok(serde_json::from_value(value)?)
        } else {
            Self::parse_legacy(path, &value)
        }
    }
}

impl SessionStore for FileSessionStore {
    fn save(&self, record: &SessionRecord) -> Result<PathBuf> {
        self.ensure_root()?;
        let path = self.session_path(&record.account.phone);
        let json = serde_json::to_string_pretty(record)?;
        fs::write(&path, json)?;
        Ok(path)
    }

    fn load_latest(&self) -> Result<SessionRecord> {
        let mut records = self.list()?;
        records.sort_by(|left, right| right.saved_at.cmp(&left.saved_at));
        records
            .into_iter()
            .next()
            .ok_or_else(|| CpassError::SessionNotFound(self.root.clone()))
    }

    fn load_by_phone(&self, phone: &str) -> Result<SessionRecord> {
        let path = self.session_path(phone);
        if !path.exists() {
            return Err(CpassError::SessionNotFound(path));
        }
        Self::load_path(&path)
    }

    fn list(&self) -> Result<Vec<SessionRecord>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            records.push(Self::load_path(&path)?);
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::{FileSessionStore, SessionStore};
    use std::fs;

    #[test]
    fn reads_legacy_session_format() {
        let temp = std::env::temp_dir().join(format!("cpass-session-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).expect("temp dir");
        let target = temp.join("13800138000.json");
        fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/legacy/legacy_session.json"
            ),
            &target,
        )
        .expect("fixture copied");

        let store = FileSessionStore::new(&temp);
        let record = store.load_latest().expect("legacy session to load");
        assert_eq!(record.account.phone, "13800138000");
        assert_eq!(
            record.cookies.get("passport2.chaoxing.com"),
            Some("_uid=114514; vc3=legacy-cookie; uf=legacy-token;")
        );

        let _ = fs::remove_dir_all(&temp);
    }
}
