//! A stream to a UDP receiver on the loopback interface, which rebuilds the picture from the packets.

use std::collections::HashMap;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

use pixel_cast::sacn::{Cid, options};
use pixel_cast::sender::Stream;
use pixel_cast::target::{Hosts, TargetSpec};

/// The fields of a received data packet the test looks at, read independently of `pixel_cast::sacn`.
#[derive(Debug)]
struct Received {
    cid: [u8; 16],
    priority: u8,
    sequence: u8,
    options: u8,
    universe: u16,
    values: Vec<u8>,
}

fn parse(packet: &[u8]) -> Received {
    assert!(packet.len() >= 126, "a packet of {} bytes", packet.len());
    assert_eq!(&packet[4..16], b"ASC-E1.17\0\0\0");
    let count = u16::from_be_bytes([packet[123], packet[124]]) as usize;
    assert_eq!(count, packet.len() - 125, "property value count");
    assert_eq!(packet[125], 0, "start code");
    Received {
        cid: packet[22..38].try_into().unwrap(),
        priority: packet[108],
        sequence: packet[111],
        options: packet[112],
        universe: u16::from_be_bytes([packet[113], packet[114]]),
        values: packet[126..].to_vec(),
    }
}

struct Receiver {
    socket: UdpSocket,
}

impl Receiver {
    fn bind() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Receiver { socket }
    }

    fn port(&self) -> u16 {
        self.socket.local_addr().unwrap().port()
    }

    /// The next packet, whatever it is.
    fn raw(&self) -> Vec<u8> {
        let mut buffer = [0u8; 1500];
        let (len, _) = self
            .socket
            .recv_from(&mut buffer)
            .expect("a packet before the timeout");
        buffer[..len].to_vec()
    }

    /// The next `count` data packets; universe discovery packets in between are left out.
    fn receive(&self, count: usize) -> Vec<Received> {
        let mut packets = Vec::new();
        while packets.len() < count {
            let packet = self.raw();
            if packet[18..22] != [0, 0, 0, 8] {
                packets.push(parse(&packet));
            }
        }
        packets
    }
}

fn stream_to(receiver: &Receiver, size: &str) -> Stream {
    let spec = TargetSpec {
        host: Some(Hosts::One("127.0.0.1".into())),
        port: Some(receiver.port()),
        size: Some(size.into()),
        priority: Some(120),
        ..TargetSpec::default()
    };
    Stream::open(&spec.resolve(Cid([9; 16])).unwrap()).unwrap()
}

#[test]
fn the_receiver_rebuilds_the_picture() {
    let receiver = Receiver::bind();
    let mut stream = stream_to(&receiver, "192x64");
    let geometry = stream.geometry();
    let frame: Vec<u8> = (0..geometry.frame_len())
        .map(|i| (i * 31 % 251) as u8)
        .collect();

    let sent = stream.send_frame(&frame, Instant::now());
    assert_eq!((sent.packets, sent.datagrams, sent.suppressed), (73, 73, 0));

    let mut rebuilt = vec![0u8; frame.len()];
    for packet in receiver.receive(73) {
        assert_eq!(packet.cid, [9; 16]);
        assert_eq!(packet.priority, 120);
        assert_eq!(packet.options, 0);
        assert_eq!(packet.sequence, 0);
        let at = (packet.universe as usize - 1) * 170 * 3;
        rebuilt[at..at + packet.values.len()].copy_from_slice(&packet.values);
    }
    assert_eq!(rebuilt, frame);
}

