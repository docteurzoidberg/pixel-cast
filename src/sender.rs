//! Sending: UDP sockets (unicast or multicast) and the per-universe state of a stream.

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};

use crate::geometry::Geometry;
use crate::sacn::{self, Source};
use crate::target::{Destination, Target};

/// Identical packets sent before unchanged data is suppressed (E1.31 §6.6.2).
pub const REPEATS_BEFORE_SUPPRESSION: u8 = 3;
/// Interval of keep-alive packets for unchanged data: §6.6.2 asks for 800 to 1000 ms.
pub const KEEP_ALIVE: Duration = Duration::from_millis(900);
/// Stream-terminated packets sent per universe at the end (§6.2.6).
pub const TERMINATION_PACKETS: u8 = 3;

/// A UDP socket and where its packets go.
#[derive(Debug)]
pub struct Sender {
    socket: UdpSocket,
    route: Route,
}

#[derive(Debug)]
enum Route {
    Unicast(Vec<SocketAddr>),
    Multicast { port: u16 },
}

impl Sender {
    /// Opens the socket and resolves host names (IPv4 only).
    pub fn open(destination: &Destination) -> Result<Self, SendError> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| SendError::socket(&e))?;
        let (route, bind) = match destination {
            Destination::Unicast { hosts, port } => {
                let mut addresses = Vec::new();
                for host in hosts {
                    addresses.push(resolve(host, *port)?);
                }
                // allows a broadcast address as host
                socket
                    .set_broadcast(true)
                    .map_err(|e| SendError::socket(&e))?;
                (Route::Unicast(addresses), Ipv4Addr::UNSPECIFIED)
            }
            Destination::Multicast {
                port,
                ttl,
                interface,
            } => {
                socket
                    .set_multicast_ttl_v4(*ttl)
                    .map_err(|e| SendError::socket(&e))?;
                // lets receivers on this machine (a simulator) see the stream
                socket
                    .set_multicast_loop_v4(true)
                    .map_err(|e| SendError::socket(&e))?;
                if let Some(interface) = interface {
                    socket
                        .set_multicast_if_v4(interface)
                        .map_err(|e| SendError::socket(&e))?;
                }
                (
                    Route::Multicast { port: *port },
                    interface.unwrap_or(Ipv4Addr::UNSPECIFIED),
                )
            }
        };
        socket
            .bind(&SocketAddrV4::new(bind, 0).into())
            .map_err(|e| SendError::socket(&e))?;
        Ok(Sender {
            socket: socket.into(),
            route,
        })
    }

    /// The addresses packets of `universe` go to.
    pub fn addresses(&self, universe: u16) -> Vec<SocketAddr> {
        match &self.route {
            Route::Unicast(addresses) => addresses.clone(),
            Route::Multicast { port } => vec![SocketAddr::V4(SocketAddrV4::new(
                sacn::multicast_group(universe),
                *port,
            ))],
        }
    }

    /// Sends one packet of `universe` to each of its addresses, going on after a failure: returns the datagrams
    /// sent, the datagrams that failed and the last failure.
    pub fn send(&self, universe: u16, packet: &[u8]) -> (usize, usize, Option<SendError>) {
        let (mut sent, mut failed, mut error) = (0, 0, None);
        for address in self.addresses(universe) {
            match self.send_to(packet, address) {
                Ok(()) => sent += 1,
                Err(e) => {
                    failed += 1;
                    error = Some(e);
                }
            }
        }
        (sent, failed, error)
    }

    fn send_to(&self, packet: &[u8], address: SocketAddr) -> Result<(), SendError> {
        self.socket
            .send_to(packet, address)
            .map(|_| ())
            .map_err(|error| SendError::Send {
                address,
                message: error.to_string(),
            })
    }
}

