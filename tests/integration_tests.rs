// Integration tests for VideoLAN Audio Streamer
use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn test_udp_socket_creation_and_binding() {
    // Test that we can create and bind a UDP socket
    let result = UdpSocket::bind("127.0.0.1:0");
    assert!(result.is_ok());
    
    let socket = result.unwrap();
    let local_addr = socket.local_addr();
    assert!(local_addr.is_ok());
}

#[test]
fn test_udp_socket_send_receive() {
    // Create sender and receiver sockets
    let receiver = UdpSocket::bind("127.0.0.1:0").expect("Failed to bind receiver");
    let receiver_addr = receiver.local_addr().unwrap();
    
    let sender = UdpSocket::bind("127.0.0.1:0").expect("Failed to bind sender");
    
    // Set timeouts
    receiver.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    
    // Send data
    let test_data = b"test audio packet";
    sender.send_to(test_data, receiver_addr).unwrap();
    
    // Receive data
    let mut buf = [0u8; 1024];
    let result = receiver.recv_from(&mut buf);
    
    assert!(result.is_ok());
    let (len, _from) = result.unwrap();
    assert_eq!(len, test_data.len());
    assert_eq!(&buf[..len], test_data);
}

#[test]
fn test_audio_buffer_simulation() {
    // Simulate the audio buffer used in the application
    type AudioBuffer = Arc<Mutex<VecDeque<u8>>>;
    let buffer: AudioBuffer = Arc::new(Mutex::new(VecDeque::new()));
    
    // Simulate pushing opus packets
    {
        let mut buf = buffer.lock().unwrap();
        // Length prefix for 100 byte packet
        buf.push_back(100);
        buf.push_back(0);
        // Add 100 bytes of data
        for i in 0..100 {
            buf.push_back(i as u8);
        }
    }
    
    // Verify buffer contents
    {
        let buf = buffer.lock().unwrap();
        assert_eq!(buf.len(), 102); // 2 bytes length + 100 bytes data
    }
    
    // Simulate packet extraction
    {
        let mut buf = buffer.lock().unwrap();
        
        // Read length
        let b0 = buf[0] as u16;
        let b1 = buf[1] as u16;
        let len = (b0 | (b1 << 8)) as usize;
        
        assert_eq!(len, 100);
        
        // Extract packet
        buf.drain(..2); // Remove length prefix
        let packet: Vec<u8> = buf.drain(..len).collect();
        
        assert_eq!(packet.len(), 100);
        assert_eq!(buf.len(), 0);
    }
}

#[test]
fn test_peer_list_management() {
    type Peerlist = Arc<Mutex<Vec<SocketAddr>>>;
    let peers: Peerlist = Arc::new(Mutex::new(Vec::new()));
    
    // Add peers
    {
        let mut p = peers.lock().unwrap();
        p.push("192.168.1.100:8080".parse().unwrap());
        p.push("192.168.1.101:8080".parse().unwrap());
        p.push("192.168.1.102:8080".parse().unwrap());
    }
    
    // Verify peers
    {
        let p = peers.lock().unwrap();
        assert_eq!(p.len(), 3);
    }
    
    // Remove a peer
    {
        let mut p = peers.lock().unwrap();
        p.retain(|addr| addr.ip().to_string() != "192.168.1.101");
    }
    
    {
        let p = peers.lock().unwrap();
        assert_eq!(p.len(), 2);
    }
}

#[test]
fn test_concurrent_audio_buffer_access() {
    type AudioBuffer = Arc<Mutex<VecDeque<u8>>>;
    let buffer: AudioBuffer = Arc::new(Mutex::new(VecDeque::new()));
    
    let mut handles = vec![];
    
    // Spawn multiple threads writing to buffer
    for i in 0..10 {
        let buffer_clone = buffer.clone();
        let handle = std::thread::spawn(move || {
            for j in 0..100 {
                let mut buf = buffer_clone.lock().unwrap();
                buf.push_back((i * 100 + j) as u8);
            }
        });
        handles.push(handle);
    }
    
    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }
    
    // Verify total size
    let buf = buffer.lock().unwrap();
    assert_eq!(buf.len(), 1000); // 10 threads * 100 bytes each
}