#[test]
fn unchanged_frames_are_suppressed_then_kept_alive_and_the_end_is_signalled() {
    let receiver = Receiver::bind();
    let mut stream = stream_to(&receiver, "16x16"); // 256 lights: universes 1 and 2
    let start = Instant::now();
    let mut frame = vec![10u8; stream.geometry().frame_len()];

    // three identical frames go out, the fourth is suppressed
    for i in 0..4 {
        let sent = stream.send_frame(&frame, start + Duration::from_millis(i * 33));
        assert_eq!(sent.packets, if i < 3 { 2 } else { 0 }, "frame {i}");
    }
    // a change in the second universe only
    *frame.last_mut().unwrap() = 99;
    let sent = stream.send_frame(&frame, start + Duration::from_millis(150));
    assert_eq!((sent.packets, sent.suppressed), (1, 1));
    // a keep-alive for the first universe once 900 ms have passed since its last packet
    let sent = stream.send_frame(&frame, start + Duration::from_millis(66 + 900));
    assert_eq!(sent.packets, 2); // universe 1: keep-alive; universe 2: its second repeat

    let terminated = stream.terminate();
    assert_eq!(terminated.packets, 6);

    let packets = receiver.receive(3 * 2 + 1 + 2 + 6);
    let mut sequences: HashMap<u16, Vec<u8>> = HashMap::new();
    for packet in &packets {
        sequences
            .entry(packet.universe)
            .or_default()
            .push(packet.sequence);
    }
    assert_eq!(sequences[&1], [0, 1, 2, 3, 4, 5, 6]);
    assert_eq!(sequences[&2], [0, 1, 2, 3, 4, 5, 6, 7]);

    let ends: Vec<_> = packets
        .iter()
        .filter(|p| p.options & options::STREAM_TERMINATED != 0)
        .collect();
    assert_eq!(ends.len(), 6);
    // the end carries the last values sent
    let last_of_2 = ends.iter().find(|p| p.universe == 2).unwrap();
    assert_eq!(*last_of_2.values.last().unwrap(), 99);
}

#[test]
fn a_serpentine_matrix_receives_its_leds_in_wiring_order() {
    let receiver = Receiver::bind();
    let spec = TargetSpec {
        host: Some(Hosts::One("127.0.0.1".into())),
        port: Some(receiver.port()),
        size: Some("3x2".into()),
        layout: Some("serpentine-rows".into()),
        ..TargetSpec::default()
    };
    let mut stream = Stream::open(&spec.resolve(Cid([9; 16])).unwrap()).unwrap();
    // light n of the picture (rows from the top-left) is (n, n, n)
    let frame: Vec<u8> = (0..6u8).flat_map(|n| [n; 3]).collect();
    stream.send_frame(&frame, Instant::now());
    let packet = receiver.receive(1).remove(0);
    let leds: Vec<u8> = packet.values.chunks(3).map(|rgb| rgb[0]).collect();
    // the second row runs back: LEDs 3, 4, 5 show lights 5, 4, 3
    assert_eq!(leds, [0, 1, 2, 5, 4, 3]);
}

#[test]
fn the_stream_lists_its_universes_for_monitors() {
    let receiver = Receiver::bind();
    let mut stream = stream_to(&receiver, "192x64");
    let frame = vec![1u8; stream.geometry().frame_len()];
    let start = Instant::now();
    stream.send_frame(&frame, start);
    // a universe discovery packet comes first: universes 1 to 73, page 0 of 0
    let discovery = receiver.raw();
    assert_eq!(
        &discovery[18..22],
        &[0, 0, 0, 8],
        "VECTOR_ROOT_E131_EXTENDED"
    );
    assert_eq!(
        &discovery[40..44],
        &[0, 0, 0, 2],
        "VECTOR_E131_EXTENDED_DISCOVERY"
    );
    assert_eq!(
        &discovery[114..118],
        &[0, 0, 0, 1],
        "VECTOR_UNIVERSE_DISCOVERY_UNIVERSE_LIST"
    );
    assert_eq!((discovery[118], discovery[119]), (0, 0));
    let universes: Vec<u16> = discovery[120..]
        .chunks(2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .collect();
    assert_eq!(universes, (1..=73).collect::<Vec<u16>>());
    receiver.receive(73);
    // not again before 10 s, again after
    stream.send_frame(&frame, start + Duration::from_secs(5));
    stream.send_frame(&frame, start + Duration::from_secs(10));
    let mut discoveries = 0;
    receiver
        .socket
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut buffer = [0u8; 1500];
    while let Ok((len, _)) = receiver.socket.recv_from(&mut buffer) {
        if buffer[..len][18..22] == [0, 0, 0, 8] {
            discoveries += 1;
        }
    }
    assert_eq!(discoveries, 1);
}
