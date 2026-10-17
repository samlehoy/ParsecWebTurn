#![allow(dead_code)]
use serde::Serialize;
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use util::Conn;

mod baseline {
    include!(concat!(env!("OUT_DIR"), "/baseline.rs"));
}
mod buffered {
    include!(concat!(env!("OUT_DIR"), "/buffered.rs"));
}

#[derive(Default)]
struct Counts {
    reads: AtomicU64,
    writes: AtomicU64,
    flushes: AtomicU64,
    read_ready: AtomicU64,
    write_ready: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}
#[derive(Serialize)]
struct Snapshot {
    read_polls: u64,
    write_polls: u64,
    flush_polls: u64,
    ready_reads: u64,
    ready_writes: u64,
    bytes_in: u64,
    bytes_out: u64,
}
impl Counts {
    fn snapshot(&self) -> Snapshot {
        let get = |v: &AtomicU64| v.load(Ordering::Relaxed);
        Snapshot {
            read_polls: get(&self.reads),
            write_polls: get(&self.writes),
            flush_polls: get(&self.flushes),
            ready_reads: get(&self.read_ready),
            ready_writes: get(&self.write_ready),
            bytes_in: get(&self.bytes_in),
            bytes_out: get(&self.bytes_out),
        }
    }
    fn reset(&self) {
        for counter in [
            &self.reads,
            &self.writes,
            &self.flushes,
            &self.read_ready,
            &self.write_ready,
            &self.bytes_in,
            &self.bytes_out,
        ] {
            counter.store(0, Ordering::Relaxed);
        }
    }
}
struct Counted<T> {
    inner: T,
    counts: Arc<Counts>,
}
impl<T: AsyncRead + Unpin> AsyncRead for Counted<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.counts.reads.fetch_add(1, Ordering::Relaxed);
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buffer);
        if let Poll::Ready(Ok(())) = &result {
            self.counts.read_ready.fetch_add(1, Ordering::Relaxed);
            self.counts
                .bytes_in
                .fetch_add((buffer.filled().len() - before) as u64, Ordering::Relaxed);
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Counted<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.counts.writes.fetch_add(1, Ordering::Relaxed);
        let result = Pin::new(&mut self.inner).poll_write(cx, buffer);
        if let Poll::Ready(Ok(count)) = &result {
            self.counts.write_ready.fetch_add(1, Ordering::Relaxed);
            self.counts
                .bytes_out
                .fetch_add(*count as u64, Ordering::Relaxed);
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.counts.flushes.fetch_add(1, Ordering::Relaxed);
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
trait Io: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> Io for T {}
type Stream = Box<dyn Io>;

struct Certificates {
    server: Arc<rustls::ServerConfig>,
    client: Arc<rustls::ClientConfig>,
}
fn certificates() -> Certificates {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.cert.der().clone()).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
        )
        .unwrap();
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Certificates {
        server: Arc::new(server),
        client: Arc::new(client),
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn frames(size: usize, count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|index| {
            let mut bytes = vec![0x55; size];
            bytes[0..2].copy_from_slice(&[0x40, 0]);
            bytes[2..4].copy_from_slice(&((size - 4) as u16).to_be_bytes());
            bytes[4..8].copy_from_slice(&(index as u32).to_be_bytes());
            bytes
        })
        .collect()
}
fn wire(messages: &[Vec<u8>]) -> Vec<u8> {
    let mut result = Vec::new();
    for message in messages {
        result.extend_from_slice(message);
        result.resize(result.len() + (4 - message.len() % 4) % 4, 0);
    }
    result
}
enum Client {
    Baseline(baseline::TurnStream),
    Buffered(buffered::TurnStream),
    Batched(buffered::TurnStream),
}
impl Client {
    fn new(policy: &str, stream: Stream, local: SocketAddr, remote: SocketAddr) -> Self {
        match policy {
            "current" => Self::Baseline(baseline::fixture(stream, local, remote)),
            "buffered_rx" => Self::Buffered(buffered::fixture(stream, local, remote)),
            "buffered_rx_ready_tx" => Self::Batched(buffered::fixture(stream, local, remote)),
            _ => unreachable!(),
        }
    }
    async fn receive(&self, out: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Baseline(c) => c.recv(out).await,
            Self::Buffered(c) | Self::Batched(c) => c.recv(out).await,
        }
        .map_err(io::Error::other)
    }
    async fn send(&self, messages: &[Vec<u8>]) -> io::Result<()> {
        match self {
            Self::Batched(c) => c.send_ready(messages).await.map_err(io::Error::other),
            _ => {
                for message in messages {
                    match self {
                        Self::Baseline(c) => c.send(message).await,
                        Self::Buffered(c) => c.send(message).await,
                        _ => unreachable!(),
                    }
                    .map_err(io::Error::other)?;
                }
                Ok(())
            }
        }
    }
    async fn close(&self) {
        match self {
            Self::Baseline(c) => c.close().await.unwrap(),
            Self::Buffered(c) | Self::Batched(c) => c.close().await.unwrap(),
        }
    }
}

