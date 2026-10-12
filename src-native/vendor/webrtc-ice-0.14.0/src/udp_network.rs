use std::sync::{Arc, Mutex};

use super::udp_mux::UDPMux;
use super::Error;

#[derive(Default, Clone)]
pub struct EphemeralUDP {
    port_min: u16,
    port_max: u16,
    receive_buffer_size: usize,
    receive_buffers: Arc<Mutex<Vec<ReceiveBufferReport>>>,
}

/// Address-free results of optional native socket tuning. OS limits may apply.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceiveBufferReport {
    pub requested_bytes: usize,
    pub before_bytes: Option<usize>,
    pub after_bytes: Option<usize>,
    pub error: bool,
}

impl EphemeralUDP {
    /// Best-effort minimum SO_RCVBUF. Zero preserves the platform default.
    /// This is capacity for bursts, not a playout delay or a batching threshold.
    pub fn set_receive_buffer_size(&mut self, bytes: usize) {
        self.receive_buffer_size = bytes;
    }

    pub fn receive_buffer_reports(&self) -> Vec<ReceiveBufferReport> {
        self.receive_buffers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Apply optional native UDP tuning before a connection starts receiving.
    /// Unsupported/virtual connections and failed socket options are nonfatal.
    pub fn configure_socket(&self, conn: &(dyn util::Conn + Send + Sync)) {
        if self.receive_buffer_size == 0 {
            return;
        }
        // Virtual networks and stream TURN connections are deliberately untouched.
        let Some(socket) = conn.as_any().downcast_ref::<tokio::net::UdpSocket>() else {
            return;
        };
        let socket = socket2::SockRef::from(socket);
        let before = socket.recv_buffer_size();
        let mut error = before.is_err();
        if let Ok(current) = &before {
            if *current < self.receive_buffer_size {
                error |= socket
                    .set_recv_buffer_size(self.receive_buffer_size)
                    .is_err();
            }
        }
        let after = socket.recv_buffer_size();
        error |= after.is_err();
        self.receive_buffers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ReceiveBufferReport {
                requested_bytes: self.receive_buffer_size,
                before_bytes: before.ok(),
                after_bytes: after.ok(),
                error,
            });
    }

    pub fn new(port_min: u16, port_max: u16) -> Result<Self, Error> {
        let mut s = Self::default();
        s.set_ports(port_min, port_max)?;

        Ok(s)
    }

    pub fn port_min(&self) -> u16 {
        self.port_min
    }

    pub fn port_max(&self) -> u16 {
        self.port_max
    }

    pub fn set_ports(&mut self, port_min: u16, port_max: u16) -> Result<(), Error> {
        if port_max < port_min {
            return Err(Error::ErrPort);
        }

        self.port_min = port_min;
        self.port_max = port_max;

        Ok(())
    }
}

/// Configuration for the underlying UDP network stack.
/// There are two ways to configure this Ephemeral and Muxed.
///
/// **Ephemeral mode**
///
/// In Ephemeral mode sockets are created and bound to random ports during ICE
/// gathering. The ports to use can be restricted by setting [`EphemeralUDP::port_min`] and
/// [`EphemeralUDP::port_max`] in which case only ports in this range will be used.
///
/// **Muxed**
///
/// In muxed mode a single UDP socket is used and all connections are muxed over this single socket.
///
#[derive(Clone)]
pub enum UDPNetwork {
    Ephemeral(EphemeralUDP),
    Muxed(Arc<dyn UDPMux + Send + Sync>),
}

impl Default for UDPNetwork {
    fn default() -> Self {
        Self::Ephemeral(Default::default())
    }
}

impl UDPNetwork {
    fn is_ephemeral(&self) -> bool {
        matches!(self, Self::Ephemeral(_))
    }

    fn is_muxed(&self) -> bool {
        matches!(self, Self::Muxed(_))
    }
}

#[cfg(test)]
mod test {
    use super::EphemeralUDP;

    #[test]
    fn test_ephemeral_udp_constructor() {
        assert!(
            EphemeralUDP::new(3000, 2999).is_err(),
            "EphemeralUDP should not allow invalid port range"
        );

        let e = EphemeralUDP::default();
        assert_eq!(e.port_min(), 0, "EphemeralUDP should default port_min to 0");
        assert_eq!(e.port_max(), 0, "EphemeralUDP should default port_max to 0");
    }

    #[test]
    fn test_ephemeral_udp_set_ports() {
        let mut e = EphemeralUDP::default();

        assert!(
            e.set_ports(3000, 2999).is_err(),
            "EphemeralUDP should not allow invalid port range"
        );

        assert!(
            e.set_ports(6000, 6001).is_ok(),
            "EphemeralUDP::set_ports should allow valid port range"
        );

        assert_eq!(
            e.port_min(),
            6000,
            "Ports set with `EphemeralUDP::set_ports` should be reflected"
        );
        assert_eq!(
            e.port_max(),
            6001,
            "Ports set with `EphemeralUDP::set_ports` should be reflected"
        );
    }
}
