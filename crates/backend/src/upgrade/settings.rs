//! Better versions settings (non-secret, in `server_setting`) and the
//! sealed secrets (FETCH agent token, Cloudflare Access service token,
//! enhancer shared secret), which never leave the server.

use diesel::prelude::*;
use diesel::sql_types::{Binary, Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};

pub use crate::library::tools::marks::SETTINGS_KEY;

/// FETCH on the local network.
pub const DEFAULT_FETCH_URL: &str = "http://192.168.1.58:8490";
/// The enhancer worker's default port on the GPU PC.
pub const DEFAULT_ENHANCER_URL: &str = "http://192.168.1.58:8765";
/// Most automatic requests a day that can be configured.
pub const MAX_AUTO_PER_DAY: u32 = 50;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FetchSettings {
    /// Talk to FETCH at all.
    pub enabled: bool,
    pub base_url: String,
    /// The Mujin library root FETCH saves music into (`Music/<Artist>/
    /// <Album>/` on its share). `None`: rescan the whole library.
    pub library_root: Option<String>,
    /// Request lossless copies in the background, without asking.
    pub auto_request: bool,
    /// Most automatic requests per day.
    pub auto_per_day: u32,
    /// Also list 16-bit/44.1 kHz lossless albums as hi-res candidates.
    pub include_cd_quality: bool,
    /// Move replaced lossy files into `replaced_folder` after a verified
    /// lossless copy arrived (never deleted).
    pub move_replaced: bool,
    pub replaced_folder: Option<String>,
}

impl Default for FetchSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: DEFAULT_FETCH_URL.to_string(),
            library_root: None,
            auto_request: false,
            auto_per_day: 5,
            include_cd_quality: false,
            move_replaced: false,
            replaced_folder: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnhancerSettings {
    pub enabled: bool,
    pub base_url: String,
    /// Where AI-enhanced files are written. `None`: a "Mujin Enhanced"
    /// folder in the first library root.
    pub output_folder: Option<String>,
    /// Rank AI-enhanced copies above the lossy originals.
    pub prefer_enhanced: bool,
    /// The worker can read the library at the same paths (same disk or
    /// share), so Mujin sends paths instead of uploading files.
    pub shared_paths: bool,
}

impl Default for EnhancerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: DEFAULT_ENHANCER_URL.to_string(),
            output_folder: None,
            prefer_enhanced: true,
            shared_paths: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpgradeSettings {
    pub fetch: FetchSettings,
    pub enhancer: EnhancerSettings,
}

/// Checks an `http(s)://host[:port][/path]` base URL and drops a
/// trailing slash. No credentials, query or fragment.
pub fn normalize_base_url(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.as_str().trim_end_matches('/').to_string())
}

impl UpgradeSettings {
    /// Clamps values and checks URLs; `Err` names the field at fault.
    pub fn validated(mut self) -> Result<Self, &'static str> {
        self.fetch.base_url =
            normalize_base_url(&self.fetch.base_url).ok_or("The FETCH address is not valid.")?;
        self.enhancer.base_url = normalize_base_url(&self.enhancer.base_url)
            .ok_or("The enhancer address is not valid.")?;
        self.fetch.auto_per_day = self.fetch.auto_per_day.clamp(1, MAX_AUTO_PER_DAY);
        for folder in [
            &mut self.fetch.library_root,
            &mut self.fetch.replaced_folder,
            &mut self.enhancer.output_folder,
        ] {
            *folder = folder
                .take()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            if let Some(value) = folder
                && (value.len() > 1024 || !std::path::Path::new(value).is_absolute())
            {
                return Err("Folders must be full paths, such as /music or D:\\Music.");
            }
        }
        if self.fetch.move_replaced && self.fetch.replaced_folder.is_none() {
            return Err("Choose the folder replaced files move into.");
        }
        Ok(self)
    }
}

#[derive(QueryableByName)]
struct ValueRow {
    #[diesel(sql_type = Text)]
    value: String,
}

pub fn load(connection: &mut SqliteConnection) -> QueryResult<UpgradeSettings> {
    let rows = diesel::sql_query("SELECT value_json AS value FROM server_setting WHERE key = ?")
        .bind::<Text, _>(SETTINGS_KEY)
        .load::<ValueRow>(connection)?;
    Ok(rows
        .first()
        .and_then(|row| serde_json::from_str(&row.value).ok())
        .unwrap_or_default())
}

pub fn save(connection: &mut SqliteConnection, settings: &UpgradeSettings) -> QueryResult<()> {
    let value = serde_json::to_string(settings).unwrap_or_else(|_| "{}".to_string());
    diesel::sql_query(
        "INSERT INTO server_setting (key, value_json) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(SETTINGS_KEY)
    .bind::<Text, _>(value)
    .execute(connection)?;
    Ok(())
}

/// The secrets Better versions keeps. Each is sealed under its own
/// context, so a sealed value copied into another row does not open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretName {
    FetchToken,
    CfAccessClientId,
    CfAccessClientSecret,
    EnhancerToken,
}

impl SecretName {
    pub const ALL: [Self; 4] = [
        Self::FetchToken,
        Self::CfAccessClientId,
        Self::CfAccessClientSecret,
        Self::EnhancerToken,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FetchToken => "fetch_token",
            Self::CfAccessClientId => "cf_access_client_id",
            Self::CfAccessClientSecret => "cf_access_client_secret",
            Self::EnhancerToken => "enhancer_token",
        }
    }

    pub fn context(self) -> String {
        format!("parson-upgrade:v1:0:{}", self.as_str())
    }
}

