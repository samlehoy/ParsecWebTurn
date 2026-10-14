//! Real UDP connectivity gates. Two native clients are connectivity peers,
//! not a stock host/media fixture: this proves the adapter's route and lifecycle
//! only. The separately configured coturn test must not use real credentials.
use lowlat_client::config::Relay;
use lowlat_client::{Event, Peer};
use lowlat_core::conn::Kind;
use parsec_native_experimental::bud::{Route, Session};
use std::time::{Duration, Instant};

fn peer(ours: lowlat_crypto::Credentials) -> Peer {
    Peer {
        ufrag: ours.ufrag,
        pwd: ours.pwd,
        fingerprint: ours.fingerprint,
        aes256: Some(ours.aes256),
    }
}

fn connect(route: Route, relay_expected: bool) {
    let mut left = Session::transport_probe().unwrap();
    let mut right = Session::transport_probe().unwrap();
    let left_ours = left.offer("left", route).unwrap();
    let right_ours = right
        .offer("right", Route::Direct { stun: vec![] })
        .unwrap();
    left.answer("left", peer(right_ours)).unwrap();
    right.answer("right", peer(left_ours)).unwrap();
    let began = Instant::now();
    let mut connected = [false; 2];
    let mut left_candidates = 0;
    while !connected.iter().all(|x| *x) && began.elapsed() < Duration::from_secs(10) {
        for (side, established) in connected.iter_mut().enumerate() {
            let (source, target, target_id) = if side == 0 {
                (&mut left, &mut right, "right")
            } else {
                (&mut right, &mut left, "left")
            };
            while let Some(event) = source.poll().unwrap() {
                match event {
                    Event::Candidate {
                        addr, from_stun, ..
                    } => {
                        if side == 0 {
                            left_candidates += 1;
                            if relay_expected {
                                assert!(from_stun, "relay leaked a direct candidate");
                            }
                        }
                        target
                            .candidate(
                                target_id,
                                addr,
                                if from_stun {
                                    Kind::Reflexive
                                } else {
                                    Kind::Direct
                                },
                            )
                            .unwrap();
                    }
                    Event::Ready => target.ready(target_id).unwrap(),
                    Event::Established { .. } => *established = true,
                    Event::Ended { outcome } => panic!("BUD path ended: {outcome:?}"),
                    _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(left_candidates > 0, "no local candidates");
    if relay_expected {
        assert_eq!(left_candidates, 1, "expected one explicit UDP allocation");
    }
    assert_eq!(
        connected,
        [true, true],
        "native UDP connectivity did not establish"
    );
    left.close();
    right.close();
}

#[test]
fn direct_udp_connectivity() {
    connect(Route::Direct { stun: vec![] }, false);
}

#[test]
#[ignore = "requires isolated coturn on loopback, PWT_TEST_TURN endpoint"]
fn turn_udp_connectivity() {
    let server = std::env::var("PWT_TEST_TURN")
        .expect("isolated TURN endpoint")
        .parse()
        .unwrap();
    connect(
        Route::TurnUdp(Relay::new(server, "pwt-fixture", "isolated-fixture-only")),
        true,
    );
}