fn resolve(host: &str, port: u16) -> Result<SocketAddr, SendError> {
    let unresolved = |message: String| SendError::Resolve {
        host: host.to_owned(),
        message,
    };
    (host, port)
        .to_socket_addrs()
        .map_err(|error| unresolved(error.to_string()))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| unresolved("no IPv4 address".to_owned()))
}

/// What a universe last sent, for sequences and the suppression of unchanged data.
#[derive(Debug)]
struct UniverseState {
    universe: u16,
    sequence: u8,
    last_values: Vec<u8>,
    repeats: u8,
    last_sent: Option<Instant>,
}

impl UniverseState {
    fn new(universe: u16) -> Self {
        UniverseState {
            universe,
            sequence: 0,
            last_values: Vec::new(),
            repeats: 0,
            last_sent: None,
        }
    }

    /// Whether `values` must go out at `now`: new data always, unchanged data three times, then as a keep-alive.
    fn is_due(&self, values: &[u8], now: Instant) -> bool {
        match self.last_sent {
            None => true,
            Some(_) if values != self.last_values.as_slice() => true,
            Some(_) if self.repeats < REPEATS_BEFORE_SUPPRESSION => true,
            Some(last) => now.duration_since(last) >= KEEP_ALIVE,
        }
    }

    fn record(&mut self, values: &[u8], now: Instant) {
        if values == self.last_values.as_slice() {
            self.repeats = self.repeats.saturating_add(1);
        } else {
            self.last_values.clear();
            self.last_values.extend_from_slice(values);
            self.repeats = 1;
        }
        self.last_sent = Some(now);
    }

    fn next_sequence(&mut self) -> u8 {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        sequence
    }
}

/// Counts of one call, for reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sent {
    /// E1.31 packets built (one per universe sent).
    pub packets: usize,
    /// UDP datagrams sent (packets × unicast hosts).
    pub datagrams: usize,
    /// Universes left out because their data had not changed (§6.6.2).
    pub suppressed: usize,
    /// UDP datagrams the system refused to send.
    pub errors: usize,
}

impl std::ops::AddAssign for Sent {
    fn add_assign(&mut self, other: Sent) {
        self.packets += other.packets;
        self.datagrams += other.datagrams;
        self.suppressed += other.suppressed;
        self.errors += other.errors;
    }
}

/// A stream of pictures to one target: splits them into universes and sends what is due.
#[derive(Debug)]
pub struct Stream {
    sender: Sender,
    source: Source,
    geometry: Geometry,
    universes: Vec<UniverseState>,
    buffer: Vec<u8>,
    last_error: Option<SendError>,
}

impl Stream {
    pub fn open(target: &Target) -> Result<Self, SendError> {
        Ok(Stream::new(
            Sender::open(&target.destination)?,
            target.source.clone(),
            target.geometry,
        ))
    }

    pub fn new(sender: Sender, source: Source, geometry: Geometry) -> Self {
        Stream {
            sender,
            source,
            geometry,
            universes: geometry.universes().map(UniverseState::new).collect(),
            buffer: Vec::with_capacity(sacn::HEADER_LEN + sacn::MAX_CHANNELS),
            last_error: None,
        }
    }

    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// The last send failure since the previous call, if any: failures do not stop the stream (a network can come
    /// back), they are counted in [`Sent::errors`].
    pub fn take_error(&mut self) -> Option<SendError> {
        self.last_error.take()
    }

    fn send(&mut self, universe: u16, sent: &mut Sent) -> bool {
        let (ok, failed, error) = self.sender.send(universe, &self.buffer);
        sent.packets += 1;
        sent.datagrams += ok;
        sent.errors += failed;
        if error.is_some() {
            self.last_error = error;
        }
        ok > 0
    }

