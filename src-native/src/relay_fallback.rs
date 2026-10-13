//! A connection-scoped TURN plan. No settings/cache mutation and no stream retry.
use anyhow::{bail, Context, Result};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use webrtc::{
    ice::url::{ProtoType, SchemeType, Url},
    ice_transport::ice_server::RTCIceServer,
    peer_connection::{
        configuration::RTCConfiguration, policy::ice_transport_policy::RTCIceTransportPolicy,
    },
};

pub const CHECK_WAIT: Duration = Duration::from_secs(8);
pub const CHAIN_LIMIT: Duration = Duration::from_secs(60);

/// Start the short route check only after the host has answered. Once DTLS
/// progresses, permanently leave completion to the normal establishment timer.
pub struct CheckDeadline {
    enabled: bool,
    started: Option<Instant>,
}
impl CheckDeadline {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            started: None,
        }
    }
    pub fn expired(
        &mut self,
        now: Instant,
        remote_ready: bool,
        dtls_progress: bool,
        established: bool,
    ) -> bool {
        if dtls_progress || established {
            self.enabled = false;
        }
        if !self.enabled || !remote_ready {
            return false;
        }
        now.saturating_duration_since(*self.started.get_or_insert(now)) >= CHECK_WAIT
    }
}

#[derive(Clone, Default)]
pub struct Route {
    plan: Arc<Mutex<Option<Plan>>>,
    index: usize,
}
struct Plan {
    configurations: Vec<RTCConfiguration>,
    started: Instant,
}
impl Route {
    pub async fn resolve(
        &self,
        manager: &crate::connection_settings::Manager,
    ) -> Result<RTCConfiguration> {
        let needs_resolution = self
            .plan
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none();
        if needs_resolution {
            let config = manager.resolve_configuration().await?;
            let configurations = configurations(config)?;
            *self.plan.lock().unwrap_or_else(|e| e.into_inner()) = Some(Plan {
                configurations,
                started: Instant::now(),
            });
        }
        let plan = self.plan.lock().unwrap_or_else(|e| e.into_inner());
        let plan = plan.as_ref().context("TURN fallback plan unavailable")?;
        plan.configurations
            .get(self.index)
            .cloned()
            .context("TURN fallback exhausted")
    }
    pub fn next(&self) -> Option<Self> {
        let plan = self.plan.lock().unwrap_or_else(|e| e.into_inner());
        let plan = plan.as_ref()?;
        if plan.started.elapsed() >= CHAIN_LIMIT || self.index + 1 >= plan.configurations.len() {
            return None;
        }
        Some(Self {
            plan: self.plan.clone(),
            index: self.index + 1,
        })
    }
    pub fn budget(&self) -> Option<Duration> {
        self.plan
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|p| CHAIN_LIMIT.saturating_sub(p.started.elapsed()))
    }
}

