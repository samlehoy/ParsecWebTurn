//! BUD lifecycle below account signaling, separate from the web WASM contract.
use std::net::SocketAddr;

use anyhow::{Result, anyhow, ensure};
use lowlat_client::config::{Backend, Decoding, FrameKind, Port, Relay};
use lowlat_client::{Client, Config, Event, Peer, Transport};
use lowlat_core::conn::Kind;
use lowlat_crypto::Credentials;

/// One explicit route per attempt. No implicit transport or provider fallback.
pub enum Route {
    Direct { stun: Vec<SocketAddr> },
    TurnUdp(Relay),
}

impl Route {
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
        })
    }

    pub fn offer(&mut self, id: &str, route: Route) -> Result<Credentials> {
        ensure!(!id.is_empty() && id.len() <= 256, "invalid BUD attempt id");
        ensure!(self.attempt.is_none(), "BUD attempt already active");
        let ours = self
            .client
            .new_attempt(id, route.configure(), Transport::Bud)
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