    /// Sends an RGB picture of the stream's size, as of `now`: each universe whose data changed, or that is due a
    /// repeat or a keep-alive.
    ///
    /// A universe whose packet could not be sent at all is sent again at the next call.
    pub fn send_frame(&mut self, frame: &[u8], now: Instant) -> Sent {
        let mut sent = Sent::default();
        for (i, (universe, values)) in self.geometry.split(frame).enumerate() {
            let state = &mut self.universes[i];
            debug_assert_eq!(state.universe, universe);
            if !state.is_due(values, now) {
                sent.suppressed += 1;
                continue;
            }
            let sequence = state.next_sequence();
            sacn::write_data_packet(
                &mut self.buffer,
                &self.source,
                universe,
                sequence,
                0,
                values,
            )
            .expect("the geometry keeps universes and lengths in range");
            if self.send(universe, &mut sent) {
                self.universes[i].record(values, now);
            }
        }
        sent
    }

    /// Ends the stream: three stream-terminated packets per universe (§6.2.6).
    ///
    /// The standard has receivers ignore their values, but some receivers ignore the option bit instead and apply
    /// the values as a frame: the packets carry the last values sent, so that such a display keeps its picture.
    pub fn terminate(&mut self) -> Sent {
        let mut sent = Sent::default();
        for _ in 0..TERMINATION_PACKETS {
            for i in 0..self.universes.len() {
                let state = &mut self.universes[i];
                let universe = state.universe;
                let sequence = state.next_sequence();
                let values = std::mem::take(&mut state.last_values);
                sacn::write_data_packet(
                    &mut self.buffer,
                    &self.source,
                    state.universe,
                    sequence,
                    sacn::options::STREAM_TERMINATED,
                    &values,
                )
                .expect("the geometry keeps universes and lengths in range");
                state.last_values = values;
                self.send(universe, &mut sent);
            }
        }
        sent
    }
}

/// A network failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendError {
    Socket(String),
    Resolve {
        host: String,
        message: String,
    },
    Send {
        address: SocketAddr,
        message: String,
    },
}

impl SendError {
    fn socket(error: &std::io::Error) -> Self {
        SendError::Socket(error.to_string())
    }
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Socket(message) => write!(f, "cannot open a UDP socket: {message}"),
            SendError::Resolve { host, message } => write!(f, "cannot resolve {host:?}: {message}"),
            SendError::Send { address, message } => {
                write!(f, "cannot send to {address}: {message}")
            }
        }
    }
}

impl std::error::Error for SendError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn unchanged_data_goes_three_times_then_as_keep_alive() {
        let start = Instant::now();
        let mut state = UniverseState::new(1);
        let values = [1, 2, 3];
        let mut sent_at = Vec::new();
        for ms in (0..3000).step_by(100) {
            let now = at(start, ms);
            if state.is_due(&values, now) {
                state.record(&values, now);
                sent_at.push(ms);
            }
        }
        assert_eq!(sent_at, [0, 100, 200, 1100, 2000, 2900]);
    }

    #[test]
    fn changed_data_always_goes() {
        let start = Instant::now();
        let mut state = UniverseState::new(1);
        for i in 0..10u8 {
            let now = at(start, u64::from(i) * 10);
            assert!(state.is_due(&[i], now));
            state.record(&[i], now);
        }
        // back to repeats after a change
        let now = at(start, 200);
        for _ in 0..2 {
            assert!(state.is_due(&[9], now));
            state.record(&[9], now);
        }
        assert!(!state.is_due(&[9], now));
    }

    #[test]
    fn multicast_addresses_follow_the_universe() {
        let sender = Sender::open(&Destination::Multicast {
            port: 5568,
            ttl: 1,
            interface: None,
        })
        .unwrap();
        assert_eq!(
            sender.addresses(0x0102),
            ["239.255.1.2:5568".parse().unwrap()]
        );
    }

    #[test]
    fn hosts_without_an_ipv4_address_are_refused() {
        // an IPv6 literal: refused without any name lookup
        let error = Sender::open(&Destination::Unicast {
            hosts: vec!["::1".into()],
            port: 5568,
        })
        .unwrap_err();
        assert!(matches!(error, SendError::Resolve { .. }), "{error}");
    }
}
