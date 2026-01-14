//! Configuration management for Rung.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Rung configuration loaded from .git/rung/config.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// General settings.
    #[serde(default)]
    pub general: GeneralConfig,

    /// GitHub-specific settings.
    #[serde(default)]
    pub github: GitHubConfig,
}

impl Config {
    /// Load config from a TOML file.
    ///
    /// # Errors
    /// Returns error if file can't be read or parsed.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save config to a TOML file.
    ///
    /// # Errors
    /// Returns error if serialization or write fails.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let content =
            toml::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        fs::write(path, content)?;
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            github: GitHubConfig::default(),
        }
    }
}

/// General Rung settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Default remote to push to.
    #[serde(default = "default_remote")]
    pub default_remote: String,

    /// Number of backups to retain.
    #[serde(default = "default_backup_retention")]
    pub backup_retention: usize,

    /// Whether to automatically sync on checkout.
    #[serde(default)]
    pub auto_sync: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_remote: default_remote(),
            backup_retention: default_backup_retention(),
            auto_sync: false,
        }
    }
}

fn default_remote() -> String {
    "origin".into()
}

const fn default_backup_retention() -> usize {
    5
}

/// GitHub-specific settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitHubConfig {
    /// Custom API URL for GitHub Enterprise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.general.default_remote, "origin");
        assert_eq!(config.general.backup_retention, 5);
        assert!(!config.general.auto_sync);
    }

    #[test]
    fn test_config_roundtrip() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");

        let config = Config {
            general: GeneralConfig {
                default_remote: "upstream".into(),
                backup_retention: 10,
                auto_sync: true,
            },
            github: GitHubConfig {
                api_url: Some("https://github.example.com/api/v3".into()),
            },
        };

        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();

        assert_eq!(loaded.general.default_remote, "upstream");
        assert_eq!(loaded.general.backup_retention, 10);
        assert!(loaded.general.auto_sync);
        assert_eq!(
            loaded.github.api_url,
            Some("https://github.example.com/api/v3".into())
        );
    }

    #[test]
    fn test_missing_config_returns_default() {
        let config = Config::load("/nonexistent/path/config.toml").unwrap();
        assert_eq!(config.general.default_remote, "origin");
    }
}