#[derive(Serialize)]
struct Trial {
    repetition: usize,
    policy: String,
    carrier: String,
    workload: String,
    messages: usize,
    wire_bytes_one_direction: usize,
    elapsed_ms: f64,
    one_direction_mib_s: f64,
    latency_p50_us: f64,
    latency_p95_us: f64,
    latency_p99_us: f64,
    first_frame_p95_us: f64,
    plaintext_io: Snapshot,
    socket_io: Snapshot,
}
fn percentile(values: &mut [f64], p: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * p).ceil() as usize]
}

async fn trial(
    repetition: usize,
    policy: &str,
    tls: bool,
    workload: &str,
    cert: &Certificates,
    counted: bool,
) -> io::Result<Trial> {
    let push = workload == "receive_burst";
    let fragmented = workload == "fragmented_correctness";
    let (size, count, rounds) = match workload {
        "single_echo" => (260, 1, 2000),
        "fragmented_correctness" => (1023, 8, 2),
        _ => (1023, 64, 512),
    };
    let messages = frames(size, count);
    let group = wire(&messages);
    let one_direction = group.len() * rounds;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let server_config = cert.server.clone();
    let server = std::thread::spawn(move || {
        runtime().block_on(async {
            tokio::time::timeout(Duration::from_secs(30), async {
                let listener = tokio::net::TcpListener::from_std(listener)?;
                let (socket, _) = listener.accept().await?;
                socket.set_nodelay(true)?;
                let mut connection: Stream = if tls {
                    let secured = tokio_rustls::TlsAcceptor::from(server_config)
                        .accept(socket)
                        .await?;
                    assert_eq!(
                        secured.get_ref().1.protocol_version(),
                        Some(rustls::ProtocolVersion::TLSv1_3)
                    );
                    Box::new(secured)
                } else {
                    Box::new(socket)
                };
                if push {
                    // Do not start the burst until client fixture/counters are ready.
                    let mut ready = [0u8; 1];
                    connection.read_exact(&mut ready).await?;
                    assert_eq!(ready, [0x7f]);
                    for _ in 0..rounds {
                        connection.write_all(&group).await?;
                    }
                    connection.flush().await?;
                } else {
                    let mut buffer = vec![0u8; 64 * 1024];
                    let mut total = 0;
                    while total < one_direction {
                        let read = connection
                            .read(&mut buffer[..(one_direction - total).min(64 * 1024)])
                            .await?;
                        if read == 0 {
                            return Err(io::ErrorKind::UnexpectedEof.into());
                        }
                        if fragmented {
                            for chunk in buffer[..read].chunks(3) {
                                connection.write_all(chunk).await?;
                                connection.flush().await?;
                                tokio::task::yield_now().await;
                            }
                        } else {
                            connection.write_all(&buffer[..read]).await?;
                            connection.flush().await?;
                        }
                        total += read;
                    }
                }
                // Keep the peer open until the client consumed the last message.
                let mut final_byte = [0u8; 1];
                let _ = connection.read(&mut final_byte).await?;
                Ok::<_, io::Error>(())
            })
            .await
            .map_err(io::Error::other)?
        })
    });
    let socket = tokio::net::TcpStream::connect(address).await?;
    socket.set_nodelay(true)?;
    let local = socket.local_addr()?;
    let socket_counts = Arc::new(Counts::default());
    let socket: Stream = if counted {
        Box::new(Counted {
            inner: socket,
            counts: socket_counts.clone(),
        })
    } else {
        Box::new(socket)
    };
    let mut stream: Stream = if tls {
        let secured = tokio_rustls::TlsConnector::from(cert.client.clone())
            .connect(
                rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                socket,
            )
            .await?;
        assert_eq!(
            secured.get_ref().1.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );
        Box::new(secured)
    } else {
        socket
    };
    let plain_counts = Arc::new(Counts::default());
    // Sending this barrier is outside measurement; it is not a TURN frame.
    if push {
        stream.write_all(&[0x7f]).await?;
        stream.flush().await?;
    }
    let stream: Stream = if counted {
        Box::new(Counted {
            inner: stream,
            counts: plain_counts.clone(),
        })
    } else {
        stream
    };
    let connection = Client::new(policy, stream, local, address);
    socket_counts.reset();
    let mut result = vec![0; 4096];
    let mut latencies = Vec::with_capacity(rounds);
    let mut first = Vec::with_capacity(rounds);
    let start = Instant::now();
    for _ in 0..rounds {
        let begin = Instant::now();
        if !push {
            connection.send(&messages).await?;
        }
        for (index, message) in messages.iter().enumerate() {
            let n = connection.receive(&mut result).await?;
            if result[..n] != message[..] {
                return Err(io::Error::other("Frame mismatch"));
            }
            if index == 0 {
                first.push(begin.elapsed().as_secs_f64() * 1e6);
            }
        }
        latencies.push(begin.elapsed().as_secs_f64() * 1e6);
    }
    let elapsed = start.elapsed();
    let plain = plain_counts.snapshot();
    let socket = socket_counts.snapshot();
    connection.close().await;
    server
        .join()
        .map_err(|_| io::Error::other("Server panic"))??;
    if counted {
        assert_eq!(plain.bytes_in as usize, one_direction);
        assert_eq!(
            plain.bytes_out as usize,
            if push { 0 } else { one_direction }
        );
    }
    Ok(Trial {
        repetition,
        policy: policy.into(),
        carrier: if tls { "tls13" } else { "tcp" }.into(),
        workload: workload.into(),
        messages: count * rounds,
        wire_bytes_one_direction: one_direction,
        elapsed_ms: elapsed.as_secs_f64() * 1000.,
        one_direction_mib_s: one_direction as f64 / (1024. * 1024.) / elapsed.as_secs_f64(),
        latency_p50_us: percentile(&mut latencies, 0.5),
        latency_p95_us: percentile(&mut latencies, 0.95),
        latency_p99_us: percentile(&mut latencies, 0.99),
        first_frame_p95_us: percentile(&mut first, 0.95),
        plaintext_io: plain,
        socket_io: socket,
    })
}

fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    let output = arguments.get(1).expect("Output JSON path");
    let repetitions: usize = arguments.get(2).map(|v| v.parse().unwrap()).unwrap_or(6);
    let counted = arguments.get(3).is_some_and(|v| v == "counted");
    let cert = certificates();
    let mut trials = Vec::new();
    let runtime = runtime();
    for tls in [false, true] {
        for workload in [
            "single_echo",
            "duplex_burst",
            "receive_burst",
            "fragmented_correctness",
        ] {
            for repetition in 0..=repetitions {
                for order in 0..3 {
                    let policy = ["current", "buffered_rx", "buffered_rx_ready_tx"]
                        [(order + repetition) % 3];
                    let result = runtime.block_on(async {
                        tokio::time::timeout(Duration::from_secs(45), async {
                            trial(repetition, policy, tls, workload, &cert, counted).await
                        })
                        .await
                    });
                    let result = result.expect("Trial timeout").expect("Trial failed");
                    eprintln!(
                        "{} {} {} rep {}: {:.1} MiB/s, p95 {:.1} us",
                        result.carrier,
                        workload,
                        policy,
                        repetition,
                        result.one_direction_mib_s,
                        result.latency_p95_us
                    );
                    if repetition != 0 {
                        trials.push(result);
                    }
                }
            }
        }
    }
    let document = serde_json::json!({"schema":1,"instrumented":counted,"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "runtime":"Tokio current-thread; server on a separate OS thread", "tls":"rustls TLS1.3; explicit isolated CA; hostname verified; no resumption",
        "baseline":"Production TurnStream source compiled by build.rs; fixture constructor only",
        "candidate":"BUD-style 64KiB receive and bounded 32KiB ready-only transmit, adapted to Conn copy-out semantics",
        "warmup_repetitions_discarded":1,"measured_repetitions":repetitions,"trials":trials});
    std::fs::write(output, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
}
