//! Two nodes on 127.0.0.1 with every engine thread running and fake audio
//! devices on a real clock: A talks, B listens. No mDNS; A has B as a
//! static peer.

use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use crossbeam_channel::unbounded;
use takkie_core::PeerId;
use takkie_engine::audio::fake::{FakeSink, FakeSource, LiveSink, LiveSource, Signal};
use takkie_engine::audio::mix::{MixShared, MixThread};
use takkie_engine::audio::tx::{LevelMeter, TxThread};
use takkie_engine::net::hello::HelloThread;
use takkie_engine::net::peers::{PEER_TIMEOUT, PeerOutputs, PeerThread, Peers};
use takkie_engine::net::rx::{RxOutputs, RxThread};
use takkie_engine::net::send::{PacketSender, SendThread};
use takkie_engine::net::{Transport, UdpTransport};

const RATE: u32 = 48_000;
const TONE: f32 = 440.0;
const AMPLITUDE: f32 = 0.4;

struct Node {
    addr: SocketAddr,
    speaker: LiveSink,
    _threads: (
        TxThread,
        SendThread,
        RxThread,
        MixThread,
        PeerThread,
        HelloThread,
    ),
}

fn node(
    id: u64,
    channel: u8,
    talking: bool,
    static_peers: Vec<SocketAddr>,
) -> Result<Node, Box<dyn Error>> {
    let me = PeerId::new(id);
    let transport: Arc<dyn Transport> = Arc::new(UdpTransport::bind(0)?);
    let addr = SocketAddr::from(([127, 0, 0, 1], transport.local_addr().port()));
    let channel = Arc::new(AtomicU8::new(channel));
    let targets = Arc::new(ArcSwap::from_pointee(Vec::new()));
    let transmitting = Arc::new(AtomicBool::new(talking));

    let mic = LiveSource::new(FakeSource::new(
        RATE,
        Signal::Sine {
            frequency: TONE,
            amplitude: AMPLITUDE,
        },
    ));
    let (tx_events, tx_out) = unbounded();
    let tx = TxThread::spawn(
        Box::new(mic),
        Arc::clone(&transmitting),
        Arc::new(LevelMeter::default()),
        tx_events,
    )?;
    let sender = Arc::new(PacketSender::new(
        Arc::clone(&transport),
        me,
        Arc::clone(&channel),
        Arc::clone(&targets),
    ));
    let send = SendThread::spawn(tx_out, Arc::clone(&sender))?;

    let (audio, audio_out) = unbounded();
    let (news, news_out) = unbounded();
    let rx = RxThread::spawn(
        transport,
        me,
        Arc::clone(&channel),
        RxOutputs {
            audio: audio.clone(),
            peers: news,
        },
    )?;
    let speaker = LiveSink::new(FakeSink::new(RATE, RATE as usize / 5));
    let mix = MixThread::spawn(
        audio_out,
        Box::new(speaker.clone()),
        MixShared {
            transmitting,
            ..MixShared::default()
        },
    )?;
    let (events, _) = unbounded();
    let table = Peers::new(channel, targets, PEER_TIMEOUT);
    let known = table.view();
    let peers = PeerThread::spawn(
        news_out,
        table,
        PeerOutputs {
            events,
            mixer: audio,
        },
    )?;
    let hello = HelloThread::spawn(
        sender,
        "node",
        Duration::from_millis(200),
        known,
        static_peers,
    )?;
    Ok(Node {
        addr,
        speaker,
        _threads: (tx, send, rx, mix, peers, hello),
    })
}

fn last_second(node: &Node) -> Vec<f32> {
    node.speaker.last(RATE as usize)
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

fn frequency(samples: &[f32]) -> f32 {
    let crossings = samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count();
    crossings as f32 / 2.0 / (samples.len() as f32 / RATE as f32)
}

#[test]
fn b_hears_a_on_the_same_channel() -> Result<(), Box<dyn Error>> {
    let b = node(2, 3, false, Vec::new())?;
    let a = node(1, 3, true, vec![b.addr])?;
    let sent = AMPLITUDE / 2.0_f32.sqrt();
    let deadline = Instant::now() + Duration::from_secs(8);
    let (level, pitch) = loop {
        let heard = b.speaker.last(RATE as usize / 5);
        let (level, pitch) = (rms(&heard), frequency(&heard));
        let clean = (level - sent).abs() / sent < 0.3 && (pitch - TONE).abs() / TONE < 0.02;
        if clean || Instant::now() >= deadline {
            break (level, pitch);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(a);

    assert!(
        (level - sent).abs() / sent < 0.3,
        "B heard a level of {level}, A sent {sent}"
    );
    assert!((pitch - TONE).abs() / TONE < 0.02, "B heard {pitch} Hz");
    Ok(())
}

#[test]
fn b_hears_nothing_from_another_channel() -> Result<(), Box<dyn Error>> {
    let b = node(2, 4, false, Vec::new())?;
    let a = node(1, 3, true, vec![b.addr])?;
    std::thread::sleep(Duration::from_millis(2_500));
    let heard = last_second(&b);
    drop(a);

    assert_eq!(heard.len(), RATE as usize);
    assert!(heard.iter().all(|s| *s == 0.0), "B heard {}", rms(&heard));
    Ok(())
}
