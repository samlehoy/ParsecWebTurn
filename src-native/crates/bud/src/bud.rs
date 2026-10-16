//! BUD lifecycle below account signaling, separate from the web WASM contract.
use std::net::SocketAddr;

use anyhow::{Result, anyhow, ensure};
pub use lowlat_client::config::RelayTransport;
use lowlat_client::config::{Port, Relay};
use lowlat_client::driver::Units;
use lowlat_client::{Client, Config, Peer, Transport};
pub use lowlat_client::{Event, Outcome};
use lowlat_core::conn::Kind;
use lowlat_crypto::Credentials;

/// One explicit route per attempt. No implicit transport or provider fallback.
pub enum Route {
    Direct { stun: Vec<SocketAddr> },
    Turn(Relay),
}

impl Route {
    pub fn turn_udp(server: SocketAddr, username: &str, password: &str) -> Self {
        Self::turn(server, username, password, RelayTransport::Udp)
    }
    pub fn turn(
        server: SocketAddr,
        username: &str,
        password: &str,
        transport: RelayTransport,
    ) -> Self {
        let mut relay = Relay::new(server, username, password);
        relay.transport = transport;
        Self::Turn(relay)
    }
    fn configure(self) -> Config {
        match self {
            Self::Direct { stun } => Config {
                servers: stun,
                ..Config::default()
            },
            Self::Turn(relay) => Config {
                relay: Some(relay),
                ..Config::default()
            },
        }
    }
}

/// Application-owned native session, with a separate compressed-media consumer.
pub struct Session {
    client: Client,
    attempt: Option<String>,
    external_units: Option<Units>,
    prefer_hevc: bool,
    ten_bit: bool,
    raw_audio: bool,
    cipher: Option<&'static str>,
}

impl Session {
    /// Native transport with application-owned video decode. The caller must
    /// have verified its existing MF decoder's HEVC support before setting it.
    /// Only 8-bit 4:2:0 is advertised. No SDK video decoder is created.
    pub fn external_video(hevc_available: bool) -> Result<Self> {
        Self::external_video_depth(hevc_available, false)
    }

    pub fn external_video_depth(hevc_available: bool, ten_bit: bool) -> Result<Self> {
        let client = Client::new_external_video(
            lowlat_client::config::Caps {
                h264: true,
                hevc: hevc_available,
                hevc_10: hevc_available && ten_bit,
                ..Default::default()
            },
            Port::Any,
        )
        .map_err(|e| anyhow!("BUD external video: {e:?}"))?;
        let units = client.units();
        Ok(Self {
            client,
            attempt: None,
            external_units: Some(units),
            prefer_hevc: hevc_available,
            ten_bit: hevc_available && ten_bit,
            raw_audio: false,
            cipher: None,
        })
    }

    pub fn request_keyframe(&self) -> bool {
        self.client.request_external_keyframe()
    }
    pub fn allow_raw_audio(&mut self, enabled: bool) {
        self.raw_audio = enabled;
    }

    pub fn take_media(&mut self) -> Result<MediaSource> {
        let units = self
            .external_units
            .take()
            .ok_or_else(|| anyhow!("BUD video consumer already taken"))?;
        let audio = self
            .client
            .take_external_audio_source()
            .map_err(|_| anyhow!("BUD audio consumer already taken"))?;
        Ok(MediaSource { units, audio })
    }

    pub fn wake_consumer(&mut self, thread: std::thread::Thread) {
        self.client.wake_external_consumer(thread);
    }

    pub fn send_control(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(
            self.client.send_external_control(bytes),
            "BUD control refused"
        );
        Ok(())
    }

    pub fn send_user_data(&mut self, id: u32, text: &[u8]) -> bool {
        self.client.send_user_data(id, text)
    }

    pub fn offer(&mut self, id: &str, route: Route) -> Result<Credentials> {
        ensure!(!id.is_empty() && id.len() <= 256, "invalid BUD attempt id");
        ensure!(self.attempt.is_none(), "BUD attempt already active");
        let mut config = route.configure();
        config.video.hevc = self.prefer_hevc;
        config.video.ten_bit = self.ten_bit;
        config.video.full_range = true;
        config.raw_audio = self.raw_audio;
        let ours = self
            .client
            .new_attempt(id, config, Transport::Bud)
            .map_err(|e| anyhow!("BUD offer: {e:?}"))?;
        self.attempt = Some(id.to_owned());
        Ok(ours)
    }

    fn matches(&self, id: &str) -> Result<()> {
        ensure!(self.attempt.as_deref() == Some(id), "stale BUD attempt");
        Ok(())
    }

    pub fn answer(&mut self, id: &str, peer: Peer) -> Result<()> {
        let cipher = if peer.aes256.as_ref().is_some_and(|v| !v.is_empty()) {
            "AES-256-GCM"
        } else {
            "AES-128-GCM"
        };
        self.matches(id)?;
        self.client
            .begin_p2p(id, &peer)
            .map_err(|e| anyhow!("BUD answer: {e:?}"))?;
        self.cipher = Some(cipher);
        Ok(())
    }

    pub fn candidate(&mut self, id: &str, addr: SocketAddr, kind: Kind) -> Result<()> {
        self.matches(id)?;
        ensure!(
            addr.port() != 0 && !addr.ip().is_unspecified(),
            "invalid BUD candidate"
        );
        self.client.add_candidate(id, addr, false, kind);
        Ok(())
    }

    pub fn ready(&mut self, id: &str) -> Result<()> {
        self.matches(id)?;
        self.client
            .add_candidate(id, "0.0.0.0:0".parse()?, true, Kind::Direct);
        Ok(())
    }

