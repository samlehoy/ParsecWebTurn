//! Isolated echo/load protocol. No Parsec account, media, or production settings.
use anyhow::{Context, Result, ensure};
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
    pki_types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::Arc,
    time::{Duration, Instant},
};

pub const NAME: &str = "evoparseq-qos.test";
pub const MAGIC: &[u8; 8] = b"EVOQOS01";
pub const BLOCK: usize = 16 * 1024;
#[derive(Clone, Serialize, Deserialize)]
pub struct Peer {
    pub version: u32,
    pub port: u16,
    pub certificate: Vec<u8>,
    pub token: [u8; 16],
}
pub fn receiver_config(port: u16) -> Result<(Peer, Arc<ServerConfig>)> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificate = rcgen::generate_simple_self_signed(vec![NAME.into()])?;
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certificate.key_pair.serialize_der(),
    ));
    let der = certificate.cert.der().clone();
    let mut token = [0; 16];
    getrandom::fill(&mut token).map_err(|_| anyhow::anyhow!("random source unavailable"))?;
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![der.clone()], key)?;
    Ok((
        Peer {
            version: 1,
            port,
            certificate: der.to_vec(),
            token,
        },
        Arc::new(config),
    ))
}
pub enum Channel {
    Plain(TcpStream),
    Client(Box<StreamOwned<ClientConnection, TcpStream>>),
    Server(Box<StreamOwned<ServerConnection, TcpStream>>),
}
impl Channel {
    fn socket(&self) -> &TcpStream {
        match self {
            Self::Plain(c) => c,
            Self::Client(c) => &c.sock,
            Self::Server(c) => &c.sock,
        }
    }
}
pub fn start(channel: &mut Channel) -> std::io::Result<()> {
    channel.write_all(&[1])?;
    channel.flush()?;
    let mut ready = [0];
    channel.read_exact(&mut ready)?;
    if ready[0] != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "receiver start refused",
        ));
    }
    Ok(())
}
impl Read for Channel {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(c) => c.read(bytes),
            Self::Client(c) => c.read(bytes),
            Self::Server(c) => c.read(bytes),
        }
    }
}
impl Write for Channel {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(c) => c.write(bytes),
            Self::Client(c) => c.write(bytes),
            Self::Server(c) => c.write(bytes),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(c) => c.flush(),
            Self::Client(c) => c.flush(),
            Self::Server(c) => c.flush(),
        }
    }
}
pub fn configure(socket: &TcpStream) -> Result<()> {
    // Windows accepted sockets can inherit the listener's nonblocking mode.
    // This isolated tool deliberately uses synchronous, deadline-bounded IO.
    socket.set_nonblocking(false)?;
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(())
}
pub fn connect(
    addr: SocketAddr,
    peer: &Peer,
    tls: bool,
    mark: bool,
    mode: u8,
    seconds: u32,
) -> Result<(Channel, crate::qos::Flow)> {
    ensure!(
        peer.version == 1 && peer.port < u16::MAX && (1..=30).contains(&seconds),
        "invalid test parameters"
    );
    let socket = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .context("receiver TCP connection failed")?;
    configure(&socket)?;
    #[cfg(windows)]
    if mode != 0 {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock as ws;
        // Isolated load sockets close abortively so queued load cannot spill
        // into the next A/B trial. Never applied to the measured echo socket.
        let linger = ws::LINGER {
            l_onoff: 1,
            l_linger: 0,
        };
        let result = unsafe {
            ws::setsockopt(
                socket.as_raw_socket() as usize,
                ws::SOL_SOCKET,
                ws::SO_LINGER,
                (&linger as *const ws::LINGER).cast(),
                std::mem::size_of::<ws::LINGER>() as i32,
            )
        };
        ensure!(result == 0, "load-socket cleanup configuration failed");
    }
    let flow = crate::qos::Flow::new(&socket, mark);
    let mut channel = if tls {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from(peer.certificate.clone()))?;
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Channel::Client(Box::new(StreamOwned::new(
            ClientConnection::new(Arc::new(config), ServerName::try_from(NAME)?)?,
            socket,
        )))
    } else {
        Channel::Plain(socket)
    };
    channel.write_all(MAGIC)?;
    channel.write_all(&peer.token)?;
    channel.write_all(&[mode])?;
    channel.write_all(&seconds.to_be_bytes())?;
    channel.flush()?;
    let mut ready = [0];
    channel.read_exact(&mut ready)?;
    ensure!(ready[0] == 1, "receiver refused test session");
    Ok((channel, flow))
}
pub fn serve(mut channel: Channel, peer: &Peer) -> Result<()> {
    let mut hello = [0u8; 29];
    channel.read_exact(&mut hello)?;
    ensure!(
        &hello[..8] == MAGIC && hello[8..24] == peer.token,
        "invalid test pairing"
    );
    let mode = hello[24];
    let seconds = u32::from_be_bytes(hello[25..29].try_into()?);
    ensure!(
        mode <= 2 && (1..=30).contains(&seconds),
        "invalid test request"
    );
    channel.write_all(&[1])?;
    channel.flush()?;
    // A paired connection waits for the coordinated client start. Auth/TLS
    // setup has already completed and does not enter the sampling window.
    channel
        .socket()
        .set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut go = [0];
    channel.read_exact(&mut go)?;
    ensure!(go[0] == 1, "invalid start request");
    channel
        .socket()
        .set_read_timeout(Some(Duration::from_secs(5)))?;
    channel.write_all(&[1])?;
    channel.flush()?;
    let until = Instant::now() + Duration::from_secs(u64::from(seconds) + 20);
    let mut buffer = [0u8; BLOCK];
    while Instant::now() < until {
        match mode {
            0 => {
                if channel.read_exact(&mut buffer[..8]).is_err() {
                    break;
                }
                channel.write_all(&buffer[..8])?;
                channel.flush()?;
            }
            1 => {
                if channel.read(&mut buffer)? == 0 {
                    break;
                }
            }
            2 => {
                channel.write_all(&buffer)?;
                channel.flush()?;
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tcp_and_pinned_tls_echo_and_load_without_production_credentials() {
        for tls in [false, true] {
            for mode in [0, 1, 2] {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let addr = listener.local_addr().unwrap();
                let (peer, config) = receiver_config(addr.port().min(65534)).unwrap();
                let server_peer = peer.clone();
                let worker = std::thread::spawn(move || {
                    let (socket, _) = listener.accept().unwrap();
                    configure(&socket).unwrap();
                    let channel = if tls {
                        Channel::Server(Box::new(StreamOwned::new(
                            ServerConnection::new(config).unwrap(),
                            socket,
                        )))
                    } else {
                        Channel::Plain(socket)
                    };
                    let _ = serve(channel, &server_peer);
                });
                let (mut channel, flow) = connect(addr, &peer, tls, false, mode, 1).unwrap();
                assert!(!flow.status.requested && !flow.status.applied);
                start(&mut channel).unwrap();
                if mode == 0 {
                    channel.write_all(&123u64.to_be_bytes()).unwrap();
                    channel.flush().unwrap();
                    let mut echoed = [0; 8];
                    channel.read_exact(&mut echoed).unwrap();
                    assert_eq!(u64::from_be_bytes(echoed), 123);
                } else if mode == 1 {
                    channel.write_all(&[7; BLOCK]).unwrap();
                    channel.flush().unwrap();
                } else {
                    let mut data = [1; BLOCK];
                    channel.read_exact(&mut data).unwrap();
                    assert!(data.iter().all(|b| *b == 0));
                }
                drop(channel);
                drop(flow);
                worker.join().unwrap();
            }
        }
    }
    #[test]
    fn tls_rejects_a_different_receivers_certificate() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (peer, config) = receiver_config(addr.port().min(65534)).unwrap();
        let (mut wrong, _) = receiver_config(peer.port).unwrap();
        // Keep pairing valid: only certificate pinning may reject this client.
        wrong.token = peer.token;
        let worker = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            configure(&socket).unwrap();
            let _ = serve(
                Channel::Server(Box::new(StreamOwned::new(
                    ServerConnection::new(config).unwrap(),
                    socket,
                ))),
                &peer,
            );
        });
        assert!(connect(addr, &wrong, true, false, 0, 1).is_err());
        worker.join().unwrap();
    }
    #[test]
    fn receiver_rejects_an_unpaired_client() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (peer, _) = receiver_config(addr.port().min(65534)).unwrap();
        let mut wrong = peer.clone();
        wrong.token[0] ^= 1;
        let worker = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            configure(&socket).unwrap();
            assert!(serve(Channel::Plain(socket), &peer).is_err());
        });
        assert!(connect(addr, &wrong, false, false, 0, 1).is_err());
        worker.join().unwrap();
    }
}