fn configurations(config: RTCConfiguration) -> Result<Vec<RTCConfiguration>> {
    if config.ice_transport_policy != RTCIceTransportPolicy::Relay {
        return Ok(vec![config]);
    }
    let mut discovery = Vec::new();
    let mut endpoints: Vec<(u8, RTCIceServer)> = Vec::new();
    for server in &config.ice_servers {
        let mut stun = server.clone();
        stun.urls.clear();
        for raw in &server.urls {
            let url = Url::parse_url(raw).map_err(|_| anyhow::anyhow!("Invalid STUN/TURN URL"))?;
            let rank = match (url.scheme, url.proto) {
                (SchemeType::Stun, _) => {
                    stun.urls.push(raw.clone());
                    continue;
                }
                (SchemeType::Turn, ProtoType::Udp) => 0,
                (SchemeType::Turns, ProtoType::Tcp) => 1,
                (SchemeType::Turn, ProtoType::Tcp) => 2,
                _ => bail!("Unsupported STUN/TURN transport"),
            };
            let endpoint = RTCIceServer {
                urls: vec![raw.clone()],
                username: server.username.clone(),
                credential: server.credential.clone(),
            };
            if !endpoints.iter().any(|(_, e)| {
                e.urls == endpoint.urls
                    && e.username == endpoint.username
                    && e.credential == endpoint.credential
            }) {
                endpoints.push((rank, endpoint));
            }
        }
        if !stun.urls.is_empty() {
            discovery.push(stun);
        }
    }
    if endpoints.is_empty() {
        bail!("Force TURN relay requires a supported TURN server");
    }
    endpoints.sort_by_key(|(rank, _)| *rank); // stable within each transport tier
    Ok(endpoints
        .into_iter()
        .map(|(_, endpoint)| {
            let mut selected = config.clone();
            selected.ice_servers = discovery.clone();
            selected.ice_servers.push(endpoint);
            selected
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> RTCConfiguration {
        RTCConfiguration {
            ice_transport_policy: RTCIceTransportPolicy::Relay,
            ice_servers: vec![RTCIceServer {
                urls: vec![
                    "stun:discovery.invalid:3478".into(),
                    "turn:relay.invalid:3478?transport=tcp".into(),
                    "turns:relay.invalid:443".into(),
                    "turn:relay.invalid:3478".into(),
                    "turn:relay.invalid:443?transport=udp".into(),
                    "turn:relay.invalid:3478".into(),
                ],
                username: "fixture".into(),
                credential: "fixture-secret".into(),
            }],
            ..Default::default()
        }
    }
    #[test]
    fn forced_plan_preserves_discovery_credentials_and_all_unique_fallbacks() {
        let original = config();
        let configs = configurations(original.clone()).unwrap();
        assert_eq!(configs.len(), 4);
        for (config, expected) in configs.iter().zip([
            "turn:relay.invalid:3478",
            "turn:relay.invalid:443?transport=udp",
            "turns:relay.invalid:443",
            "turn:relay.invalid:3478?transport=tcp",
        ]) {
            assert_eq!(config.ice_transport_policy, RTCIceTransportPolicy::Relay);
            assert_eq!(config.ice_servers.len(), 2);
            assert_eq!(config.ice_servers[0].urls, ["stun:discovery.invalid:3478"]);
            assert_eq!(config.ice_servers[1].urls, [expected]);
            assert_eq!(config.ice_servers[1].credential, "fixture-secret");
        }
        assert_eq!(original.ice_servers[0].urls.len(), 6);
        let mut ordinary = original;
        ordinary.ice_transport_policy = RTCIceTransportPolicy::All;
        let unchanged = configurations(ordinary.clone()).unwrap();
        assert_eq!(unchanged.len(), 1);
        assert_eq!(
            unchanged[0].ice_servers[0].urls,
            ordinary.ice_servers[0].urls
        );
    }
    #[test]
    fn chain_is_bounded_and_credentials_are_reused_only_within_that_chain() {
        let route = Route::default();
        assert!(route.next().is_none());
        *route.plan.lock().unwrap() = Some(Plan {
            configurations: configurations(config()).unwrap(),
            started: Instant::now(),
        });
        let second = route.next().unwrap();
        assert!(Arc::ptr_eq(&route.plan, &second.plan));
        let last = second.next().unwrap().next().unwrap();
        assert!(last.next().is_none());
        route.plan.lock().unwrap().as_mut().unwrap().started = Instant::now() - CHAIN_LIMIT;
        assert!(route.next().is_none());
        assert_eq!(route.budget(), Some(Duration::ZERO));
        let mut missing = config();
        missing.ice_servers.clear();
        assert!(configurations(missing).is_err());
    }
    #[test]
    fn short_timeout_waits_for_host_and_never_interrupts_progress_or_a_stream() {
        let now = Instant::now();
        let mut timer = CheckDeadline::new(true);
        assert!(!timer.expired(now, false, false, false));
        let answer = now + Duration::from_secs(20);
        assert!(!timer.expired(answer, true, false, false));
        assert!(!timer.expired(
            answer + CHECK_WAIT - Duration::from_millis(1),
            true,
            false,
            false
        ));
        assert!(timer.expired(answer + CHECK_WAIT, true, false, false));
        let mut progressing = CheckDeadline::new(true);
        assert!(!progressing.expired(now, true, true, false));
        assert!(!progressing.expired(now + CHAIN_LIMIT, true, false, false));
        let mut normal = CheckDeadline::new(false);
        assert!(!normal.expired(now + CHAIN_LIMIT, true, false, false));
        let mut connected = CheckDeadline::new(true);
        assert!(!connected.expired(now, true, false, true));
        assert!(!connected.expired(now + CHAIN_LIMIT, true, false, false));
    }
}
