use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse config file: {0}")]
    ParseError(#[from] json5::Error),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RppConfig {
    #[serde(default = "default_source_dir")]
    pub source_dir: String,

    #[serde(default)]
    pub dev_server: DevServerConfig,

    #[serde(default)]
    pub plugin_repositories: Vec<String>,
}

impl RppConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path)?;
        let config = json5::from_str(&contents)?;
        Ok(config)
    }

    pub fn load_from_current_dir() -> Result<Option<Self>, ConfigError> {
        let config_path = Path::new("rpp.jsonc");
        if config_path.exists() {
            Ok(Some(Self::load(config_path)?))
        } else {
            Ok(None)
        }
    }
}

impl Default for RppConfig {
    fn default() -> Self {
        Self {
            source_dir: default_source_dir(),
            dev_server: DevServerConfig::default(),
            plugin_repositories: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevServerConfig {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_hot_reload")]
    pub hot_reload: bool,
}

impl Default for DevServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            hot_reload: default_hot_reload(),
        }
    }
}

fn default_source_dir() -> String {
    "src".to_string()
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_hot_reload() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RppConfig::default();
        assert_eq!(config.source_dir, "src");
        assert_eq!(config.dev_server.host, "127.0.0.1");
        assert_eq!(config.dev_server.port, 8080);
        assert!(config.dev_server.hot_reload);
        assert!(config.plugin_repositories.is_empty());
    }

    #[test]
    fn test_parse_minimal_config() {
        let json = "{}";
        let config: RppConfig = json5::from_str(json).unwrap();
        assert_eq!(config.source_dir, "src");
    }

    #[test]
    fn test_parse_full_config() {
        let json = r#"{
            "sourceDir": "custom_src",
            "devServer": {
                "host": "0.0.0.0",
                "port": 3000,
                "hotReload": false
            },
            "pluginRepositories": [
                "https://example.com/plugins"
            ]
        }"#;
        let config: RppConfig = json5::from_str(json).unwrap();
        assert_eq!(config.source_dir, "custom_src");
        assert_eq!(config.dev_server.host, "0.0.0.0");
        assert_eq!(config.dev_server.port, 3000);
        assert!(!config.dev_server.hot_reload);
        assert_eq!(config.plugin_repositories.len(), 1);
    }

    #[test]
    fn test_parse_jsonc_with_comments() {
        let jsonc = r#"{
            // Source directory containing pack contents
            "sourceDir": "src",

            // Dev server configuration
            "devServer": {
                "host": "127.0.0.1",
                "port": 8080
            }
        }"#;
        let config: RppConfig = json5::from_str(jsonc).unwrap();
        assert_eq!(config.source_dir, "src");
        assert_eq!(config.dev_server.port, 8080);
    }
}
