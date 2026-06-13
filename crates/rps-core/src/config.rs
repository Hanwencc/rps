use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ControllerConfig {
    pub server: ServerConfig,
    #[serde(default)]
    pub clients: Vec<ClientConfig>,
    #[serde(default)]
    pub tunnels: Vec<TunnelConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub bridge_addr: String,
    #[serde(default = "default_web_addr")]
    pub web_addr: String,
    #[serde(default = "default_web_dir")]
    pub web_dir: String,
    #[serde(default = "default_database_path")]
    pub database_path: String,
    #[serde(default)]
    pub web_auth: WebAuthConfig,
    #[serde(default)]
    pub http_proxy: Option<ProxyListenConfig>,
    #[serde(default)]
    pub socks5: Option<ProxyListenConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebAuthConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_admin_username")]
    pub username: String,
    #[serde(default = "default_admin_password")]
    pub password: String,
    pub totp_secret: Option<String>,
    #[serde(default = "default_session_ttl_secs")]
    pub session_ttl_secs: u64,
}

impl Default for WebAuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            username: default_admin_username(),
            password: default_admin_password(),
            totp_secret: None,
            session_ttl_secs: default_session_ttl_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProxyListenConfig {
    pub listen: String,
    pub client_id: String,
    pub public_udp_addr: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientConfig {
    pub id: String,
    pub psk: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub remark: Option<String>,
    pub max_connections: Option<u32>,
    #[serde(default)]
    pub compress: bool,
    #[serde(default)]
    pub encrypt: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum TunnelMode {
    #[serde(rename = "tcp")]
    Tcp,
    #[serde(rename = "udp")]
    Udp,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TunnelConfig {
    pub id: String,
    pub client_id: String,
    pub mode: TunnelMode,
    pub listen: String,
    pub target: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfigRoot {
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub server_addr: String,
    pub client_id: String,
    pub psk: String,
    #[serde(default = "default_reconnect_interval")]
    pub reconnect_interval_secs: u64,
}

pub fn load_controller_config(path: impl AsRef<Path>) -> anyhow::Result<ControllerConfig> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read controller config {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("failed to parse controller config {}", path.display()))
}

pub fn load_agent_config(path: impl AsRef<Path>) -> anyhow::Result<AgentConfigRoot> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read agent config {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("failed to parse agent config {}", path.display()))
}

fn default_true() -> bool {
    true
}

fn default_admin_username() -> String {
    "admin".to_string()
}

fn default_admin_password() -> String {
    "change-me".to_string()
}

fn default_session_ttl_secs() -> u64 {
    24 * 60 * 60
}

fn default_reconnect_interval() -> u64 {
    5
}

fn default_web_addr() -> String {
    "0.0.0.0:8080".to_string()
}

fn default_web_dir() -> String {
    "web/dist".to_string()
}

fn default_database_path() -> String {
    "data/rps.db".to_string()
}
