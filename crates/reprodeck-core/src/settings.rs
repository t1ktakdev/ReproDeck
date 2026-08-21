use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};
use thiserror::Error;

const SETTINGS_KEY: &str = "app";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Clock(#[from] SystemTimeError),
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),
    #[error("unsupported theme: {0}")]
    UnsupportedTheme(String),
    #[error("invalid setting: {0}")]
    InvalidSetting(&'static str),
}

pub type Result<T> = std::result::Result<T, SettingsError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub timeout_secs: u64,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai-compatible".to_string(),
            base_url: "http://localhost:1234/v1".to_string(),
            model: String::new(),
            timeout_secs: 60,
            max_tokens: 2048,
            temperature: 0.2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiSettings {
    pub density: String,
    pub font_size: String,
    pub mono_font_size: u8,
    pub animations: bool,
    pub reduced_motion: bool,
    pub sidebar_mode: String,
    pub remember_sidebar_width: bool,
    pub sidebar_width: u16,
    pub remember_inspector_width: bool,
    pub inspector_width: u16,
    pub remember_inspector_state: bool,
    pub inspector_open: bool,
    pub zoom: u16,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            density: "comfortable".to_string(),
            font_size: "default".to_string(),
            mono_font_size: 13,
            animations: true,
            reduced_motion: false,
            sidebar_mode: "expanded".to_string(),
            remember_sidebar_width: true,
            sidebar_width: 256,
            remember_inspector_width: true,
            inspector_width: 480,
            remember_inspector_state: true,
            inspector_open: true,
            zoom: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BehaviorSettings {
    pub restore_last_project: bool,
    pub restore_last_workspace: bool,
    pub auto_open_investigation: bool,
    pub auto_scroll_logs: bool,
    pub open_logs_on_failure: bool,
    pub notifications: bool,
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        Self {
            restore_last_project: true,
            restore_last_workspace: true,
            auto_open_investigation: true,
            auto_scroll_logs: true,
            open_logs_on_failure: true,
            notifications: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkspaceSettings {
    pub kind: String,
    pub root_view: String,
    pub project_path: Option<String>,
    pub project_tab: String,
    pub investigation_case_id: Option<String>,
    pub session_id: Option<String>,
    pub session_tab: String,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            kind: "root".to_string(),
            root_view: "home".to_string(),
            project_path: None,
            project_tab: "project-overview".to_string(),
            investigation_case_id: None,
            session_id: None,
            session_tab: "overview".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppSettings {
    pub language: String,
    pub theme: String,
    pub ui: UiSettings,
    pub behavior: BehaviorSettings,
    pub workspace: WorkspaceSettings,
    pub ai: AiSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "en".to_string(),
            theme: "system".to_string(),
            ui: UiSettings::default(),
            behavior: BehaviorSettings::default(),
            workspace: WorkspaceSettings::default(),
            ai: AiSettings::default(),
        }
    }
}

fn unix_time_secs() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn validate(settings: &AppSettings) -> Result<()> {
    if !matches!(settings.language.as_str(), "en" | "ru") {
        return Err(SettingsError::UnsupportedLanguage(
            settings.language.clone(),
        ));
    }
    if !matches!(settings.theme.as_str(), "system" | "dark" | "light") {
        return Err(SettingsError::UnsupportedTheme(settings.theme.clone()));
    }
    if !matches!(settings.ui.density.as_str(), "comfortable" | "compact") {
        return Err(SettingsError::InvalidSetting("ui.density"));
    }
    if !matches!(
        settings.ui.font_size.as_str(),
        "small" | "default" | "large"
    ) {
        return Err(SettingsError::InvalidSetting("ui.font_size"));
    }
    if !matches!(settings.ui.mono_font_size, 12..=15) {
        return Err(SettingsError::InvalidSetting("ui.mono_font_size"));
    }
    if !matches!(settings.ui.sidebar_mode.as_str(), "expanded" | "compact") {
        return Err(SettingsError::InvalidSetting("ui.sidebar_mode"));
    }
    if !(220..=340).contains(&settings.ui.sidebar_width) {
        return Err(SettingsError::InvalidSetting("ui.sidebar_width"));
    }
    if !(360..=760).contains(&settings.ui.inspector_width) {
        return Err(SettingsError::InvalidSetting("ui.inspector_width"));
    }
    if !matches!(settings.ui.zoom, 90 | 100 | 110 | 125) {
        return Err(SettingsError::InvalidSetting("ui.zoom"));
    }
    if settings.ai.provider != "openai-compatible" {
        return Err(SettingsError::InvalidSetting("ai.provider"));
    }
    if !settings.ai.temperature.is_finite() || !(0.0..=2.0).contains(&settings.ai.temperature) {
        return Err(SettingsError::InvalidSetting("ai.temperature"));
    }
    Ok(())
}

pub fn load(conn: &Connection) -> Result<AppSettings> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value_json FROM settings WHERE key=?1",
            rusqlite::params![SETTINGS_KEY],
            |row| row.get(0),
        )
        .optional()?;
    match raw {
        Some(value) => {
            let settings: AppSettings = serde_json::from_str(&value)?;
            validate(&settings)?;
            Ok(settings)
        }
        None => Ok(AppSettings::default()),
    }
}

pub fn save(conn: &Connection, settings: &AppSettings) -> Result<AppSettings> {
    validate(settings)?;
    let now = unix_time_secs()?;
    conn.execute(
        "INSERT INTO settings(key,value_json,updated_at) VALUES (?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,updated_at=excluded.updated_at",
        rusqlite::params![SETTINGS_KEY, serde_json::to_string(settings)?, now],
    )?;
    load(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::NamedTempFile;

    #[test]
    fn settings_round_trip() {
        let file = NamedTempFile::new().unwrap();
        let conn = init_db(file.path()).unwrap();
        let settings = AppSettings {
            language: "ru".into(),
            theme: "dark".into(),
            ..AppSettings::default()
        };
        save(&conn, &settings).unwrap();
        assert_eq!(load(&conn).unwrap(), settings);
    }

    #[test]
    fn invalid_language_is_rejected() {
        let file = NamedTempFile::new().unwrap();
        let conn = init_db(file.path()).unwrap();
        let settings = AppSettings {
            language: "xx".into(),
            ..AppSettings::default()
        };
        assert!(matches!(
            save(&conn, &settings),
            Err(SettingsError::UnsupportedLanguage(_))
        ));
    }

    #[test]
    fn older_nested_settings_receive_new_defaults() {
        let value = r#"{
          "language":"ru",
          "theme":"dark",
          "ui":{"density":"compact","font_size":"default","mono_font_size":13,"animations":true,"reduced_motion":false,"sidebar_mode":"expanded","remember_sidebar_width":true,"sidebar_width":280,"inspector_width":520,"remember_inspector_state":true,"inspector_open":false,"zoom":100},
          "behavior":{},
          "workspace":{"kind":"project","root_view":"projects","project_path":"C:/repo","project_tab":"checks","session_id":null,"session_tab":"overview"},
          "ai":{}
        }"#;
        let settings: AppSettings = serde_json::from_str(value).unwrap();
        assert!(settings.ui.remember_inspector_width);
        assert_eq!(settings.ui.inspector_width, 520);
        assert_eq!(settings.workspace.investigation_case_id, None);
        assert_eq!(settings.workspace.project_path.as_deref(), Some("C:/repo"));
        validate(&settings).unwrap();
    }
}
