//! RFC 8656 section 7: STUN and padded ChannelData messages over TCP/TLS.
//! The relay allocation remains UDP; only the client-to-TURN leg is a stream.
use async_trait::async_trait;
use std::{any::Any, io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf},
    net::TcpStream,
    sync::{watch, Mutex},
};
use util::conn::Conn;

trait Io: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> Io for T {}
type Stream = Box<dyn Io>;

// Cover the largest legal STUN body as well as ChannelData and its padding.
// Reads return available bytes; this capacity never imposes a fill delay.
const RECEIVE_CAPACITY: usize = 64 * 1024 + 20;
const CHANNEL_CAPACITY: usize = (4 + u16::MAX as usize + 3) & !3;

struct ReceiveBuffer {
    bytes: Box<[u8]>,
    start: usize,
    end: usize,
}

impl ReceiveBuffer {
    async fn receive(
        &mut self,
        reader: &mut ReadHalf<Stream>,
        destination: &mut [u8],
    ) -> io::Result<usize> {
        loop {
            if self.end - self.start >= 4 {
                let (size, padding) = frame_shape(&self.bytes[self.start..])?;
                if size > destination.len() || size + padding > self.bytes.len() {
                    return Err(invalid());
                }
                if self.end - self.start >= size + padding {
                    let message = &self.bytes[self.start..self.start + size];
                    if message[0] >> 6 == 0 && message[4..8] != [0x21, 0x12, 0xa4, 0x42] {
                        return Err(invalid());
                    }
                    // Conn requires caller-owned output; unlike the BUD driver,
                    // it cannot borrow this frame directly from our buffer.
                    destination[..size].copy_from_slice(message);
                    self.start += size + padding;
                    return Ok(size);
                }
            }
            if self.start == self.end || self.end == self.bytes.len() {
                self.bytes.copy_within(self.start..self.end, 0);
                self.end -= self.start;
                self.start = 0;
            }
            let count = reader.read(&mut self.bytes[self.end..]).await?;
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.end += count;
        }
    }
}

struct Reader {
    stream: ReadHalf<Stream>,
    buffer: Option<ReceiveBuffer>,
}

impl Reader {
    async fn receive(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        if let Some(buffer) = &mut self.buffer {
            return buffer.receive(&mut self.stream, destination).await;
        }
        // rustls already buffers incoming records: retain the direct TLS path.
        let mut header = [0; 4];
        self.stream.read_exact(&mut header).await?;
        let (size, padding) = frame_shape(&header)?;
        if size > destination.len() {
            return Err(invalid());
        }
        destination[..4].copy_from_slice(&header);
        self.stream.read_exact(&mut destination[4..size]).await?;
        if header[0] >> 6 == 0 && destination[4..8] != [0x21, 0x12, 0xa4, 0x42] {
            return Err(invalid());
        }
        let mut pad = [0; 3];
        self.stream.read_exact(&mut pad[..padding]).await?;
        Ok(size)
    }
}

struct Writer {
    stream: WriteHalf<Stream>,
    padded: Box<[u8]>,
}

impl Writer {
    async fn send(&mut self, message: &[u8], size: usize, padding: usize) -> io::Result<()> {
        if padding == 0 {
            // Aligned ChannelData and STUN need no staging copy.
            self.stream.write_all(&message[..size]).await?;
        } else {
            let wire = self.padded.get_mut(..size + padding).ok_or_else(invalid)?;
            wire[..size].copy_from_slice(&message[..size]);
            wire[size..].fill(0);
            // One framed write prevents a separate tiny TCP write/TLS record
            // for padding. Partial writes stay serialized under the same lock.
            self.stream.write_all(wire).await?;
        }
        self.stream.flush().await
    }
}

/// The upstream TURN client does not own/close its supplied transport.
/// Bind stream lifetime to the allocation so ICE shutdown closes both.
pub(crate) struct RelayStream {
    pub relay: Arc<dyn Conn + Send + Sync>,
    pub transport: Arc<dyn Conn + Send + Sync>,
}
#[async_trait]
impl Conn for RelayStream {
    async fn connect(&self, addr: SocketAddr) -> util::Result<()> {
        self.relay.connect(addr).await
    }
    async fn recv(&self, buf: &mut [u8]) -> util::Result<usize> {
        self.relay.recv(buf).await
    }
    async fn recv_from(&self, buf: &mut [u8]) -> util::Result<(usize, SocketAddr)> {
        self.relay.recv_from(buf).await
    }
    async fn send(&self, buf: &[u8]) -> util::Result<usize> {
        self.relay.send(buf).await
    }
    async fn send_to(&self, buf: &[u8], addr: SocketAddr) -> util::Result<usize> {
        self.relay.send_to(buf, addr).await
    }
    fn local_addr(&self) -> util::Result<SocketAddr> {
        self.relay.local_addr()
    }
    fn remote_addr(&self) -> Option<SocketAddr> {
        self.relay.remote_addr()
    }
    async fn close(&self) -> util::Result<()> {
        let relay = self.relay.close().await;
        let transport = self.transport.close().await;
        relay.and(transport)
    }
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}

pub struct TurnStream {
    _qos: evoparseq_socket_qos::Flow,
    reader: Mutex<Reader>,
    writer: Mutex<Writer>,
    local: SocketAddr,
    remote: SocketAddr,
    closed: watch::Sender<bool>,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid TURN stream frame")
}

