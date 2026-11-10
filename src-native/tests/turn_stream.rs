//! Loopback-only RFC 8656 framing and transport lifetime tests.
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
use webrtc::{ice::turn_stream::TurnStream, util::conn::Conn};

fn binding() -> Vec<u8> {
    let mut packet = vec![0; 20];
    packet[1] = 1;
    packet[4..8].copy_from_slice(&[0x21, 0x12, 0xa4, 0x42]);
    packet
}

fn channel(payload: usize, value: u8) -> Vec<u8> {
    let mut packet = vec![value; 4 + payload];
    packet[..2].copy_from_slice(&[0x40, 1]);
    packet[2..4].copy_from_slice(&(payload as u16).to_be_bytes());
    packet
}

fn framed(packet: &[u8]) -> Vec<u8> {
    let mut result = packet.to_vec();
    result.resize((result.len() + 3) & !3, 0);
    result
}

#[tokio::test]
async fn tcp_receive_burst_preserves_tails_and_maximum_frame_lengths() {
    timeout(Duration::from_secs(10), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut packets = vec![binding()];
        for length in 0..300 {
            packets.push(channel(length, length as u8));
        }
        packets.push(channel(u16::MAX as usize, 0x6a));
        let mut maximum_stun = binding();
        maximum_stun[2..4].copy_from_slice(&65532u16.to_be_bytes());
        // Repeated unknown optional attributes with zero-length bodies.
        for _ in 0..65532 / 4 {
            maximum_stun.extend_from_slice(&[0x80, 0x22, 0, 0]);
        }
        packets.push(maximum_stun);
        packets.push(binding());
        let wire: Vec<u8> = packets.iter().flat_map(|p| framed(p)).collect();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            for chunk in wire.chunks(65531) {
                socket.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        let stream = TurnStream::dial_with_qos("127.0.0.1", address.port(), false, true)
            .await
            .unwrap();
        let mut output = vec![0; 65556];
        for expected in packets {
            let count = stream.recv(&mut output).await.unwrap();
            assert_eq!(&output[..count], expected);
        }
        server.await.unwrap();
        assert!(stream.recv(&mut output).await.is_err());
        assert!(stream.recv(&mut output).await.is_err());
        stream.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn framed_send_normalizes_padding_and_reuses_storage_for_large_messages() {
    timeout(Duration::from_secs(10), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut packets = vec![binding()];
        for length in [0, 1, 2, 3, 4, 65535, 2, 1] {
            let packet = channel(length, 0x5a);
            packets.push(packet.clone());
            if !packet.len().is_multiple_of(4) {
                // Caller-supplied padding must still be normalized to zero.
                let mut padded = packet;
                padded.resize((padded.len() + 3) & !3, 0xef);
                packets.push(padded);
            }
        }
        let expected: Vec<u8> = packets
            .iter()
            .flat_map(|packet| {
                let size = if packet[0] >> 6 == 0 {
                    packet.len()
                } else {
                    4 + u16::from_be_bytes([packet[2], packet[3]]) as usize
                };
                framed(&packet[..size])
            })
            .collect();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut actual = vec![0; expected.len()];
            socket.read_exact(&mut actual).await.unwrap();
            assert_eq!(actual, expected);
            socket.write_all(&binding()).await.unwrap();
        });
        let stream = TurnStream::dial("127.0.0.1", address.port(), false)
            .await
            .unwrap();
        for packet in packets {
            assert_eq!(stream.send(&packet).await.unwrap(), packet.len());
        }
        assert_eq!(stream.recv(&mut [0; 20]).await.unwrap(), 20);
        server.await.unwrap();
        stream.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tcp_truncated_headers_bodies_padding_and_small_outputs_are_terminal() {
    let complete = framed(&channel(3, 0x5a));
    for prefix in 0..complete.len() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stream = TurnStream::dial("127.0.0.1", address.port(), false)
            .await
            .unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.write_all(&complete[..prefix]).await.unwrap();
        socket.shutdown().await.unwrap();
        assert!(timeout(Duration::from_secs(2), stream.recv(&mut [0; 100]))
            .await
            .unwrap()
            .is_err());
        assert!(stream.recv(&mut [0; 100]).await.is_err());
        stream.close().await.unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stream = TurnStream::dial("127.0.0.1", address.port(), false)
        .await
        .unwrap();
    let (mut socket, _) = listener.accept().await.unwrap();
    socket.write_all(&complete).await.unwrap();
    assert!(stream.recv(&mut [0; 6]).await.is_err());
    assert!(stream.recv(&mut [0; 100]).await.is_err());
    stream.close().await.unwrap();
}

#[tokio::test]
async fn canceled_tcp_receive_retains_its_partial_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stream = TurnStream::dial("127.0.0.1", address.port(), false)
        .await
        .unwrap();
    let (mut socket, _) = listener.accept().await.unwrap();
    let wire = framed(&channel(3, 0x5a));
    for range in [0..3, 3..6] {
        socket.write_all(&wire[range]).await.unwrap();
        assert!(
            timeout(Duration::from_millis(10), stream.recv(&mut [0; 100]))
                .await
                .is_err()
        );
    }
    socket.write_all(&wire[6..]).await.unwrap();
    socket.write_all(&binding()).await.unwrap();
    let mut output = [0; 100];
    assert_eq!(stream.recv(&mut output).await.unwrap(), 7);
    assert_eq!(&output[..7], &wire[..7]);
    assert_eq!(stream.recv(&mut output).await.unwrap(), 20);
    stream.close().await.unwrap();
}

async fn binding_client(
    stream: Arc<TurnStream>,
    address: std::net::SocketAddr,
) -> webrtc::turn::client::Client {
    webrtc::turn::client::Client::new(webrtc::turn::client::ClientConfig {
        stun_serv_addr: address.to_string(),
        turn_serv_addr: String::new(),
        username: String::new(),
        password: String::new(),
        realm: String::new(),
        software: String::new(),
        rto_in_ms: 200,
        conn: stream,
        vnet: None,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn reliable_stun_times_out_once_without_udp_retransmissions() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stream = Arc::new(
        TurnStream::dial("127.0.0.1", address.port(), false)
            .await
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut header = [0; 20];
        socket.read_exact(&mut header).await.unwrap();
        let length = u16::from_be_bytes([header[2], header[3]]) as usize;
        socket.read_exact(&mut vec![0; length]).await.unwrap();
        // The upstream UDP timer would send a duplicate after 200 ms.
        assert!(
            timeout(Duration::from_millis(900), socket.read(&mut [0; 100]))
                .await
                .is_err()
        );
    });
    let client = binding_client(stream.clone(), address).await;
    client
        .set_reliable_transport_timeout(Duration::from_millis(800))
        .await
        .unwrap();
    client.listen().await.unwrap();
    let started = std::time::Instant::now();
    let result = timeout(Duration::from_secs(2), client.send_binding_request())
        .await
        .unwrap();
    assert!(result.is_err());
    assert!(started.elapsed() >= Duration::from_millis(700));
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    client.close().await.unwrap();
    stream.close().await.unwrap();
}

#[tokio::test]
async fn reliable_transport_failure_fails_pending_transaction_immediately() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stream = Arc::new(
        TurnStream::dial("127.0.0.1", address.port(), false)
            .await
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.read_exact(&mut [0; 20]).await.unwrap();
        // Drop the transport with a response outstanding.
    });
    let client = binding_client(stream.clone(), address).await;
    client
        .set_reliable_transport_timeout(Duration::from_millis(39500))
        .await
        .unwrap();
    client.listen().await.unwrap();
    assert!(
        timeout(Duration::from_secs(2), client.send_binding_request())
            .await
            .unwrap()
            .is_err()
    );
    server.await.unwrap();
    client.close().await.unwrap();
    stream.close().await.unwrap();
}

#[tokio::test]
async fn fragmented_and_coalesced_turn_frames_preserve_message_boundaries() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stun = binding();
    let expected = stun.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut wire = expected.clone();
        // Three-byte payload plus one padding byte, then another STUN frame.
        wire.extend_from_slice(&[0x40, 1, 0, 3, 10, 20, 30, 0]);
        wire.extend_from_slice(&expected);
        for part in wire.chunks(3) {
            socket.write_all(part).await.unwrap();
            tokio::task::yield_now().await;
        }
        let mut outgoing = [0; 8];
        socket.read_exact(&mut outgoing).await.unwrap();
        assert_eq!(outgoing, [0x40, 2, 0, 3, 40, 50, 60, 0]);
    });
    let stream = TurnStream::dial("127.0.0.1", address.port(), false)
        .await
        .unwrap();
    let mut buffer = [0; 100];
    assert_eq!(stream.recv(&mut buffer).await.unwrap(), 20);
    assert_eq!(&buffer[..20], stun);
    assert_eq!(stream.recv(&mut buffer).await.unwrap(), 7);
    assert_eq!(&buffer[..7], &[0x40, 1, 0, 3, 10, 20, 30]);
    assert_eq!(stream.recv(&mut buffer).await.unwrap(), 20);
    assert_eq!(&buffer[..20], stun);
    assert!(stream.send(&[0x40, 2, 0, 3, 40]).await.is_err());
    assert_eq!(stream.send(&[0x40, 2, 0, 3, 40, 50, 60]).await.unwrap(), 7);
    timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    stream.close().await.unwrap();
}

