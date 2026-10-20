//! Separate CI fixture; never linked into the shipped client.
#[cfg(test)]
#[path = "../../../src/signaling.rs"]
mod signaling;
#[cfg(test)]
#[path = "../../turn_stream.rs"]
mod stream;

#[cfg(test)]
mod integration {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tokio::{
        sync::mpsc,
        time::{sleep, timeout},
    };
    use webrtc::{
        api::{setting_engine::SettingEngine, APIBuilder},
        data_channel::{
            data_channel_init::RTCDataChannelInit, data_channel_state::RTCDataChannelState,
        },
        ice::network_type::NetworkType,
        ice_transport::ice_server::RTCIceServer,
        peer_connection::{
            configuration::RTCConfiguration, policy::ice_transport_policy::RTCIceTransportPolicy,
        },
    };

    async fn exchange(url: &str) {
        exchange_urls(&[url], url, false).await;
    }

    async fn exchange_urls(urls: &[&str], url: &str, prefer_udp: bool) {
        exchange_with_compact_candidates(urls, url, prefer_udp, false, false).await;
    }
    async fn exchange_with_compact_candidates(
        urls: &[&str],
        url: &str,
        prefer_udp: bool,
        compact: bool,
        full_discovery: bool,
    ) {
        let mut settings = SettingEngine::default();
        settings.set_network_types(vec![NetworkType::Udp4]);
        settings.set_data_channel_only(true);
        settings.set_prefer_turn_udp(prefer_udp);
        settings.detach_data_channels();
        settings.set_sctp_max_message_size_can_send(
            webrtc::api::setting_engine::SctpMaxMessageSize::Bounded(262144),
        );
        let host_api = APIBuilder::new()
            .with_setting_engine(settings.clone())
            .build();
        settings.set_relay_only_connectivity(full_discovery);
        let api = APIBuilder::new().with_setting_engine(settings).build();
        let a = Arc::new(
            api.new_peer_connection(RTCConfiguration {
                ice_transport_policy: if full_discovery {
                    RTCIceTransportPolicy::All
                } else {
                    RTCIceTransportPolicy::Relay
                },
                ice_servers: {
                    let mut servers = vec![RTCIceServer {
                        urls: urls.iter().map(|url| (*url).into()).collect(),
                        username: "fixture".into(),
                        credential: "fixture-password".into(),
                    }];
                    if full_discovery {
                        servers.push(RTCIceServer {
                            urls: vec!["stun:localhost:43478".into()],
                            ..Default::default()
                        });
                    }
                    servers
                },
                ..Default::default()
            })
            .await
            .unwrap(),
        );
        let b = Arc::new(
            host_api
                .new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap(),
        );
        let (tx, mut rx) = mpsc::channel(8);
        let mut channels = Vec::new();
        for id in 0..3 {
            let config = Some(RTCDataChannelInit {
                negotiated: Some(id),
                ordered: Some(true),
                ..Default::default()
            });
            let ca = a
                .create_data_channel("fixture", config.clone())
                .await
                .unwrap();
            let cb = b.create_data_channel("fixture", config).await.unwrap();
            for (side, channel) in [(0, &ca), (1, &cb)] {
                let tx = tx.clone();
                let weak = Arc::downgrade(channel);
                channel.on_open(Box::new(move || {
                    let tx = tx.clone();
                    let weak = weak.clone();
                    Box::pin(async move {
                        let detached = weak.upgrade().unwrap().detach().await.unwrap();
                        tokio::spawn(async move {
                            let mut buffer = vec![0; 262144];
                            while let Ok((length, text)) =
                                detached.read_data_channel(&mut buffer).await
                            {
                                if length == 0 {
                                    break;
                                }
                                assert!(!text);
                                if tx
                                    .send((
                                        side,
                                        id,
                                        bytes::Bytes::copy_from_slice(&buffer[..length]),
                                    ))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        });
                    })
                }));
            }
            channels.push((ca, cb));
        }
        // Real trickle ICE: a blocked allocation must not hold up fallback
        // connectivity until gathering-complete, just like the native client.
        let (atx, mut arx) = mpsc::unbounded_channel();
        let (btx, mut brx) = mpsc::unbounded_channel();
        let discovered = Arc::new(AtomicUsize::new(0));
        let discovery = discovered.clone();
        a.on_ice_candidate(Box::new(move |candidate| {
            let tx = atx.clone();
            let discovery = discovery.clone();
            Box::pin(async move {
                if let Some(candidate) = candidate {
                    let kind = candidate.typ.to_string();
                    let relay = kind == "relay";
                    if !full_discovery {
                        assert!(relay);
                    }
                    discovery.fetch_or(
                        match kind.as_str() {
                            "host" => 1,
                            "srflx" => 2,
                            "relay" => 4,
                            _ => 0,
                        },
                        Ordering::SeqCst,
                    );
                    let mut init = candidate.to_json().unwrap();
                    if compact && relay {
                        // Parsec's compact address/port/fromStun ABI cannot carry
                        // a relay type. Its remote adapter rebuilds typ host for
                        // fromStun=false. The real local candidate stays relay.
                        init.candidate = format!(
                            "candidate:2395300328 1 udp 2113937151 {} {} typ host",
                            candidate.address, candidate.port
                        );
                    }
                    let _ = tx.send(Some((init, relay)));
                } else {
                    let _ = tx.send(None);
                }
            })
        }));
        b.on_ice_candidate(Box::new(move |candidate| {
            let tx = btx.clone();
            Box::pin(async move {
                if let Some(candidate) = candidate {
                    let _ = tx.send(candidate.to_json().unwrap());
                }
            })
        }));
        a.set_local_description(a.create_offer(None).await.unwrap())
            .await
            .unwrap();
        b.set_remote_description(a.local_description().await.unwrap())
            .await
            .unwrap();
        b.set_local_description(b.create_answer(None).await.unwrap())
            .await
            .unwrap();
        a.set_remote_description(b.local_description().await.unwrap())
            .await
            .unwrap();
        let description =
            crate::signaling::Description::from_sdp(&b.local_description().await.unwrap().sdp)
                .unwrap();
        let host_description =
            crate::signaling::Description::from_sdp(&a.local_description().await.unwrap().sdp)
                .unwrap();
        let remote = Arc::downgrade(&b);
        let forward_a = tokio::spawn(async move {
            let mut host_gate = crate::signaling::CandidateGate::new("relay-readiness").unwrap();
            host_gate
                .remote_ready(
                    "relay-readiness",
                    &host_description.mid,
                    &host_description.credentials.ufrag,
                )
                .unwrap();
            let mut local_sync = crate::signaling::LocalCandidateSync::default();
            let mut relay_candidates = 0;
            while let Some(candidate) = arx.recv().await {
                let Some(remote) = remote.upgrade() else {
                    break;
                };
                if !compact {
                    if let Some((candidate, _)) = candidate {
                        remote.add_ice_candidate(candidate).await.unwrap();
                    }
                    continue;
                }
                let gathering_finished = candidate.is_none();
                if let Some((candidate, relay)) = candidate {
                    relay_candidates += usize::from(relay);
                    host_gate
                        .push(
                            "relay-readiness",
                            crate::signaling::Candidate::from_sdp_line(&format!(
                                "a={}",
                                candidate.candidate
                            ))
                            .unwrap(),
                        )
                        .unwrap();
                    // The first real relay releases the host's readiness barrier,
                    // before the end-of-gathering event. Later addresses trickle.
                    if !local_sync.queued() {
                        assert!(host_gate.pop_ready().is_none());
                    }
                }
                if let Some(marker) =
                    local_sync.relay_ready("relay-readiness", true, relay_candidates)
                {
                    assert!(
                        !gathering_finished,
                        "readiness must precede gathering completion"
                    );
                    assert_eq!(marker["sync"], true);
                    host_gate.sync("relay-readiness").unwrap();
                }
                while let Some(candidate) = host_gate.pop_ready() {
                    remote.add_ice_candidate(candidate).await.unwrap();
                }
            }
        });
        let remote = Arc::downgrade(&a);
        let forward_b = tokio::spawn(async move {
            let mut gate = crate::signaling::CandidateGate::new("relay-no-sync").unwrap();
            gate.allow_relay_trickle();
            gate.remote_ready(
                "relay-no-sync",
                &description.mid,
                &description.credentials.ufrag,
            )
            .unwrap();
            while let Some(candidate) = brx.recv().await {
                let Some(remote) = remote.upgrade() else {
                    break;
                };
                if compact {
                    let candidate = crate::signaling::Candidate::from_sdp_line(&format!(
                        "a={}",
                        candidate.candidate
                    ))
                    .unwrap();
                    gate.push("relay-no-sync", candidate).unwrap();
                    // No Parsec sync call: exercise the production gate that
                    // previously withheld every host address indefinitely.
                    while let Some(candidate) = gate.pop_ready() {
                        remote.add_ice_candidate(candidate).await.unwrap();
                    }
                } else {
                    remote.add_ice_candidate(candidate).await.unwrap();
                }
            }
        });
        while channels.iter().any(|(a, b)| {
            a.ready_state() != RTCDataChannelState::Open
                || b.ready_state() != RTCDataChannelState::Open
        }) {
            sleep(Duration::from_millis(10)).await;
        }
        if full_discovery {
            assert_eq!(
                discovered.load(Ordering::SeqCst) & 7,
                7,
                "real host, srflx and relay discovery must all be signaled"
            );
        }
        // SCTP fragments a realistic video-sized message across TURN frames.
        for (id, (a, b)) in channels.iter().enumerate() {
            // The upstream SDP advertises the RFC 8841 default 64 KiB limit.
            let payload = bytes::Bytes::from(vec![id as u8; 65536]);
            a.send(&payload).await.unwrap();
            let (side, got_id, got) = rx.recv().await.unwrap();
            assert_eq!((side, got_id), (1, id as u16));
            assert_eq!(got, payload);
            b.send(&payload).await.unwrap();
            let (side, got_id, got) = rx.recv().await.unwrap();
            assert_eq!((side, got_id), (0, id as u16));
            assert_eq!(got, payload);
        }
        let pair = a
            .sctp()
            .transport()
            .ice_transport()
            .get_selected_candidate_pair()
            .await
            .unwrap();
        assert_eq!(pair.local().typ.to_string(), "relay");
        let stats = a.get_stats().await;
        let id = format!("{}-{}", pair.local().stats_id, pair.remote().stats_id);
        let mut measured_rtt = false;
        let mut allocation = false;
        for value in stats.reports.values() {
            match value {
                webrtc::stats::StatsReportType::CandidatePair(p) if p.id == id => {
                    assert!(p.responses_received > 0);
                    assert!(p.current_round_trip_time > 0.0);
                    assert!(p.total_round_trip_time >= p.current_round_trip_time);
                    measured_rtt = true;
                }
                webrtc::stats::StatsReportType::LocalCandidate(c)
                    if c.id == pair.local().stats_id =>
                {
                    assert_eq!(c.url, url);
                    assert_eq!(
                        c.relay_protocol,
                        if url.starts_with("turns:") {
                            "tls"
                        } else if url.ends_with("tcp") {
                            "tcp"
                        } else {
                            "udp"
                        }
                    );
                    assert!(!c.url.contains("fixture"));
                    allocation = true;
                }
                _ => {}
            }
        }
        assert!(
            measured_rtt && allocation,
            "selected TURN telemetry must be present"
        );
        forward_a.abort();
        forward_b.abort();
        timeout(Duration::from_secs(5), a.close())
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(5), b.close())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn relay_only_first_relay_sync_releases_host_gate_and_connects_sctp() {
        let url = "turn:localhost:43478?transport=udp";
        timeout(
            Duration::from_secs(40),
            exchange_with_compact_candidates(&[url], url, true, true, false),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn full_stun_discovery_preserves_signaling_but_selects_only_relay() {
        let url = "turn:localhost:43478?transport=udp";
        timeout(
            Duration::from_secs(40),
            exchange_with_compact_candidates(&[url], url, true, true, true),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn relay_connectivity_never_falls_back_to_available_direct_path() {
        for force in [false, true] {
            let mut settings = SettingEngine::default();
            settings.set_network_types(vec![NetworkType::Udp4]);
            settings.set_data_channel_only(true);
            let host_api = APIBuilder::new()
                .with_setting_engine(settings.clone())
                .build();
            settings.set_relay_only_connectivity(force);
            let api = APIBuilder::new().with_setting_engine(settings).build();
            let a = api
                .new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap();
            let b = host_api
                .new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap();
            let config = Some(RTCDataChannelInit {
                negotiated: Some(0),
                ..Default::default()
            });
            let ca = a
                .create_data_channel("fixture", config.clone())
                .await
                .unwrap();
            let cb = b.create_data_channel("fixture", config).await.unwrap();
            let mut done = a.gathering_complete_promise().await;
            a.set_local_description(a.create_offer(None).await.unwrap())
                .await
                .unwrap();
            done.recv().await;
            let offer = a.local_description().await.unwrap();
            assert!(offer.sdp.contains(" typ host"));
            b.set_remote_description(offer).await.unwrap();
            let mut done = b.gathering_complete_promise().await;
            b.set_local_description(b.create_answer(None).await.unwrap())
                .await
                .unwrap();
            done.recv().await;
            a.set_remote_description(b.local_description().await.unwrap())
                .await
                .unwrap();
            if force {
                sleep(Duration::from_secs(2)).await;
                assert_ne!(ca.ready_state(), RTCDataChannelState::Open);
                assert_ne!(cb.ready_state(), RTCDataChannelState::Open);
                assert!(a
                    .sctp()
                    .transport()
                    .ice_transport()
                    .get_selected_candidate_pair()
                    .await
                    .is_none());
                assert!(!a
                    .get_stats()
                    .await
                    .reports
                    .values()
                    .any(|v| matches!(v, webrtc::stats::StatsReportType::CandidatePair(_))));
            } else {
                timeout(Duration::from_secs(5), async {
                    while ca.ready_state() != RTCDataChannelState::Open
                        || cb.ready_state() != RTCDataChannelState::Open
                    {
                        sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("control: the same host-only path must actually work");
            }
            timeout(Duration::from_secs(5), a.close())
                .await
                .unwrap()
                .unwrap();
            timeout(Duration::from_secs(5), b.close())
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn coturn_udp_tcp_and_tls_carry_real_dtls_sctp_data() {
        for url in [
            "turn:localhost:43478?transport=udp",
            "turn:localhost:43478?transport=tcp",
            "turns:localhost:43549?transport=tcp",
        ] {
            eprintln!("Verifying {url}");
            timeout(Duration::from_secs(40), exchange(url))
                .await
                .expect(url);
        }
    }

    #[tokio::test]
    async fn udp_then_tls_then_plain_tcp_fallback_preserves_sctp() {
        let udp = "turn:localhost:43478?transport=udp";
        let tls = "turns:localhost:43549?transport=tcp";
        let tcp = "turn:localhost:43478?transport=tcp";
        let unavailable_udp = "turn:127.0.0.1:43479?transport=udp";
        let unavailable_tls = "turns:localhost:43550?transport=tcp";
        for (urls, expected) in [
            (vec![tcp, tls, udp], udp),
            (vec![tcp, unavailable_udp, tls], tls),
            (vec![tcp, unavailable_udp, unavailable_tls], tcp),
        ] {
            timeout(
                Duration::from_secs(25),
                exchange_urls(&urls, expected, true),
            )
            .await
            .expect(expected);
        }
    }

    #[tokio::test]
    async fn trusted_tls_certificate_with_wrong_hostname_is_rejected() {
        use webrtc::ice::turn_stream::TurnStream;
        // Fixture CA is trusted; its server certificate has only DNS:localhost.
        assert!(TurnStream::dial("127.0.0.1", 43549, true).await.is_err());
    }
}
