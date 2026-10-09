/// Choosing a peer's address.
pub mod address;
/// mDNS announcing and browsing.
pub mod discovery;
/// Peer news and the peer table.
pub mod peers;
/// The receiving thread.
pub mod rx;
/// The sending side.
pub mod send;
/// Datagram sockets.
pub mod transport;

pub use transport::{MemoryNetwork, MemoryTransport, Transport, UdpTransport};