#[derive(QueryableByName)]
struct SealedRow {
    #[diesel(sql_type = Binary)]
    sealed: Vec<u8>,
}

#[derive(QueryableByName)]
struct NameRow {
    #[diesel(sql_type = Text)]
    name: String,
}

pub fn sealed_secret(
    connection: &mut SqliteConnection,
    name: SecretName,
) -> QueryResult<Option<Vec<u8>>> {
    Ok(
        diesel::sql_query("SELECT sealed FROM upgrade_secret WHERE name = ?")
            .bind::<Text, _>(name.as_str())
            .load::<SealedRow>(connection)?
            .into_iter()
            .next()
            .map(|row| row.sealed),
    )
}

pub fn set_sealed_secret(
    connection: &mut SqliteConnection,
    name: SecretName,
    sealed: &[u8],
    admin_id: Option<i32>,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO upgrade_secret (name, sealed, updated_by) VALUES (?, ?, ?)
         ON CONFLICT(name) DO UPDATE SET sealed = excluded.sealed,
             updated_by = excluded.updated_by, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(name.as_str())
    .bind::<Binary, _>(sealed)
    .bind::<Nullable<Integer>, _>(admin_id)
    .execute(connection)?;
    Ok(())
}

pub fn delete_secret(connection: &mut SqliteConnection, name: SecretName) -> QueryResult<()> {
    diesel::sql_query("DELETE FROM upgrade_secret WHERE name = ?")
        .bind::<Text, _>(name.as_str())
        .execute(connection)?;
    Ok(())
}

/// Which secrets are saved (names only, never values).
pub fn saved_secrets(connection: &mut SqliteConnection) -> QueryResult<Vec<SecretName>> {
    let names: Vec<String> = diesel::sql_query("SELECT name FROM upgrade_secret")
        .load::<NameRow>(connection)?
        .into_iter()
        .map(|row| row.name)
        .collect();
    Ok(SecretName::ALL
        .into_iter()
        .filter(|name| names.iter().any(|saved| saved == name.as_str()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_point_at_the_home_network_and_are_conservative() {
        let settings = UpgradeSettings::default();
        assert_eq!(settings.fetch.base_url, "http://192.168.1.58:8490");
        assert!(!settings.fetch.enabled);
        assert!(
            !settings.fetch.auto_request,
            "never requests on its own by default"
        );
        assert!(!settings.fetch.include_cd_quality);
        assert!(!settings.fetch.move_replaced);
        assert!(settings.enhancer.prefer_enhanced);
        // Settings saved before a field existed still load.
        let old: UpgradeSettings = serde_json::from_str(r#"{"fetch":{"enabled":true}}"#).unwrap();
        assert!(old.fetch.enabled);
        assert_eq!(old.fetch.auto_per_day, 5);
    }

    #[test]
    fn validation_rejects_bad_urls_and_relative_folders() {
        let mut settings = UpgradeSettings::default();
        settings.fetch.base_url = "http://192.168.1.58:8490/".into();
        settings.fetch.auto_per_day = 500;
        let valid = settings.clone().validated().unwrap();
        assert_eq!(valid.fetch.base_url, "http://192.168.1.58:8490");
        assert_eq!(valid.fetch.auto_per_day, MAX_AUTO_PER_DAY);

        let mut bad = settings.clone();
        bad.fetch.base_url = "ftp://fetch".into();
        assert!(bad.validated().is_err());
        let mut bad = settings.clone();
        bad.fetch.base_url = "http://user:pw@fetch".into();
        assert!(bad.validated().is_err());
        let mut bad = settings.clone();
        bad.enhancer.output_folder = Some("relative/folder".into());
        assert!(bad.validated().is_err());
        let mut bad = settings;
        bad.fetch.move_replaced = true;
        assert!(bad.validated().is_err(), "moving needs a folder");
    }

    #[test]
    fn settings_and_secret_names_round_trip_in_the_database() {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        assert_eq!(load(&mut connection).unwrap(), UpgradeSettings::default());
        let mut settings = UpgradeSettings::default();
        settings.fetch.enabled = true;
        settings.enhancer.prefer_enhanced = false;
        save(&mut connection, &settings).unwrap();
        assert_eq!(load(&mut connection).unwrap(), settings);
        // The catalog layer reads the same preference.
        assert!(!crate::library::tools::marks::prefer_enhanced(
            &mut connection
        ));

        assert!(saved_secrets(&mut connection).unwrap().is_empty());
        set_sealed_secret(&mut connection, SecretName::FetchToken, b"sealed", Some(1)).unwrap();
        assert_eq!(
            saved_secrets(&mut connection).unwrap(),
            vec![SecretName::FetchToken]
        );
        assert_eq!(
            sealed_secret(&mut connection, SecretName::FetchToken)
                .unwrap()
                .as_deref(),
            Some(&b"sealed"[..])
        );
        delete_secret(&mut connection, SecretName::FetchToken).unwrap();
        assert!(
            sealed_secret(&mut connection, SecretName::FetchToken)
                .unwrap()
                .is_none()
        );
    }
}