/// Return the logical frame size and transport padding after a four-byte header.
fn frame_shape(header: &[u8]) -> io::Result<(usize, usize)> {
    let length = u16::from_be_bytes([header[2], header[3]]) as usize;
    match header[0] >> 6 {
        0 if length % 4 == 0 => Ok((20 + length, 0)),
        1 => Ok((4 + length, (4 - length % 4) % 4)),
        _ => Err(invalid()),
    }
}

impl TurnStream {
    /// TLS always validates the server hostname and public certificate chain.
    /// No permissive verifier or legacy Parsec DTLS option applies to TURN TLS.
    pub async fn dial(host: &str, port: u16, tls: bool) -> io::Result<Self> {
        Self::dial_with_qos(host, port, tls, false).await
    }

    pub async fn dial_with_qos(host: &str, port: u16, tls: bool, qos: bool) -> io::Result<Self> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let addresses: Vec<_> = tokio::net::lookup_host((host, port))
                .await?
                .filter(SocketAddr::is_ipv4)
                .collect();
            // ICE relay gathering currently uses UDP4. Try every matching DNS
            // address rather than failing on the first unreachable endpoint.
            if addresses.is_empty() {
                return Err(io::Error::other("TURN server has no IPv4 address"));
            }
            let tcp = TcpStream::connect(addresses.as_slice()).await?;
            tcp.set_nodelay(true)?;
            let marking = evoparseq_socket_qos::Flow::new(&tcp, qos, "WebRTC");
            let local = tcp.local_addr()?;
            let remote = tcp.peer_addr()?;
            let stream: Stream = if tls {
                let mut roots = rustls::RootCertStore::from_iter(
                    webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
                );
                // Include OS-trusted enterprise/self-hosted TURN CAs. Trust is
                // still explicit at OS level; hostname verification is retained.
                roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
                let config = rustls::ClientConfig::builder_with_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .map_err(io::Error::other)?
                .with_root_certificates(roots)
                .with_no_client_auth();
                let name = rustls::pki_types::ServerName::try_from(host.to_owned())
                    .map_err(|_| invalid())?;
                Box::new(
                    tokio_rustls::TlsConnector::from(Arc::new(config))
                        .connect(name, tcp)
                        .await?,
                )
            } else {
                Box::new(tcp)
            };
            let mut connection = Self::from_stream(stream, local, remote, !tls);
            connection._qos = marking;
            Ok(connection)
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TURN connect timed out"))?
    }

    fn from_stream(
        stream: Stream,
        local: SocketAddr,
        remote: SocketAddr,
        buffer_tcp: bool,
    ) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            _qos: Default::default(),
            reader: Mutex::new(Reader {
                stream: reader,
                buffer: buffer_tcp.then(|| ReceiveBuffer {
                    bytes: vec![0; RECEIVE_CAPACITY].into_boxed_slice(),
                    start: 0,
                    end: 0,
                }),
            }),
            writer: Mutex::new(Writer {
                stream: writer,
                padded: vec![0; CHANNEL_CAPACITY].into_boxed_slice(),
            }),
            local,
            remote,
            closed: watch::channel(false).0,
        }
    }
}

#[async_trait]
impl Conn for TurnStream {
    async fn connect(&self, address: SocketAddr) -> util::Result<()> {
        if address != self.remote {
            return Err(invalid().into());
        }
        Ok(())
    }
    async fn recv(&self, buffer: &mut [u8]) -> util::Result<usize> {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "TURN stream closed").into());
        }
        tokio::select! {
            biased;
            _ = closed.changed() => Err(io::Error::new(io::ErrorKind::NotConnected, "TURN stream closed").into()),
            result = async {
                let mut reader = self.reader.lock().await;
                reader.receive(buffer).await
            } => {
                // A short read or malformed frame destroys stream alignment.
                // Make failure terminal rather than interpreting payload as a
                // subsequent header or spinning on EOF in the TURN reader.
                if result.is_err() { self.closed.send_replace(true); }
                result.map_err(Into::into)
            },
        }
    }
    async fn recv_from(&self, buffer: &mut [u8]) -> util::Result<(usize, SocketAddr)> {
        Ok((self.recv(buffer).await?, self.remote))
    }
    async fn send(&self, buffer: &[u8]) -> util::Result<usize> {
        if buffer.len() < 4 {
            return Err(invalid().into());
        }
        let (size, padding) = frame_shape(buffer)?;
        // Some TURN implementations already pad ChannelData on UDP as well.
        if buffer.len() != size && buffer.len() != size + padding {
            return Err(invalid().into());
        }
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "TURN stream closed").into());
        }
        tokio::select! {
            biased;
            _ = closed.changed() => Err(io::Error::new(io::ErrorKind::NotConnected, "TURN stream closed").into()),
            result = async {
                let mut writer = self.writer.lock().await;
                writer.send(buffer, size, padding).await?;
                Ok::<_, io::Error>(buffer.len())
            } => {
                if result.is_err() { self.closed.send_replace(true); }
                result.map_err(Into::into)
            },
        }
    }
    async fn send_to(&self, buffer: &[u8], target: SocketAddr) -> util::Result<usize> {
        self.connect(target).await?;
        self.send(buffer).await
    }
    fn local_addr(&self) -> util::Result<SocketAddr> {
        Ok(self.local)
    }
    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.remote)
    }
    async fn close(&self) -> util::Result<()> {
        self.closed.send_replace(true);
        let _ = tokio::time::timeout(Duration::from_secs(1), async {
            self.writer.lock().await.stream.shutdown().await
        })
        .await;
        Ok(())
    }
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}
