//! Read-only investigation of the pinned TURN client's renewal behavior.
//! Loopback, synthetic credentials; no Parsec core or external service.
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::UdpSocket, sync::mpsc, time::timeout};
use webrtc::{
    stun::{
        attributes::*, error_code::*, fingerprint::FINGERPRINT, integrity::MessageIntegrity,
        message::*, textattrs::*,
    },
    turn::{
        client::{Client, ClientConfig},
        proto::{lifetime::Lifetime, relayaddr::RelayedAddress},
    },
    util::Conn,
};

#[derive(Clone, Copy)]
enum Scenario {
    RefreshStale,
    RefreshDenied,
    BindStale,
    BindRefresh,
}

#[derive(Debug)]
struct Observation {
    method: Method,
    fresh_nonce: bool,
    channel_number: Option<u16>,
}

async fn observe(scenario: Scenario) -> Vec<Observation> {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = server.local_addr().unwrap();
    let (tx, mut rx) = mpsc::channel(32);
    let worker = tokio::spawn(async move {
        let integrity = MessageIntegrity::new_long_term_integrity(
            "fixture".into(),
            "fixture".into(),
            "fixture-password".into(),
        );
        let mut buffer = [0; 2048];
        let mut challenged = false;
        loop {
            let (n, from) = server.recv_from(&mut buffer).await.unwrap();
            if !is_message(&buffer[..n]) {
                continue;
            }
            let mut request = Message {
                raw: buffer[..n].to_vec(),
                ..Default::default()
            };
            request.decode().unwrap();
            let method = request.typ.method;
            let nonce = Nonce::get_from_as(&request, ATTR_NONCE).ok();
            if method == METHOD_REFRESH || method == METHOD_CHANNEL_BIND {
                let mut number = webrtc::turn::proto::channum::ChannelNumber::default();
                let channel_number = number.get_from(&request).ok().map(|_| number.0);
                let _ = tx
                    .send(Observation {
                        method,
                        fresh_nonce: nonce.as_ref().is_some_and(|n| n.text == "fixture-rotated"),
                        channel_number,
                    })
                    .await;
            }
            if request.typ.class == CLASS_INDICATION {
                continue;
            }
            let response = {
                let mut attributes: Vec<Box<dyn Setter>> = vec![Box::new(request.transaction_id)];
                let mut error = None;
                if method == METHOD_ALLOCATE {
                    if nonce.is_none() {
                        error = Some(CODE_UNAUTHORIZED);
                    } else {
                        attributes.push(Box::new(RelayedAddress {
                            ip: "127.0.0.1".parse().unwrap(),
                            port: 49999,
                        }));
                        attributes.push(Box::new(Lifetime(Duration::from_secs(
                            if matches!(scenario, Scenario::BindStale | Scenario::BindRefresh) {
                                600
                            } else {
                                2
                            },
                        ))));
                    }
                } else if method == METHOD_REFRESH {
                    let mut life = Lifetime::default();
                    life.get_from(&request).unwrap();
                    if !life.0.is_zero() {
                        if matches!(scenario, Scenario::RefreshDenied) {
                            error = Some(CODE_FORBIDDEN);
                        } else if !challenged {
                            challenged = true;
                            error = Some(CODE_STALE_NONCE);
                        }
                    }
                    if error.is_none() {
                        attributes.push(Box::new(Lifetime(life.0)));
                    }
                } else if method == METHOD_CHANNEL_BIND && matches!(scenario, Scenario::BindStale) {
                    error = Some(CODE_STALE_NONCE);
                }
                if let Some(code) = error {
                    attributes.push(Box::new(ErrorCodeAttribute {
                        code,
                        reason: vec![],
                    }));
                    if code == CODE_UNAUTHORIZED || code == CODE_STALE_NONCE {
                        attributes.push(Box::new(Realm::new(ATTR_REALM, "fixture".into())));
                        attributes.push(Box::new(Nonce::new(
                            ATTR_NONCE,
                            if code == CODE_STALE_NONCE {
                                "fixture-rotated"
                            } else {
                                "fixture-initial"
                            }
                            .into(),
                        )));
                    }
                }
                attributes.push(Box::new(MessageType::new(
                    method,
                    if error.is_some() {
                        CLASS_ERROR_RESPONSE
                    } else {
                        CLASS_SUCCESS_RESPONSE
                    },
                )));
                if nonce.is_some() {
                    attributes.push(Box::new(integrity.clone()));
                }
                attributes.push(Box::new(FINGERPRINT));
                let mut response = Message::new();
                response.build(&attributes).unwrap();
                response
            };
            server.send_to(&response.raw, from).await.unwrap();
        }
    });
    let client = Client::new(ClientConfig {
        stun_serv_addr: address.to_string(),
        turn_serv_addr: address.to_string(),
        username: "fixture".into(),
        password: "fixture-password".into(),
        realm: "fixture".into(),
        software: String::new(),
        rto_in_ms: 50,
        conn: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
        vnet: None,
    })
    .await
    .unwrap();
    client.listen().await.unwrap();
    let relay = client.allocate().await.unwrap();
    let mut observations = Vec::new();
    if matches!(scenario, Scenario::BindStale | Scenario::BindRefresh) {
        let peer: SocketAddr = "127.0.0.1:49998".parse().unwrap();
        relay.send_to(b"synthetic", peer).await.unwrap();
        observations.push(
            timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap(),
        );
        // Allow the background binding response to be processed, then send again.
        tokio::time::sleep(Duration::from_millis(100)).await;
        if matches!(scenario, Scenario::BindRefresh) {
            // Exercise the real diagnostic threshold, not a shortened test-only
            // replacement. Allocation (300 s) and permission (120 s) timers
            // cannot fire before this observation completes.
            tokio::time::sleep(Duration::from_millis(60100)).await;
        }
        relay.send_to(b"synthetic", peer).await.unwrap();
        if matches!(scenario, Scenario::BindRefresh) {
            observations.push(
                timeout(Duration::from_secs(3), rx.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        } else {
            assert!(timeout(Duration::from_millis(200), rx.recv())
                .await
                .is_err());
        }
    } else {
        for _ in 0..3 {
            observations.push(
                timeout(Duration::from_secs(3), rx.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
    }
    relay.close().await.unwrap();
    client.close().await.unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    observations
}

#[tokio::test]
#[ignore = "60-second opt-in experiment; run in a separate process with renewal-diagnostics"]
async fn early_channel_bind_refresh_only() {
    assert_eq!(
        std::env::var("PARSECWEBTURN_TURN_EARLY_BIND_REFRESH").as_deref(),
        Ok("1")
    );
    let start = std::time::Instant::now();
    let seen = timeout(Duration::from_secs(70), observe(Scenario::BindRefresh))
        .await
        .unwrap();
    assert!(start.elapsed() >= Duration::from_secs(60));
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|o| o.method == METHOD_CHANNEL_BIND));
    assert!(seen[0].channel_number.is_some());
    assert_eq!(seen[0].channel_number, seen[1].channel_number);
    println!("renewal-observation: same channel refreshed after 60 seconds, before any allocation or permission timer");
}

#[tokio::test]
async fn pinned_allocation_refresh_retries_stale_nonce_with_rotated_value() {
    let seen = timeout(Duration::from_secs(8), observe(Scenario::RefreshStale))
        .await
        .unwrap();
    assert!(seen.iter().all(|o| o.method == METHOD_REFRESH));
    assert!(!seen[0].fresh_nonce);
    assert!(seen[1].fresh_nonce);
    println!("renewal-observation: allocation 438 is retried using the replacement nonce");
}

#[tokio::test]
async fn observe_pinned_refresh_rejection_and_binding_nonce_limitations() {
    let denied = timeout(Duration::from_secs(8), observe(Scenario::RefreshDenied))
        .await
        .unwrap();
    assert_eq!(denied.len(), 3);
    let binding = timeout(Duration::from_secs(8), observe(Scenario::BindStale))
        .await
        .unwrap();
    assert_eq!(binding.len(), 1);
    assert_eq!(binding[0].method, METHOD_CHANNEL_BIND);
    assert!(!binding[0].fresh_nonce);
    // These observations document the current library, not an ExpressTURN cause.
    // Update this investigation fixture when implementing standards-based fixes.
    println!("renewal-observation: rejected refresh repeats 3 times; ChannelBind 438 is not retried, including after a second send");
}
