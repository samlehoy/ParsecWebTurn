//! The wait and batched receive over socket and wake on macOS.

use core::net::{IpAddr, SocketAddr};
use std::io;
use std::mem;

use super::socket::{Socket, from_storage, socklen};
use super::wake::Wake;
use crate::shell::{MIN_WAIT_MS, Ready};
use crate::socket::{RECV_BATCH, RECV_SLOT};

#[derive(Debug)]
pub(crate) struct Io {
    socket: Socket,
    wake: Wake,
    batch: Batch,
}

impl Io {
    pub(crate) fn new(socket: Socket, wake: Wake) -> Self {
        Self {
            socket,
            wake,
            batch: Batch::new(),
        }
    }

    pub(crate) fn socket(&self) -> &Socket {
        &self.socket
    }

    pub(crate) fn wake(&self) -> &Wake {
        &self.wake
    }

    pub(crate) fn wait(&mut self, timeout_ms: f64) -> io::Result<Ready> {
        let timeout = timeout_ms.max(MIN_WAIT_MS).ceil() as libc::c_int;
        let mut fds = [
            libc::pollfd {
                fd: self.socket.raw(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.wake.raw(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
        if rc <= 0 {
            if rc < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
            return Ok(Ready::default());
        }
        let [socket, wake] = fds;
        Ok(Ready {
            socket: socket.revents != 0,
            wake: wake.revents != 0,
        })
    }

    pub(crate) fn take_wake(&mut self) -> io::Result<bool> {
        self.wake.take()
    }

    pub(crate) fn drain(&mut self) -> io::Result<usize> {
        self.batch.drain(&self.socket)
    }

    pub(crate) fn saturated(&self) -> bool {
        self.batch.filled == RECV_BATCH
    }

    pub(crate) fn mark(&mut self, _to: SocketAddr) {}

    pub(crate) fn iter(&self) -> impl Iterator<Item = (SocketAddr, Option<IpAddr>, &[u8])> {
        self.batch.iter()
    }
}

#[derive(Debug)]
struct Batch {
    slots: Box<[[u8; RECV_SLOT]]>,
    names: Box<[libc::sockaddr_storage]>,
    lengths: Box<[usize]>,
    filled: usize,
}

impl Batch {
    fn new() -> Self {
        let slots = vec![[0u8; RECV_SLOT]; RECV_BATCH].into_boxed_slice();
        let names =
            vec![unsafe { mem::zeroed::<libc::sockaddr_storage>() }; RECV_BATCH].into_boxed_slice();
        let lengths = vec![0usize; RECV_BATCH].into_boxed_slice();
        Self {
            slots,
            names,
            lengths,
            filled: 0,
        }
    }

    fn drain(&mut self, socket: &Socket) -> io::Result<usize> {
        self.filled = 0;
        let mut count = 0;
        while count < RECV_BATCH {
            let slot = &mut self.slots[count];
            let name = &mut self.names[count];
            let mut name_len = socklen(mem::size_of::<libc::sockaddr_storage>());
            let rc = unsafe {
                libc::recvfrom(
                    socket.raw(),
                    slot.as_mut_ptr().cast(),
                    slot.len(),
                    0,
                    core::ptr::addr_of_mut!(*name).cast(),
                    &mut name_len,
                )
            };
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::Interrupted {
                    break;
                }
                if count > 0 {
                    break;
                }
                return Err(err);
            }
            self.lengths[count] = rc as usize;
            count += 1;
        }
        self.filled = count;
        Ok(self.filled)
    }

    fn iter(&self) -> impl Iterator<Item = (SocketAddr, Option<IpAddr>, &[u8])> {
        (0..self.filled).filter_map(move |index| {
            let len = *self.lengths.get(index)?;
            let bytes = self.slots.get(index)?.get(..len)?;
            let from = from_storage(self.names.get(index)?)?;
            Some((from, None, bytes))
        })
    }
}
