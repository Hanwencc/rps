use bytes::Bytes;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const HEADER_LEN: usize = 9;
const MAX_PAYLOAD_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    Open = 0x01,
    OpenAck = 0x02,
    Data = 0x03,
    Close = 0x04,
    Ping = 0x05,
    Pong = 0x06,
    Error = 0x07,
}

impl TryFrom<u8> for FrameType {
    type Error = io::Error;

    fn try_from(value: u8) -> Result<Self, io::Error> {
        match value {
            0x01 => Ok(Self::Open),
            0x02 => Ok(Self::OpenAck),
            0x03 => Ok(Self::Data),
            0x04 => Ok(Self::Close),
            0x05 => Ok(Self::Ping),
            0x06 => Ok(Self::Pong),
            0x07 => Ok(Self::Error),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown frame type {value}"),
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub frame_type: FrameType,
    pub stream_id: u32,
    pub payload: Bytes,
}

impl Frame {
    pub fn new(frame_type: FrameType, stream_id: u32, payload: impl Into<Bytes>) -> Self {
        Self {
            frame_type,
            stream_id,
            payload: payload.into(),
        }
    }
}

pub async fn read_frame<R>(reader: &mut R) -> io::Result<Frame>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; HEADER_LEN];
    reader.read_exact(&mut header).await?;
    let frame_type = FrameType::try_from(header[0])?;
    let stream_id = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
    let len = u32::from_be_bytes([header[5], header[6], header[7], header[8]]) as usize;
    if len > MAX_PAYLOAD_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mux payload too large",
        ));
    }
    let mut payload = vec![0_u8; len];
    if len > 0 {
        reader.read_exact(&mut payload).await?;
    }
    Ok(Frame::new(frame_type, stream_id, Bytes::from(payload)))
}

pub async fn write_frame<W>(writer: &mut W, frame: &Frame) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    if frame.payload.len() > MAX_PAYLOAD_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mux payload too large",
        ));
    }
    let mut header = [0_u8; HEADER_LEN];
    header[0] = frame.frame_type as u8;
    header[1..5].copy_from_slice(&frame.stream_id.to_be_bytes());
    header[5..9].copy_from_slice(&(frame.payload.len() as u32).to_be_bytes());
    writer.write_all(&header).await?;
    if !frame.payload.is_empty() {
        writer.write_all(&frame.payload).await?;
    }
    Ok(())
}
