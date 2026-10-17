//! Isolated provider comparison, with one identical rustls loopback server.
//! No live host, production certificates, credentials, GUI or GPU is used.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::{PrivatePkcs8KeyDer, ServerName};
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};
use serde::Serialize;

const HOST: &str = "relay.benchmark.test";
const PINGS: usize = 1000;
const WARM_PINGS: usize = 100;
const BYTES: usize = 64 * 1024 * 1024;
const CHUNK: usize = 16 * 1024;
const REPEATS: usize = 7;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Serialize)]
struct Trial {
    provider: &'static str,
    protocol: String,
    cipher: String,
    handshake_us: f64,
    ping_p50_us: f64,
    ping_p95_us: f64,
    ping_p99_us: f64,
    upload_mib_s: f64,
    download_mib_s: f64,
    client_cpu_ms: f64,
    client_cpu_ms_per_mib: f64,
    upload_cpu_ms_per_mib: f64,
    download_cpu_ms_per_mib: f64,
}

enum Client {
    Rustls(Box<StreamOwned<ClientConnection, TcpStream>>),
    Native(Box<native_tls::TlsStream<TcpStream>>),
}
impl Read for Client {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Rustls(s) => s.read(bytes),
            Self::Native(s) => s.read(bytes),
        }
    }
}
impl Write for Client {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Rustls(s) => s.write(bytes),
            Self::Native(s) => s.write(bytes),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Rustls(s) => s.flush(),
            Self::Native(s) => s.flush(),
        }
    }
}

#[cfg(windows)]
fn cpu_ms() -> io::Result<f64> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: current thread pseudo-handle and writable FILETIME storage.
    if unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Ok((ticks(kernel) + ticks(user)) as f64 / 10_000.0)
}
#[cfg(not(windows))]
fn cpu_ms() -> io::Result<f64> {
    Err(io::Error::other("Schannel comparison must run on Windows"))
}

fn socket_options(socket: &TcpStream) -> io::Result<()> {
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(Duration::from_secs(15)))?;
    socket.set_write_timeout(Some(Duration::from_secs(15)))
}

fn serve(socket: TcpStream, config: Arc<ServerConfig>) -> Result<(String, String)> {
    socket_options(&socket)?;
    let mut stream = StreamOwned::new(ServerConnection::new(config)?, socket);
    let mut ping = [0u8; 256];
    for _ in 0..WARM_PINGS + PINGS {
        stream.read_exact(&mut ping)?;
        stream.write_all(&ping)?;
        stream.flush()?;
    }
    let mut chunk = [0u8; CHUNK];
    for _ in 0..BYTES / CHUNK {
        stream.read_exact(&mut chunk)?;
        if chunk.iter().any(|b| *b != 0x5a) {
            return Err("upload payload mismatch".into());
        }
    }
    stream.write_all(&[1])?;
    stream.flush()?;
    chunk.fill(0xa5);
    for _ in 0..BYTES / CHUNK {
        stream.write_all(&chunk)?;
    }
    stream.flush()?;
    stream.read_exact(&mut [0u8; 1])?;
    let protocol = format!(
        "{:?}",
        stream.conn.protocol_version().ok_or("no TLS version")?
    );
    let cipher = format!(
        "{:?}",
        stream
            .conn
            .negotiated_cipher_suite()
            .ok_or("no cipher")?
            .suite()
    );
    Ok((protocol, cipher))
}

fn trial(
    native: bool,
    server: Arc<ServerConfig>,
    client: Arc<ClientConfig>,
    connector: &native_tls::TlsConnector,
) -> Result<Trial> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let peer = std::thread::spawn(move || -> Result<_> {
        let (socket, _) = listener.accept()?;
        serve(socket, server)
    });
    let socket = TcpStream::connect(address)?;
    socket_options(&socket)?;
    let started = Instant::now();
    let mut stream = if native {
        Client::Native(Box::new(connector.connect(HOST, socket)?))
    } else {
        let mut conn = ClientConnection::new(client, ServerName::try_from(HOST)?)?;
        let mut socket = socket;
        while conn.is_handshaking() {
            conn.complete_io(&mut socket)?;
        }
        Client::Rustls(Box::new(StreamOwned::new(conn, socket)))
    };
    let handshake_us = started.elapsed().as_secs_f64() * 1e6;
    let outgoing = [0x27; 256];
    let mut incoming = [0; 256];
    for _ in 0..WARM_PINGS {
        stream.write_all(&outgoing)?;
        stream.flush()?;
        stream.read_exact(&mut incoming)?;
    }
    let mut pings = Vec::with_capacity(PINGS);
    for _ in 0..PINGS {
        let started = Instant::now();
        stream.write_all(&outgoing)?;
        stream.flush()?;
        stream.read_exact(&mut incoming)?;
        pings.push(started.elapsed().as_secs_f64() * 1e6);
        if incoming != outgoing {
            return Err("ping payload mismatch".into());
        }
    }
    let cpu_start = cpu_ms()?;
    let mut chunk = [0x5a; CHUNK];
    let started = Instant::now();
    for _ in 0..BYTES / CHUNK {
        stream.write_all(&chunk)?;
    }
    stream.flush()?;
    stream.read_exact(&mut [0u8; 1])?;
    let upload = started.elapsed().as_secs_f64();
    let cpu_after_upload = cpu_ms()?;
    let started = Instant::now();
    for _ in 0..BYTES / CHUNK {
        stream.read_exact(&mut chunk)?;
        if chunk.iter().any(|b| *b != 0xa5) {
            return Err("download payload mismatch".into());
        }
    }
    let download = started.elapsed().as_secs_f64();
    let cpu_after_download = cpu_ms()?;
    let cpu = cpu_after_download - cpu_start;
    stream.write_all(&[1])?;
    stream.flush()?;
    let (protocol, cipher) = peer.join().map_err(|_| "server panicked")??;
    pings.sort_by(f64::total_cmp);
    let percentile = |fraction: f64| pings[((pings.len() - 1) as f64 * fraction).round() as usize];
    let mib = BYTES as f64 / (1024.0 * 1024.0);
    Ok(Trial {
        provider: if native {
            "native-tls / Schannel"
        } else {
            "rustls / ring"
        },
        protocol,
        cipher,
        handshake_us,
        ping_p50_us: percentile(0.50),
        ping_p95_us: percentile(0.95),
        ping_p99_us: percentile(0.99),
        upload_mib_s: mib / upload,
        download_mib_s: mib / download,
        client_cpu_ms: cpu,
        client_cpu_ms_per_mib: cpu / (mib * 2.0),
        upload_cpu_ms_per_mib: (cpu_after_upload - cpu_start) / mib,
        download_cpu_ms_per_mib: (cpu_after_download - cpu_after_upload) / mib,
    })
}

