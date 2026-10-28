//! The media socket through the kernel's own interface on macOS.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use core::time::Duration;
use std::io;
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use crate::socket::{DEFAULT_TTL, DSCP_EF, WANT_RCVBUF, WANT_SNDBUF};

pub(super) fn socklen(bytes: usize) -> libc::socklen_t {
    libc::socklen_t::try_from(bytes).unwrap_or(libc::socklen_t::MAX)
}

fn family_v6() -> libc::sa_family_t {
    libc::sa_family_t::try_from(libc::AF_INET6).unwrap_or(0)
}

#[derive(Debug)]
pub struct Socket {
    fd: OwnedFd,
    granted_rcvbuf: i32,
    granted_sndbuf: i32,
}

impl Socket {
    pub(crate) fn bound(port: u16) -> io::Result<Self> {
        let raw = unsafe { libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let socket = Self {
            fd,
            granted_rcvbuf: 0,
            granted_sndbuf: 0,
        };
        socket.configure()?;
        socket.bind(port)?;

        let granted_rcvbuf = socket.get_int(libc::SOL_SOCKET, libc::SO_RCVBUF)?;
        let granted_sndbuf = socket.get_int(libc::SOL_SOCKET, libc::SO_SNDBUF)?;
        Ok(Self {
            granted_rcvbuf,
            granted_sndbuf,
            ..socket
        })
    }

    fn configure(&self) -> io::Result<()> {
        self.set_int(libc::IPPROTO_IPV6, libc::IPV6_V6ONLY, 0)?;
        self.set_int(libc::SOL_SOCKET, libc::SO_RCVBUF, WANT_RCVBUF)?;
        self.set_int(libc::SOL_SOCKET, libc::SO_SNDBUF, WANT_SNDBUF)?;

        let _ = self.set_int(libc::IPPROTO_IPV6, libc::IPV6_RECVPKTINFO, 1);
        let _ = self.set_int(libc::IPPROTO_IP, libc::IP_RECVDSTADDR, 1);

        let _ = self.set_int(libc::IPPROTO_IP, libc::IP_TOS, DSCP_EF);
        let _ = self.set_int(libc::IPPROTO_IPV6, libc::IPV6_TCLASS, DSCP_EF);

        let _ = self.set_int(libc::IPPROTO_IP, libc::IP_DONTFRAG, 1);

        self.set_ttl(DEFAULT_TTL)?;
        self.set_nonblocking()?;
        Ok(())
    }

    fn bind(&self, port: u16) -> io::Result<()> {
        let addr = libc::sockaddr_in6 {
            sin6_len: mem::size_of::<libc::sockaddr_in6>() as u8,
            sin6_family: family_v6(),
            sin6_port: port.to_be(),
            sin6_flowinfo: 0,
            sin6_addr: libc::in6_addr { s6_addr: [0u8; 16] },
            sin6_scope_id: 0,
        };
        let rc = unsafe {
            libc::bind(
                self.fd.as_raw_fd(),
                core::ptr::addr_of!(addr).cast(),
                socklen(mem::size_of::<libc::sockaddr_in6>()),
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn granted_recv_buffer(&self) -> i32 {
        self.granted_rcvbuf
    }

    pub fn granted_send_buffer(&self) -> i32 {
        self.granted_sndbuf
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
        let mut len = socklen(mem::size_of::<libc::sockaddr_storage>());
        let rc = unsafe {
            libc::getsockname(
                self.fd.as_raw_fd(),
                core::ptr::addr_of_mut!(storage).cast(),
                &mut len,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        from_storage(&storage).ok_or_else(|| io::Error::other("unrecognised local address"))
    }

    pub fn set_ttl(&self, ttl: u8) -> io::Result<()> {
        let _ = self.set_int(libc::IPPROTO_IP, libc::IP_TTL, i32::from(ttl));
        self.set_int(libc::IPPROTO_IPV6, libc::IPV6_UNICAST_HOPS, i32::from(ttl))
    }

    pub fn ttl(&self) -> io::Result<u8> {
        let value = self.get_int(libc::IPPROTO_IPV6, libc::IPV6_UNICAST_HOPS)
            .or_else(|_| self.get_int(libc::IPPROTO_IP, libc::IP_TTL))?;
        u8::try_from(value).map_err(|_| io::Error::other("ttl outside the byte range"))
    }

    fn set_nonblocking(&self) -> io::Result<()> {
        let flags = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let rc = unsafe {
            libc::fcntl(self.fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK)
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        let _ = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
        Ok(())
    }

    fn set_int(&self, level: libc::c_int, name: libc::c_int, value: libc::c_int) -> io::Result<()> {
        let rc = unsafe {
            libc::setsockopt(
                self.fd.as_raw_fd(),
                level,
                name,
                core::ptr::addr_of!(value).cast(),
                socklen(mem::size_of::<libc::c_int>()),
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn get_int(&self, level: libc::c_int, name: libc::c_int) -> io::Result<i32> {
        let mut value: libc::c_int = 0;
        let mut len = socklen(mem::size_of::<libc::c_int>());
        let rc = unsafe {
            libc::getsockopt(
                self.fd.as_raw_fd(),
                level,
                name,
                core::ptr::addr_of_mut!(value).cast(),
                &mut len,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(value)
    }

    pub fn send_to(&self, datagram: &[u8], to: SocketAddr) -> io::Result<usize> {
        let (addr, len) = to_storage(to);
        let sent = unsafe {
            libc::sendto(
                self.fd.as_raw_fd(),
                datagram.as_ptr().cast(),
                datagram.len(),
                0,
                core::ptr::addr_of!(addr).cast(),
                len,
            )
        };
        if sent < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(usize::try_from(sent).unwrap_or(0))
    }

    pub fn recv_from(
        &self,
        buf: &mut [u8],
        wait: Duration,
    ) -> io::Result<Option<(usize, SocketAddr)>> {
        let mut ready = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = libc::c_int::try_from(wait.as_millis()).unwrap_or(libc::c_int::MAX);
        let rc = unsafe { libc::poll(&raw mut ready, 1, ms) };
        if rc < 0 {
            let error = io::Error::last_os_error();
            return match error.kind() {
                io::ErrorKind::Interrupted => Ok(None),
                _ => Err(error),
            };
        }
        if rc == 0 {
            return Ok(None);
        }
        let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
        let mut len = socklen(mem::size_of::<libc::sockaddr_storage>());
        let got = unsafe {
            libc::recvfrom(
                self.fd.as_raw_fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                0,
                core::ptr::addr_of_mut!(storage).cast(),
                &mut len,
            )
        };
        if got < 0 {
            let error = io::Error::last_os_error();
            return match error.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => Ok(None),
                _ => Err(error),
            };
        }
        let from = from_storage(&storage)
            .ok_or_else(|| io::Error::other("unrecognised source address"))?;
        Ok(Some((usize::try_from(got).unwrap_or(0), from)))
    }

    pub(super) fn raw(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

pub(super) fn to_storage(addr: SocketAddr) -> (libc::sockaddr_in6, libc::socklen_t) {
    let (ip, port) = match addr {
        SocketAddr::V4(v4) => (v4.ip().to_ipv6_mapped(), v4.port()),
        SocketAddr::V6(v6) => (*v6.ip(), v6.port()),
    };
    let storage = libc::sockaddr_in6 {
        sin6_len: mem::size_of::<libc::sockaddr_in6>() as u8,
        sin6_family: family_v6(),
        sin6_port: port.to_be(),
        sin6_flowinfo: 0,
        sin6_addr: libc::in6_addr {
            s6_addr: ip.octets(),
        },
        sin6_scope_id: 0,
    };
    (storage, socklen(mem::size_of::<libc::sockaddr_in6>()))
}

pub(super) fn from_storage(storage: &libc::sockaddr_storage) -> Option<SocketAddr> {
    match libc::c_int::from(storage.ss_family) {
        libc::AF_INET6 => {
            let v6 = unsafe { &*core::ptr::from_ref(storage).cast::<libc::sockaddr_in6>() };
            let ip = Ipv6Addr::from(v6.sin6_addr.s6_addr);
            let port = u16::from_be(v6.sin6_port);
            Some(match ip.to_ipv4_mapped() {
                Some(v4) => SocketAddr::V4(SocketAddrV4::new(v4, port)),
                None => SocketAddr::V6(SocketAddrV6::new(
                    ip,
                    port,
                    v6.sin6_flowinfo,
                    v6.sin6_scope_id,
                )),
            })
        }
        libc::AF_INET => {
            let v4 = unsafe { &*core::ptr::from_ref(storage).cast::<libc::sockaddr_in>() };
            Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr))),
                u16::from_be(v4.sin_port),
            ))
        }
        _ => None,
    }
}
