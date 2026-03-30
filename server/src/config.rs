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
        match config_path {
            Some(path) if path.exists() => {
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                toml::from_str(&content).unwrap_or_default()
            }
            _ => Self::default(),
        }
    }

    pub fn bind_addr(&self) -> SocketAddr {
        let ip: std::net::IpAddr = self.server.host.parse().unwrap_or([0, 0, 0, 0].into());
        SocketAddr::new(ip, self.server.port)
    }
}
