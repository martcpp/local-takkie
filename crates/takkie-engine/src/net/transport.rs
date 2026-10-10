//! Sending and receiving datagrams. The engine only sees [`Transport`], so
//! tests can run on an in-memory network.

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicU16, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use socket2::{Domain, Protocol, Socket, Type};

/// How long a receive waits before giving the caller a chance to stop.
pub const RECV_TIMEOUT: Duration = Duration::from_millis(100);

const BUFFER_BYTES: usize = 1 << 20;

/// A datagram socket.
pub trait Transport: Send + Sync {
    /// The address peers reach us on.
    fn local_addr(&self) -> SocketAddr;

    /// Sends one datagram.
    ///
    /// # Errors
    /// The OS error, if sending failed.
    fn send_to(&self, packet: &[u8], to: SocketAddr) -> io::Result<()>;

    /// Waits up to [`RECV_TIMEOUT`] for a datagram; `None` if none came.
    ///
    /// # Errors
    /// The OS error, if receiving failed.
    fn recv_from(&self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>>;
}

/// UDP on IPv4.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
    local: SocketAddr,
}

impl UdpTransport {
    /// Binds `0.0.0.0:port`; port 0 lets the OS pick a free one.
    ///
    /// # Errors
    /// The OS error, for example when the port is taken.
    pub fn bind(port: u16) -> io::Result<Self> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        // Bigger buffers ride out bursts; the OS may cap them, which is fine.
        let _ = socket.set_recv_buffer_size(BUFFER_BYTES);
        let _ = socket.set_send_buffer_size(BUFFER_BYTES);
        socket.bind(&SocketAddr::from(([0, 0, 0, 0], port)).into())?;
        let socket: UdpSocket = socket.into();
        socket.set_read_timeout(Some(RECV_TIMEOUT))?;
        let local = socket.local_addr()?;
        Ok(Self { socket, local })
    }
}

impl Transport for UdpTransport {
    fn local_addr(&self) -> SocketAddr {
        self.local
    }

    fn send_to(&self, packet: &[u8], to: SocketAddr) -> io::Result<()> {
        self.socket.send_to(packet, to).map(|_| ())
    }

    fn recv_from(&self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
        match self.socket.recv_from(buf) {
            Ok(received) => Ok(Some(received)),
            // Windows reports an earlier send to a closed port as a reset on
            // the next receive; for UDP that just means nothing arrived.
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ConnectionReset
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}

type Inbox = Sender<(Vec<u8>, SocketAddr)>;

/// An in-memory network for tests: transports bound on it reach each other
/// by address, and datagrams to unknown addresses vanish, like UDP.
#[derive(Clone, Debug, Default)]
pub struct MemoryNetwork {
    inboxes: Arc<Mutex<HashMap<SocketAddr, Inbox>>>,
    next_port: Arc<AtomicU16>,
}

impl MemoryNetwork {
    /// An empty network.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inboxes: Arc::default(),
            next_port: Arc::new(AtomicU16::new(40_000)),
        }
    }

    /// A transport on `127.0.0.1:port`; port 0 picks a free one.
    #[must_use]
    pub fn bind(&self, port: u16) -> MemoryTransport {
        let port = if port == 0 {
            self.next_port.fetch_add(1, Relaxed)
        } else {
            port
        };
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let (sender, inbox) = unbounded();
        if let Ok(mut inboxes) = self.inboxes.lock() {
            inboxes.insert(addr, sender);
        }
        MemoryTransport {
            addr,
            inbox,
            network: self.clone(),
        }
    }
}

/// One endpoint on a [`MemoryNetwork`].
#[derive(Debug)]
pub struct MemoryTransport {
    addr: SocketAddr,
    inbox: Receiver<(Vec<u8>, SocketAddr)>,
    network: MemoryNetwork,
}

impl Transport for MemoryTransport {
    fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    fn send_to(&self, packet: &[u8], to: SocketAddr) -> io::Result<()> {
        let inboxes = self
            .network
            .inboxes
            .lock()
            .map_err(|_| io::Error::other("memory network poisoned"))?;
        if let Some(inbox) = inboxes.get(&to) {
            let _ = inbox.send((packet.to_vec(), self.addr));
        }
        Ok(())
    }

    fn recv_from(&self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
        let Ok((packet, from)) = self.inbox.recv_timeout(RECV_TIMEOUT) else {
            return Ok(None);
        };
        let len = packet.len().min(buf.len());
        buf[..len].copy_from_slice(&packet[..len]);
        Ok(Some((len, from)))
    }
}

impl Drop for MemoryTransport {
    fn drop(&mut self) {
        if let Ok(mut inboxes) = self.network.inboxes.lock() {
            inboxes.remove(&self.addr);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn loopback(transport: &impl Transport) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], transport.local_addr().port()))
    }

    fn exchange(a: &impl Transport, b: &impl Transport) {
        a.send_to(b"hello", loopback(b)).unwrap();
        let mut buf = [0; 64];
        let (len, from) = b.recv_from(&mut buf).unwrap().unwrap();
        assert_eq!(&buf[..len], b"hello");
        assert_eq!(from.port(), a.local_addr().port());
    }

    fn waits_then_gives_up(transport: &impl Transport) {
        let start = Instant::now();
        assert!(transport.recv_from(&mut [0; 64]).unwrap().is_none());
        assert!(start.elapsed() >= RECV_TIMEOUT - Duration::from_millis(20));
    }

    #[test]
    fn udp_sends_and_receives_on_loopback() {
        let a = UdpTransport::bind(0).unwrap();
        let b = UdpTransport::bind(0).unwrap();
        assert_ne!(a.local_addr().port(), 0);
        exchange(&a, &b);
    }

    #[test]
    fn udp_receive_blocks_until_the_timeout_when_idle() {
        waits_then_gives_up(&UdpTransport::bind(0).unwrap());
    }

    #[test]
    fn udp_survives_sending_to_a_closed_port() {
        let a = UdpTransport::bind(0).unwrap();
        let closed = {
            let gone = UdpTransport::bind(0).unwrap();
            loopback(&gone)
        };
        a.send_to(b"anyone?", closed).unwrap();
        assert!(a.recv_from(&mut [0; 64]).unwrap().is_none());
        let b = UdpTransport::bind(0).unwrap();
        exchange(&b, &a);
    }

    #[test]
    fn a_taken_port_is_an_error() {
        let a = UdpTransport::bind(0).unwrap();
        assert!(UdpTransport::bind(a.local_addr().port()).is_err());
    }

    #[test]
    fn memory_network_delivers_by_address() {
        let network = MemoryNetwork::new();
        let a = network.bind(0);
        let b = network.bind(0);
        assert_ne!(a.local_addr(), b.local_addr());
        exchange(&a, &b);
    }

    #[test]
    fn memory_network_drops_unknown_and_closed_addresses() {
        let network = MemoryNetwork::new();
        let a = network.bind(0);
        let gone = network.bind(0).local_addr();
        a.send_to(b"lost", gone).unwrap();
        a.send_to(b"lost", SocketAddr::from(([10, 0, 0, 1], 9)))
            .unwrap();
        waits_then_gives_up(&a);
    }
}
