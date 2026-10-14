//! BUD lifecycle below account signaling, separate from the web WASM contract.
use std::net::SocketAddr;

use anyhow::{Result, anyhow, ensure};
pub use lowlat_client::Event;
use lowlat_client::config::{Backend, Decoding, FrameKind, Port, Relay};
use lowlat_client::driver::Units;
use lowlat_client::{Client, Config, Peer, Transport};
use lowlat_core::conn::Kind;
use lowlat_crypto::Credentials;

/// One explicit route per attempt. No implicit transport or provider fallback.
pub enum Route {
    Direct { stun: Vec<SocketAddr> },
    TurnUdp(Relay),
}

impl Route {
    pub fn turn_udp(server: SocketAddr, username: &str, password: &str) -> Self {
        Self::TurnUdp(Relay::new(server, username, password))
    }
    fn configure(self) -> Config {
        match self {
            Self::Direct { stun } => Config {
                servers: stun,
                ..Config::default()
            },
            Self::TurnUdp(relay) => Config {
                relay: Some(relay),
                ..Config::default()
            },
        }
    }
}

/// A transport-only gate. It deliberately discards video; it is NOT a playable
/// client. This lets direct/relay/authentication be tested without accepting the
/// SDK's split-plane export as equivalent to our current NV12 renderer.
pub struct Session {
    client: Client,
    attempt: Option<String>,
    external_units: Option<Units>,
    prefer_hevc: bool,
}

impl Session {
    pub fn transport_probe() -> Result<Self> {
        let client = Client::new(
            &Decoding {
                backend: Backend::None,
                kind: FrameKind::Planes,
                ..Decoding::default()
            },
            Port::Any,
        )
        .map_err(|e| anyhow!("BUD create: {e:?}"))?;
        Ok(Self {
            client,
            attempt: None,
            external_units: None,
            prefer_hevc: false,
        })
    }

    /// Native transport with application-owned video decode. The caller must
    /// have verified its existing MF decoder's HEVC support before setting it.
    /// Only 8-bit 4:2:0 is advertised. No SDK video decoder is created.
    pub fn external_video(hevc_available: bool) -> Result<Self> {
        let client = Client::new_external_video(
            lowlat_client::config::Caps {
                h264: true,
                hevc: hevc_available,
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
        })
    }

    /// The sole compressed-video consumer. The SDK pool lease stays live
    /// throughout the callback, and is returned even if the sink rejects it.
    /// The callback must not retain a borrowed pointer after returning.
    pub fn with_video<T>(
        &mut self,
        sink: impl FnOnce(&[u8], u64) -> Result<T>,
    ) -> Result<Option<T>> {
        ensure!(self.attempt.is_some(), "no BUD attempt");
        let units = self
            .external_units
            .as_ref()
            .ok_or_else(|| anyhow!("transport-only BUD probe has no video consumer"))?;
        let Some(unit) = units.take() else {
            return Ok(None);
        };
        let arrived_us = self.client.arrived_us(unit.stamp());
        sink(unit.bytes(), arrived_us).map(Some)
    }

    pub fn request_keyframe(&self) -> bool {
        self.client.request_external_keyframe()
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

    pub fn send_input(&mut self, input: lowlat_client::input::Input) -> Result<()> {
        self.client
            .send_input(input)
            .map_err(|e| anyhow!("BUD input: {e:?}"))
    }

    pub fn set_viewport(&mut self, viewport: lowlat_client::input::Viewport) -> bool {
        self.client.set_viewport(viewport)
    }

    pub fn with_audio<T>(&mut self, sink: impl FnOnce(&[u8]) -> Result<T>) -> Result<Option<T>> {
        let Some(packet) = self
            .client
            .take_external_audio()
            .map_err(|e| anyhow!("BUD external audio: {e:?}"))?
        else {
            return Ok(None);
        };
        sink(packet.bytes()).map(Some)
    }

    pub fn offer(&mut self, id: &str, route: Route) -> Result<Credentials> {
        ensure!(!id.is_empty() && id.len() <= 256, "invalid BUD attempt id");
        ensure!(self.attempt.is_none(), "BUD attempt already active");
        let mut config = route.configure();
        config.video.hevc = self.prefer_hevc;
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
        self.matches(id)?;
        self.client
            .begin_p2p(id, &peer)
            .map_err(|e| anyhow!("BUD answer: {e:?}"))
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
        let relay =
            Route::TurnUdp(Relay::new(endpoint, "isolated", "not-a-real-secret")).configure();
        assert!(relay.servers.is_empty());
        assert_eq!(relay.relay.unwrap().server, endpoint);
    }

    #[test]
    fn native_credentials_lifecycle_and_stale_attempts() {
        let mut session = Session::transport_probe().unwrap();
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