fn main() -> Result<()> {
    if !cfg!(windows) {
        return Err("run this comparison on Windows (Schannel)".into());
    }
    let mut ca_params = rcgen::CertificateParams::default();
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "PWT isolated benchmark CA");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    let ca_key = rcgen::KeyPair::generate()?;
    let ca = ca_params.self_signed(&ca_key)?;
    let key = rcgen::KeyPair::generate()?;
    let mut leaf_params = rcgen::CertificateParams::new(vec![HOST.into()])?;
    leaf_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, HOST);
    let leaf = leaf_params.signed_by(&key, &ca, &ca_key)?;
    let private = PrivatePkcs8KeyDer::from(key.serialize_der());
    let mut trials = Vec::new();
    for tls13 in [false, true] {
        let mut provider = rustls::crypto::ring::default_provider();
        provider.cipher_suites.retain(|s| {
            matches!(
                s.suite(),
                rustls::CipherSuite::TLS13_AES_128_GCM_SHA256
                    | rustls::CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            )
        });
        let provider = Arc::new(provider);
        let version = if tls13 {
            &rustls::version::TLS13
        } else {
            &rustls::version::TLS12
        };
        let mut server = ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&[version])?
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.der().clone(), ca.der().clone()],
                private.clone_key().into(),
            )?;
        server.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
        server.send_tls13_tickets = 0;
        let server = Arc::new(server);
        let mut roots = RootCertStore::empty();
        roots.add(ca.der().clone())?;
        let mut client = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[version])?
            .with_root_certificates(roots)
            .with_no_client_auth();
        client.resumption = rustls::client::Resumption::disabled();
        let client = Arc::new(client);
        let protocol = if tls13 {
            native_tls::Protocol::Tlsv13
        } else {
            native_tls::Protocol::Tlsv12
        };
        let connector = native_tls::TlsConnector::builder()
            .min_protocol_version(Some(protocol))
            .max_protocol_version(Some(protocol))
            .disable_built_in_roots(true)
            .add_root_certificate(native_tls::Certificate::from_der(ca.der())?)
            .build()?;
        for repeat in 0..REPEATS + 1 {
            for native in if repeat % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                eprintln!(
                    "TLS {} pair {}/{}: {}",
                    if tls13 { "1.3" } else { "1.2" },
                    repeat,
                    REPEATS,
                    if native { "Schannel" } else { "rustls" }
                );
                let row = trial(native, Arc::clone(&server), Arc::clone(&client), &connector)?;
                if repeat != 0 {
                    trials.push(row);
                }
            }
        }
    }
    let result = serde_json::json!({ "schema":2, "scope":"Windows loopback TLS provider comparison",
        "rustls":"0.23.46 / ring", "native_tls":"0.2.18 / Schannel",
        "server":"identical rustls server; AES-128-GCM only; resumption disabled",
        "build_profile":"client release settings: opt-level=s, thin LTO, one codegen unit, static CRT",
        "repeats":REPEATS, "discarded_warmup_pairs_per_protocol":1,
        "pings_per_trial":PINGS, "ping_bytes":256, "stream_bytes_per_direction":BYTES,
        "stream_chunk_bytes":CHUNK, "cpu_scope":"client thread kernel + user time; excludes server thread",
        "limitations":["loopback, not WAN/TURN latency", "no async IOCP backpressure validation",
            "no binary size comparison", "not the client's full media pipeline"], "trials":trials });
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tls-comparison.json".into());
    std::fs::write(&output, serde_json::to_vec_pretty(&result)?)?;
    eprintln!("Results: {output}");
    Ok(())
}
