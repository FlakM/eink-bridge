use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub state_dir: PathBuf,
    pub session_timeout_minutes: u64,
    pub long_poll_seconds: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        let state_dir = directories::ProjectDirs::from("com", "flakm", "eink-bridge")
            .map(|d| d.data_local_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/tmp/eink-bridge"));
        Self {
            host: "0.0.0.0".into(),
            port: 3333,
            state_dir,
            session_timeout_minutes: 30,
            long_poll_seconds: 30,
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        let config_path = directories::ProjectDirs::from("com", "flakm", "eink-bridge")
            .map(|d| d.config_dir().join("config.toml"));
        Self::load_from(config_path.as_deref())
    }

    fn load_from(path: Option<&std::path::Path>) -> Self {
        match path {
            Some(p) if p.exists() => {
                tracing::info!(path = %p.display(), "loading config");
                let content = std::fs::read_to_string(p).unwrap_or_default();
                match toml::from_str(&content) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        tracing::warn!(path = %p.display(), error = %e, "invalid config, using defaults");
                        Self::default()
                    }
                }
            }
            _ => {
                tracing::debug!("no config file found, using defaults");
                Self::default()
            }
        }
    }

    pub fn bind_addr(&self) -> SocketAddr {
        let ip: std::net::IpAddr = self.server.host.parse().unwrap_or([0, 0, 0, 0].into());
        SocketAddr::new(ip, self.server.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sane_values() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.server.port, 3333);
        assert_eq!(cfg.server.host, "0.0.0.0");
        assert_eq!(cfg.server.long_poll_seconds, 30);
        assert_eq!(cfg.server.session_timeout_minutes, 30);
    }

    #[test]
    fn bind_addr_uses_configured_host_and_port() {
        let mut cfg = AppConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.server.port = 8080;
        let addr = cfg.bind_addr();
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn bind_addr_falls_back_on_invalid_host() {
        let mut cfg = AppConfig::default();
        cfg.server.host = "not-an-ip".into();
        let addr = cfg.bind_addr();
        assert_eq!(addr.ip().to_string(), "0.0.0.0");
    }

    #[test]
    fn valid_toml_parses() {
        let toml = r#"
[server]
host = "127.0.0.1"
port = 9999
session_timeout_minutes = 60
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.server.port, 9999);
        assert_eq!(cfg.server.session_timeout_minutes, 60);
        assert_eq!(cfg.server.long_poll_seconds, 30);
    }

    #[test]
    fn empty_toml_gives_defaults() {
        let cfg: AppConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.server.port, 3333);
    }

    #[test]
    fn invalid_toml_errors() {
        let result: Result<AppConfig, _> = toml::from_str("not valid { toml");
        assert!(result.is_err());
    }

    #[test]
    fn load_from_none_returns_defaults() {
        let cfg = AppConfig::load_from(None);
        assert_eq!(cfg.server.port, 3333);
    }

    #[test]
    fn load_from_nonexistent_path_returns_defaults() {
        let cfg = AppConfig::load_from(Some(std::path::Path::new("/nonexistent/config.toml")));
        assert_eq!(cfg.server.port, 3333);
    }

    #[test]
    fn load_from_valid_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[server]\nport = 9876\n").unwrap();
        let cfg = AppConfig::load_from(Some(&path));
        assert_eq!(cfg.server.port, 9876);
    }

    #[test]
    fn load_from_invalid_toml_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid { toml !!!").unwrap();
        let cfg = AppConfig::load_from(Some(&path));
        assert_eq!(cfg.server.port, 3333);
    }
}
