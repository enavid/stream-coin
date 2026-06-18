use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_URL: &str = "http://localhost:8080";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ServerConfig {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AuthConfig {
    pub token: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                url: DEFAULT_URL.to_string(),
            },
            auth: AuthConfig { token: None },
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        if let Ok(path) = std::env::var("SC_CONFIG_PATH") {
            return PathBuf::from(path);
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sc")
            .join("config.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if !path.exists() {
            return Self::default();
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&content).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, content).map_err(|e| e.to_string())
    }

    pub fn set_url(&mut self, url: &str) {
        self.server.url = url.to_string();
    }

    pub fn set_token(&mut self, token: Option<String>) {
        self.auth.token = token;
    }

    pub fn is_authenticated(&self) -> bool {
        self.auth.token.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // SC_CONFIG_PATH is process-global; cargo runs tests in parallel threads
    // within the same process, so tests that set/read it must not interleave.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn config_default_server_url_is_localhost_8080() {
        let config = Config::default();
        assert_eq!(config.server.url, "http://localhost:8080");
    }

    #[test]
    fn config_is_not_authenticated_by_default() {
        let config = Config::default();
        assert!(!config.is_authenticated());
    }

    #[test]
    fn config_is_authenticated_after_token_set() {
        let mut config = Config::default();
        config.set_token(Some("my-token".to_string()));
        assert!(config.is_authenticated());
    }

    #[test]
    fn config_set_token_none_clears_authentication() {
        let mut config = Config::default();
        config.set_token(Some("my-token".to_string()));
        config.set_token(None);
        assert!(!config.is_authenticated());
    }

    #[test]
    fn config_set_url_updates_server_url() {
        let mut config = Config::default();
        config.set_url("http://example.com:9090");
        assert_eq!(config.server.url, "http://example.com:9090");
    }

    #[test]
    fn config_serializes_to_toml_with_server_section() {
        let config = Config::default();
        let toml = toml::to_string(&config).unwrap();
        assert!(toml.contains("[server]"));
        assert!(toml.contains("url = "));
    }

    #[test]
    fn config_serializes_to_toml_with_auth_section() {
        let config = Config::default();
        let toml = toml::to_string(&config).unwrap();
        assert!(toml.contains("[auth]"));
    }

    #[test]
    fn config_deserializes_from_valid_toml() {
        let toml_str = r#"
[server]
url = "http://test.local:8080"

[auth]
token = "abc123"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.url, "http://test.local:8080");
        assert_eq!(config.auth.token.as_deref(), Some("abc123"));
    }

    #[test]
    fn config_path_uses_sc_config_path_env_var() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::set_var("SC_CONFIG_PATH", "/tmp/test-sc-config.toml");
        let path = Config::config_path();
        assert_eq!(path, PathBuf::from("/tmp/test-sc-config.toml"));
        std::env::remove_var("SC_CONFIG_PATH");
    }

    #[test]
    fn config_path_contains_sc_directory_by_default() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::remove_var("SC_CONFIG_PATH");
        let path = Config::config_path();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("sc"));
        assert!(path_str.ends_with("config.toml"));
    }

    #[test]
    fn config_save_and_load_roundtrip() {
        let _guard = ENV_GUARD.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::env::set_var("SC_CONFIG_PATH", config_path.to_str().unwrap());

        let mut config = Config::default();
        config.set_url("http://saved.example.com");
        config.set_token(Some("saved-token".to_string()));
        config.save().unwrap();

        let loaded = Config::load();
        assert_eq!(loaded.server.url, "http://saved.example.com");
        assert_eq!(loaded.auth.token.as_deref(), Some("saved-token"));

        std::env::remove_var("SC_CONFIG_PATH");
    }
}
