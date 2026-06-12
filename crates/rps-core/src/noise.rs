use snow::{Builder, TransportState, params::NoiseParams};
use std::{io, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream},
    sync::Mutex,
};

const NOISE_PARAMS: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const PSK_LEN: usize = 32;
const PSK_HEX_LEN: usize = PSK_LEN * 2;
const MAX_NOISE_MESSAGE_LEN: usize = 65_535;
const MAX_TRANSPORT_PLAINTEXT_LEN: usize = 16 * 1024;
const DUPLEX_BUFFER_LEN: usize = 256 * 1024;

pub async fn connect<T>(mut io: T, psk: &str) -> anyhow::Result<DuplexStream>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let psk = decode_psk(psk)?;
    let params: NoiseParams = NOISE_PARAMS.parse()?;
    let mut noise = Builder::new(params).psk(0, &psk).build_initiator()?;

    let mut buffer = vec![0_u8; MAX_NOISE_MESSAGE_LEN];
    let len = noise.write_message(&[], &mut buffer)?;
    write_noise_message(&mut io, &buffer[..len]).await?;

    let message = read_noise_message(&mut io).await?;
    noise.read_message(&message, &mut buffer)?;

    Ok(spawn_transport(io, noise.into_transport_mode()?))
}

pub async fn accept<T>(mut io: T, psk: &str) -> anyhow::Result<DuplexStream>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let psk = decode_psk(psk)?;
    let params: NoiseParams = NOISE_PARAMS.parse()?;
    let mut noise = Builder::new(params).psk(0, &psk).build_responder()?;

    let mut buffer = vec![0_u8; MAX_NOISE_MESSAGE_LEN];
    let message = read_noise_message(&mut io).await?;
    noise.read_message(&message, &mut buffer)?;

    let len = noise.write_message(&[], &mut buffer)?;
    write_noise_message(&mut io, &buffer[..len]).await?;

    Ok(spawn_transport(io, noise.into_transport_mode()?))
}

pub fn decode_psk(psk: &str) -> anyhow::Result<[u8; PSK_LEN]> {
    let psk = psk.trim();
    if psk.len() != PSK_HEX_LEN {
        anyhow::bail!("psk must be {PSK_HEX_LEN} hex chars");
    }

    let mut out = [0_u8; PSK_LEN];
    for (index, byte) in out.iter_mut().enumerate() {
        let offset = index * 2;
        let hi = decode_hex_nibble(psk.as_bytes()[offset])?;
        let lo = decode_hex_nibble(psk.as_bytes()[offset + 1])?;
        *byte = (hi << 4) | lo;
    }
    Ok(out)
}

fn decode_hex_nibble(value: u8) -> anyhow::Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => anyhow::bail!("psk must contain only hex chars"),
    }
}

async fn read_noise_message<R>(reader: &mut R) -> io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let len = reader.read_u32().await? as usize;
    if len > MAX_NOISE_MESSAGE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "noise message too large",
        ));
    }
    let mut message = vec![0_u8; len];
    reader.read_exact(&mut message).await?;
    Ok(message)
}

async fn write_noise_message<W>(writer: &mut W, message: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    if message.len() > MAX_NOISE_MESSAGE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "noise message too large",
        ));
    }
    writer.write_u32(message.len() as u32).await?;
    writer.write_all(message).await
}

fn spawn_transport<T>(io: T, noise: TransportState) -> DuplexStream
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (app_io, task_io) = tokio::io::duplex(DUPLEX_BUFFER_LEN);
    let (mut plain_reader, mut plain_writer) = tokio::io::split(task_io);
    let (mut encrypted_reader, mut encrypted_writer) = tokio::io::split(io);
    let noise = Arc::new(Mutex::new(noise));

    let outbound_noise = noise.clone();
    tokio::spawn(async move {
        let mut plain = vec![0_u8; MAX_TRANSPORT_PLAINTEXT_LEN];
        let mut encrypted = vec![0_u8; MAX_NOISE_MESSAGE_LEN];
        loop {
            let read = match plain_reader.read(&mut plain).await {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => break,
            };

            let encrypted_len = {
                let mut noise = outbound_noise.lock().await;
                match noise.write_message(&plain[..read], &mut encrypted) {
                    Ok(len) => len,
                    Err(_) => break,
                }
            };

            if write_noise_message(&mut encrypted_writer, &encrypted[..encrypted_len])
                .await
                .is_err()
            {
                break;
            }
        }
        let _ = encrypted_writer.shutdown().await;
    });

    let inbound_noise = noise.clone();
    tokio::spawn(async move {
        let mut plain = vec![0_u8; MAX_NOISE_MESSAGE_LEN];
        loop {
            let encrypted = match read_noise_message(&mut encrypted_reader).await {
                Ok(encrypted) => encrypted,
                Err(_) => break,
            };

            let plain_len = {
                let mut noise = inbound_noise.lock().await;
                match noise.read_message(&encrypted, &mut plain) {
                    Ok(len) => len,
                    Err(_) => break,
                }
            };

            if plain_writer.write_all(&plain[..plain_len]).await.is_err() {
                break;
            }
        }
        let _ = plain_writer.shutdown().await;
    });

    app_io
}

#[cfg(test)]
mod tests {
    use super::decode_psk;

    #[test]
    fn decodes_64_hex_psk() {
        let psk = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let decoded = decode_psk(psk).expect("valid psk");
        assert_eq!(decoded.len(), 32);
        assert_eq!(decoded[0], 0x01);
        assert_eq!(decoded[31], 0xef);
    }

    #[test]
    fn rejects_wrong_length_psk() {
        assert!(decode_psk("abcd").is_err());
    }

    #[test]
    fn rejects_non_hex_psk() {
        let psk = "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(decode_psk(psk).is_err());
    }
}
