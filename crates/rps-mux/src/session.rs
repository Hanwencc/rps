use crate::frame::{Frame, FrameType, read_frame, write_frame};
use bytes::Bytes;
use dashmap::DashMap;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};
use tracing::{debug, warn};

type StreamTx = mpsc::Sender<Bytes>;
type PendingTx = oneshot::Sender<()>;

#[derive(Clone)]
pub struct MuxHandle {
    writer_tx: mpsc::Sender<Frame>,
    streams: Arc<DashMap<u32, StreamTx>>,
    pending: Arc<DashMap<u32, PendingTx>>,
    next_id: Arc<AtomicU32>,
}

pub struct Mux {
    handle: MuxHandle,
    incoming_rx: mpsc::Receiver<MuxStream>,
}

pub struct MuxStream {
    id: u32,
    writer_tx: mpsc::Sender<Frame>,
    inbound_rx: mpsc::Receiver<Bytes>,
}

#[derive(Clone)]
pub struct MuxStreamWriter {
    id: u32,
    writer_tx: mpsc::Sender<Frame>,
}

pub struct MuxStreamReader {
    inbound_rx: mpsc::Receiver<Bytes>,
}

impl Mux {
    pub fn new<T>(io: T) -> Self
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (mut reader, mut writer) = tokio::io::split(io);
        let (writer_tx, mut writer_rx) = mpsc::channel::<Frame>(1024);
        let (incoming_tx, incoming_rx) = mpsc::channel::<MuxStream>(1024);
        let streams = Arc::new(DashMap::<u32, StreamTx>::new());
        let pending = Arc::new(DashMap::<u32, PendingTx>::new());

        let handle = MuxHandle {
            writer_tx: writer_tx.clone(),
            streams: streams.clone(),
            pending: pending.clone(),
            next_id: Arc::new(AtomicU32::new(1)),
        };

        tokio::spawn(async move {
            while let Some(frame) = writer_rx.recv().await {
                if let Err(err) = write_frame(&mut writer, &frame).await {
                    warn!(error = %err, "mux writer stopped");
                    break;
                }
            }
        });

        let read_writer_tx = writer_tx.clone();
        tokio::spawn(async move {
            loop {
                let frame = match read_frame(&mut reader).await {
                    Ok(frame) => frame,
                    Err(err) => {
                        debug!(error = %err, "mux reader stopped");
                        break;
                    }
                };

                match frame.frame_type {
                    FrameType::Open => {
                        let (inbound_tx, inbound_rx) = mpsc::channel::<Bytes>(1024);
                        if !frame.payload.is_empty() {
                            let _ = inbound_tx.send(frame.payload).await;
                        }
                        streams.insert(frame.stream_id, inbound_tx);
                        let stream = MuxStream {
                            id: frame.stream_id,
                            writer_tx: read_writer_tx.clone(),
                            inbound_rx,
                        };
                        if incoming_tx.send(stream).await.is_err() {
                            break;
                        }
                        let _ = read_writer_tx
                            .send(Frame::new(
                                FrameType::OpenAck,
                                frame.stream_id,
                                Bytes::new(),
                            ))
                            .await;
                    }
                    FrameType::OpenAck => {
                        if let Some((_, tx)) = pending.remove(&frame.stream_id) {
                            let _ = tx.send(());
                        }
                    }
                    FrameType::Data => {
                        if let Some(tx) = streams.get(&frame.stream_id) {
                            let _ = tx.send(frame.payload).await;
                        }
                    }
                    FrameType::Close | FrameType::Error => {
                        streams.remove(&frame.stream_id);
                        if let Some((_, tx)) = pending.remove(&frame.stream_id) {
                            let _ = tx.send(());
                        }
                    }
                    FrameType::Ping => {
                        let _ = read_writer_tx
                            .send(Frame::new(FrameType::Pong, frame.stream_id, frame.payload))
                            .await;
                    }
                    FrameType::Pong => {}
                }
            }
        });

        Self {
            handle,
            incoming_rx,
        }
    }

    pub fn handle(&self) -> MuxHandle {
        self.handle.clone()
    }

    pub async fn accept(&mut self) -> Option<MuxStream> {
        self.incoming_rx.recv().await
    }
}

impl MuxHandle {
    pub async fn open_stream(&self, payload: Bytes) -> io::Result<MuxStream> {
        let id = self.next_stream_id();
        let (inbound_tx, inbound_rx) = mpsc::channel::<Bytes>(1024);
        let (ack_tx, ack_rx) = oneshot::channel();
        self.streams.insert(id, inbound_tx);
        self.pending.insert(id, ack_tx);
        self.writer_tx
            .send(Frame::new(FrameType::Open, id, payload))
            .await
            .map_err(closed)?;
        timeout(Duration::from_secs(10), ack_rx)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "mux open stream timed out"))?
            .map_err(closed)?;
        Ok(MuxStream {
            id,
            writer_tx: self.writer_tx.clone(),
            inbound_rx,
        })
    }

    fn next_stream_id(&self) -> u32 {
        self.next_id.fetch_add(2, Ordering::Relaxed)
    }
}

impl MuxStream {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn split(self) -> (MuxStreamWriter, MuxStreamReader) {
        (
            MuxStreamWriter {
                id: self.id,
                writer_tx: self.writer_tx,
            },
            MuxStreamReader {
                inbound_rx: self.inbound_rx,
            },
        )
    }
}

impl MuxStreamWriter {
    pub async fn send_data(&self, data: impl Into<Bytes>) -> io::Result<()> {
        self.writer_tx
            .send(Frame::new(FrameType::Data, self.id, data.into()))
            .await
            .map_err(closed)
    }

    pub async fn close(&self) -> io::Result<()> {
        self.writer_tx
            .send(Frame::new(FrameType::Close, self.id, Bytes::new()))
            .await
            .map_err(closed)
    }
}

impl MuxStreamReader {
    pub async fn recv_data(&mut self) -> Option<Bytes> {
        self.inbound_rx.recv().await
    }
}

fn closed<T>(_: T) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "mux closed")
}