#[test]
fn test_nonblocking_udp_socket() {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("Failed to bind socket");
    
    // Set nonblocking mode
    let result = socket.set_nonblocking(true);
    assert!(result.is_ok());
    
    // Try to receive (should fail immediately with WouldBlock)
    let mut buf = [0u8; 1024];
    let result = socket.recv_from(&mut buf);
    
    // In non-blocking mode, recv should return WouldBlock error
    assert!(result.is_err());
    if let Err(e) = result {
        assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock);
    }
}

#[test]
fn test_multiple_packet_buffer() {
    type AudioBuffer = Arc<Mutex<VecDeque<u8>>>;
    let buffer: AudioBuffer = Arc::new(Mutex::new(VecDeque::new()));
    
    // Helper function to push length-prefixed packets
    fn push_packet(buffer: &AudioBuffer, data: &[u8]) {
        let mut buf = buffer.lock().unwrap();
        let len = data.len() as u16;
        buf.push_back(len as u8);
        buf.push_back((len >> 8) as u8);
        buf.extend(data.iter().copied());
    }
    
    // Push three packets
    push_packet(&buffer, &[1, 2, 3]);
    push_packet(&buffer, &[4, 5, 6, 7, 8]);
    push_packet(&buffer, &[9, 10]);
    
    // Total: 3 packets * 2 bytes length + (3 + 5 + 2) bytes data = 16 bytes
    {
        let buf = buffer.lock().unwrap();
        assert_eq!(buf.len(), 16);
    }
    
    // Extract first packet
    {
        let mut buf = buffer.lock().unwrap();
        let b0 = buf[0] as u16;
        let b1 = buf[1] as u16;
        let len = (b0 | (b1 << 8)) as usize;
        
        assert_eq!(len, 3);
        buf.drain(..2);
        let packet: Vec<u8> = buf.drain(..len).collect();
        assert_eq!(packet, vec![1, 2, 3]);
    }
    
    // Verify remaining size
    {
        let buf = buffer.lock().unwrap();
        assert_eq!(buf.len(), 11); // 16 - 5 (one packet extracted)
    }
}

#[test]
fn test_socket_clone() {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("Failed to bind socket");
    let socket_clone = socket.try_clone();
    
    assert!(socket_clone.is_ok());
    
    let clone = socket_clone.unwrap();
    let orig_addr = socket.local_addr().unwrap();
    let clone_addr = clone.local_addr().unwrap();
    
    assert_eq!(orig_addr, clone_addr);
}

#[test]
fn test_opus_frame_sizes() {
    // Test various frame size calculations
    let sample_rate: u32 = 48000;
    let frame_sizes = vec![
        (1, 960),   // 20ms mono
        (2, 1920),  // 20ms stereo
    ];
    
    for (channels, expected) in frame_sizes {
        let frame_size = (sample_rate as usize / 1000) * 20 * channels;
        assert_eq!(frame_size, expected);
    }
}

#[test]
fn test_sample_buffer_frame_extraction() {
    let sample_buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let frame_size = 960;
    
    // Fill buffer with more than one frame
    {
        let mut buf = sample_buffer.lock().unwrap();
        for i in 0..2000 {
            buf.push(i as f32 / 1000.0);
        }
    }
    
    // Extract first frame
    {
        let mut buf = sample_buffer.lock().unwrap();
        let frame: Vec<f32> = buf.drain(..frame_size).collect();
        
        assert_eq!(frame.len(), frame_size);
        assert_eq!(buf.len(), 1040); // 2000 - 960
    }
}

#[test]
fn test_peer_deduplication() {
    type Peerlist = Arc<Mutex<Vec<SocketAddr>>>;
    let peers: Peerlist = Arc::new(Mutex::new(Vec::new()));
    
    let addr: SocketAddr = "192.168.1.100:8080".parse().unwrap();
    
    // Add peer if not exists
    {
        let mut p = peers.lock().unwrap();
        if !p.contains(&addr) {
            p.push(addr);
        }
    }
    
    // Try to add same peer again
    {
        let mut p = peers.lock().unwrap();
        if !p.contains(&addr) {
            p.push(addr);
        }
    }
    
    // Should only have one peer
    let p = peers.lock().unwrap();
    assert_eq!(p.len(), 1);
}