#[tokio::test]
async fn close_interrupts_a_partial_frame_without_waiting_for_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stream = Arc::new(
        TurnStream::dial("127.0.0.1", address.port(), false)
            .await
            .unwrap(),
    );
    let (mut socket, _) = listener.accept().await.unwrap();
    socket.write_all(&[0, 1]).await.unwrap();
    let reader = stream.clone();
    let pending = tokio::spawn(async move { reader.recv(&mut [0; 100]).await });
    tokio::task::yield_now().await;
    timeout(Duration::from_secs(2), stream.close())
        .await
        .unwrap()
        .unwrap();
    assert!(timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(stream.send(&binding()).await.is_err());
}

#[tokio::test]
async fn invalid_frame_is_terminal_and_cannot_desynchronize_next_receive() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stream = TurnStream::dial("127.0.0.1", address.port(), false)
        .await
        .unwrap();
    let (mut socket, _) = listener.accept().await.unwrap();
    socket.write_all(&[0x80, 0, 0, 0]).await.unwrap();
    assert!(stream.recv(&mut [0; 100]).await.is_err());
    assert!(timeout(Duration::from_secs(1), stream.recv(&mut [0; 100]))
        .await
        .unwrap()
        .is_err());
    stream.close().await.unwrap();
}

#[tokio::test]
async fn tls_rejects_an_untrusted_server_certificate() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
    )
    .unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let _ = tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept(socket)
            .await;
    });
    assert!(TurnStream::dial("localhost", address.port(), true)
        .await
        .is_err());
    timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
