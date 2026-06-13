use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAGIC: &str = "RPS1";
pub const VERSION: &str = "0.1.0";
const MAX_JSON_MESSAGE_LEN: usize = 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HelloRole {
    Control,
    Data,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NoisePrelude {
    pub magic: String,
    pub client_id: String,
    pub version: String,
}

impl NoisePrelude {
    pub fn new(client_id: String) -> Self {
        Self {
            magic: MAGIC.to_string(),
            client_id,
            version: VERSION.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Hello {
    pub magic: String,
    pub role: HelloRole,
    pub client_id: String,
    pub version: String,
}

impl Hello {
    pub fn new(role: HelloRole, client_id: String) -> Self {
        Self {
            magic: MAGIC.to_string(),
            role,
            client_id,
            version: VERSION.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HelloAck {
    pub ok: bool,
    pub error: Option<String>,
    pub server_version: String,
}

impl HelloAck {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            server_version: VERSION.to_string(),
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
            server_version: VERSION.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenRequest {
    pub tunnel_id: String,
    pub protocol: TargetProtocol,
    pub target: String,
    pub remote_addr: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenResponse {
    pub ok: bool,
    pub error: Option<String>,
}

impl OpenResponse {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ControlMessage {
    Ping { ts: u64 },
    Pong { ts: u64 },
    Shutdown { reason: String },
}

pub async fn write_json<W, T>(writer: &mut W, value: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(value).map_err(invalid_data)?;
    if bytes.len() > MAX_JSON_MESSAGE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "json message too large",
        ));
    }
    writer.write_u32(bytes.len() as u32).await?;
    writer.write_all(&bytes).await
}

pub async fn read_json<R, T>(reader: &mut R) -> io::Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let len = reader.read_u32().await? as usize;
    if len > MAX_JSON_MESSAGE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "json message too large",
        ));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(invalid_data)
}

fn invalid_data(err: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}