    pub fn poll(&mut self) -> Result<Option<Event>> {
        let Some(received) = self.client.poll_event() else {
            return Ok(None);
        };
        ensure!(
            received.dropped == 0,
            "BUD signaling events lost; end attempt"
        );
        Ok(Some(received.event))
    }

    /// Per-channel counters describe delivered messages. UDP totals count
    /// socket payload bytes, including protocol/relay overhead, not IP headers.
    pub fn snapshot(&self) -> Snapshot {
        use std::sync::atomic::Ordering::Relaxed;
        let t = self.client.telemetry();
        let connected = t.state.load(Relaxed) == 1;
        let rtt = t.rtt_ms.load(Relaxed);
        let encode = t.encode_us.load(Relaxed);
        Snapshot {
            udp_received: t.udp_received.load(Relaxed),
            udp_sent: t.udp_sent.load(Relaxed),
            connected,
            relay: connected && t.path_relayed.load(Relaxed),
            relay_transport: (connected && t.path_relayed.load(Relaxed)).then(|| {
                match t.relay_transport.load(Relaxed) {
                    1 => "tcp",
                    2 => "tls",
                    _ => "udp",
                }
            }),
            cipher: connected.then_some(self.cipher).flatten(),
            rtt_ms: (connected && rtt > 0).then_some(f64::from(rtt)),
            encode_ms: (connected && encode > 0).then_some(f64::from(encode) / 1000.0),
            channels: std::array::from_fn(|i| Channel {
                fragments: t.fragments[i].load(Relaxed),
                late: t.late[i].load(Relaxed),
                duplicates: t.duplicates[i].load(Relaxed),
                window_drops: t.out_of_window[i].load(Relaxed),
                nacks: t.nacks_sent[i].load(Relaxed),
                bytes: t.bytes[i].load(Relaxed),
            }),
            video_backlog: t.behind.load(Relaxed),
            video_backlog_ms: t.behind_ms.load(Relaxed),
            skipped_video: t.skipped.load(Relaxed),
            audio_pool_drops: t.audio_dropped.load(Relaxed),
            input_drops: t.input_dropped.load(Relaxed),
        }
    }

    pub fn close(&mut self) {
        if let Some(id) = self.attempt.take() {
            // No GUI or WebRTC lock held while SDK joins its own worker threads.
            self.client.end_connection(&id);
            if let Some(units) = &self.external_units {
                units.clear();
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

/// Sole media consumer; cannot clone or outlive borrowed pool leases.
/// Decode/inspection runs without locking the input/session command path.
pub struct MediaSource {
    units: Units,
    audio: lowlat_client::sound::EncodedAudioSource,
}
impl MediaSource {
    pub fn with_video<T>(&mut self, sink: impl FnOnce(&[u8]) -> Result<T>) -> Result<Option<T>> {
        let Some(unit) = self.units.take() else {
            return Ok(None);
        };
        sink(unit.bytes()).map(Some)
    }
    pub fn with_audio<T>(&mut self, sink: impl FnOnce(&[u8]) -> Result<T>) -> Result<Option<T>> {
        let Some(packet) = self.audio.take() else {
            return Ok(None);
        };
        sink(packet.bytes()).map(Some)
    }
}

#[derive(Clone, Default)]
pub struct Channel {
    pub fragments: u64,
    pub late: u64,
    pub duplicates: u64,
    pub window_drops: u64,
    pub nacks: u64,
    pub bytes: u64,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub udp_received: u64,
    pub udp_sent: u64,
    pub connected: bool,
    pub relay: bool,
    pub relay_transport: Option<&'static str>,
    pub cipher: Option<&'static str>,
    pub rtt_ms: Option<f64>,
    pub encode_ms: Option<f64>,
    pub channels: [Channel; 3],
    pub video_backlog: u32,
    pub video_backlog_ms: u32,
    pub skipped_video: u64,
    pub audio_pool_drops: u32,
    pub input_drops: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_do_not_mix_direct_candidates_and_relay() {
        let endpoint = "127.0.0.1:3478".parse().unwrap();
        let direct = Route::Direct {
            stun: vec![endpoint],
        }
        .configure();
        assert!(direct.relay.is_none());
        assert_eq!(direct.servers, [endpoint]);
        let relay = Route::turn_udp(endpoint, "isolated", "not-a-real-secret").configure();
        assert!(relay.servers.is_empty());
        assert_eq!(relay.relay.unwrap().server, endpoint);
    }

    #[test]
    fn media_consumer_is_exclusive_and_separate_from_session_commands() {
        let mut session = Session::external_video(false).unwrap();
        session
            .offer("media-source", Route::Direct { stun: vec![] })
            .unwrap();
        let mut source = session.take_media().unwrap();
        assert!(session.take_media().is_err());
        assert!(source.with_video(|_| Ok(())).unwrap().is_none());
        assert!(source.with_audio(|_| Ok(())).unwrap().is_none());
        drop(source);
        session.close();
    }

    #[test]
    fn native_credentials_lifecycle_and_stale_attempts() {
        let mut session = Session::external_video(false).unwrap();
        let ours = session
            .offer("isolated-test", Route::Direct { stun: vec![] })
            .unwrap();
        assert_eq!(ours.ufrag.len(), 8);
        assert_eq!(ours.pwd.len(), 32);
        assert_eq!(ours.fingerprint.len(), 64);
        assert_eq!(ours.aes256.len(), 254);
        assert!(session.ready("stale").is_err());
        assert!(
            session
                .offer("duplicate", Route::Direct { stun: vec![] })
                .is_err()
        );
        session.close();
        assert!(session.ready("isolated-test").is_err());
        assert!(
            session
                .offer("second", Route::Direct { stun: vec![] })
                .is_ok()
        );
    }
}
