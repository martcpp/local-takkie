/// mDNS announcing and browsing.
pub mod discovery;
/// The receiving thread.
pub mod rx;
/// The sending side.
pub mod send;
/// Datagram sockets.
pub mod transport;

pub use transport::{MemoryNetwork, MemoryTransport, Transport, UdpTransport};
