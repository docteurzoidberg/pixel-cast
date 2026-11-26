//! E1.31 (sACN) data packets, as laid out by ANSI E1.31-2025, Table 4-1 (see `docs/standard/README.md`).
//!
//! Building a packet is pure: no I/O, so every byte can be tested.

use std::fmt;
use std::str::FromStr;

/// The UDP port of E1.31.
pub const PORT: u16 = 5568;
/// The highest DMX value count of a universe (start code excluded).
pub const MAX_CHANNELS: usize = 512;
/// Universe numbers that may carry data (64000 to 65535 are reserved).
pub const UNIVERSES: std::ops::RangeInclusive<u16> = 1..=63999;
/// The highest priority a source may send.
pub const MAX_PRIORITY: u8 = 200;
/// The default priority of the standard.
pub const DEFAULT_PRIORITY: u8 = 100;
/// The longest source name, in UTF-8 bytes (the field holds 64 bytes, null-terminated).
pub const MAX_SOURCE_NAME: usize = 63;

/// Size of everything before the DMX values.
pub const HEADER_LEN: usize = 126;

const ACN_PACKET_IDENTIFIER: &[u8; 12] = b"ASC-E1.17\0\0\0";
const VECTOR_ROOT_E131_DATA: u32 = 0x0000_0004;
const VECTOR_E131_DATA_PACKET: u32 = 0x0000_0002;
const VECTOR_DMP_SET_PROPERTY: u8 = 0x02;
const DMP_ADDRESS_AND_DATA_TYPE: u8 = 0xa1;
const FLAGS: u16 = 0x7000;

/// Framing options (E1.31 §6.2.6).
pub mod options {
    /// The data is for preview (visualisers), not for live output.
    pub const PREVIEW_DATA: u8 = 0x80;
    /// The source stops sending this universe; receivers drop it at once and ignore the values (three such
    /// packets are sent).
    pub const STREAM_TERMINATED: u8 = 0x40;
}

/// Component identifier: identifies a source across universes and restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cid(pub [u8; 16]);

impl Cid {
    /// A new random CID (UUID version 4).
    pub fn random() -> Self {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        // `RandomState` is seeded from the operating system's random source, so its hashers give random words
        // without pulling a dependency for 16 bytes used once per machine.
        let mut bytes = [0u8; 16];
        for (i, chunk) in bytes.chunks_mut(8).enumerate() {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_usize(i);
            chunk.copy_from_slice(&hasher.finish().to_le_bytes());
        }
        bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
        bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
        Cid(bytes)
    }
}

impl fmt::Display for Cid {
    /// The UUID form: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for Cid {
    type Err = HeaderError;

    /// Reads 32 hexadecimal digits, dashes allowed anywhere (UUID form or bare).
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits: Vec<u8> = text.bytes().filter(|&b| b != b'-').collect();
        if digits.len() != 32 {
            return Err(HeaderError::Cid(text.to_owned()));
        }
        let mut bytes = [0u8; 16];
        for (byte, pair) in bytes.iter_mut().zip(digits.chunks(2)) {
            let pair = std::str::from_utf8(pair).map_err(|_| HeaderError::Cid(text.to_owned()))?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| HeaderError::Cid(text.to_owned()))?;
        }
        Ok(Cid(bytes))
    }
}

/// What a source sends in every packet, whatever the universe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    cid: Cid,
    name: String,
    priority: u8,
}

impl Source {
    pub fn new(cid: Cid, name: &str, priority: u8) -> Result<Self, HeaderError> {
        if name.len() > MAX_SOURCE_NAME {
            return Err(HeaderError::SourceName(name.to_owned()));
        }
        if priority > MAX_PRIORITY {
            return Err(HeaderError::Priority(priority));
        }
        Ok(Source {
            cid,
            name: name.to_owned(),
            priority,
        })
    }

    pub fn cid(&self) -> Cid {
        self.cid
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn priority(&self) -> u8 {
        self.priority
    }
}

/// Writes one data packet into `out` (cleared first) and returns its length.
///
/// `values` are the DMX values of the universe, start code excluded (at most 512). `options` is a mask of
/// [`options`] bits.
pub fn write_data_packet(
    out: &mut Vec<u8>,
    source: &Source,
    universe: u16,
    sequence: u8,
    options: u8,
    values: &[u8],
) -> Result<usize, HeaderError> {
    if !UNIVERSES.contains(&universe) {
        return Err(HeaderError::Universe(universe));
    }
    if values.len() > MAX_CHANNELS {
        return Err(HeaderError::TooManyValues(values.len()));
    }
    let length = HEADER_LEN + values.len();
    out.clear();
    out.reserve(length);

    // Root layer
    out.extend_from_slice(&0x0010u16.to_be_bytes()); // preamble size
    out.extend_from_slice(&0x0000u16.to_be_bytes()); // postamble size
    out.extend_from_slice(ACN_PACKET_IDENTIFIER);
    out.extend_from_slice(&flags_and_length(length - 16));
    out.extend_from_slice(&VECTOR_ROOT_E131_DATA.to_be_bytes());
    out.extend_from_slice(&source.cid.0);

    // Framing layer
    out.extend_from_slice(&flags_and_length(length - 38));
    out.extend_from_slice(&VECTOR_E131_DATA_PACKET.to_be_bytes());
    let mut name = [0u8; 64];
    name[..source.name.len()].copy_from_slice(source.name.as_bytes());
    out.extend_from_slice(&name);
    out.push(source.priority);
    out.extend_from_slice(&0u16.to_be_bytes()); // synchronization address: none
    out.push(sequence);
    out.push(options);
    out.extend_from_slice(&universe.to_be_bytes());

    // DMP layer
    out.extend_from_slice(&flags_and_length(length - 115));
    out.push(VECTOR_DMP_SET_PROPERTY);
    out.push(DMP_ADDRESS_AND_DATA_TYPE);
    out.extend_from_slice(&0u16.to_be_bytes()); // first property address
    out.extend_from_slice(&1u16.to_be_bytes()); // address increment
    out.extend_from_slice(&((values.len() + 1) as u16).to_be_bytes()); // property value count, start code included
    out.push(0x00); // DMX start code
    out.extend_from_slice(values);

    debug_assert_eq!(out.len(), length);
    Ok(length)
}

/// A data packet in a new buffer: see [`write_data_packet`].
pub fn data_packet(
    source: &Source,
    universe: u16,
    sequence: u8,
    options: u8,
    values: &[u8],
) -> Result<Vec<u8>, HeaderError> {
    let mut out = Vec::new();
    write_data_packet(&mut out, source, universe, sequence, options, values)?;
    Ok(out)
}

fn flags_and_length(length: usize) -> [u8; 2] {
    (FLAGS | length as u16).to_be_bytes()
}

/// The multicast group of a universe: 239.255.<high byte>.<low byte>.
pub fn multicast_group(universe: u16) -> std::net::Ipv4Addr {
    let [high, low] = universe.to_be_bytes();
    std::net::Ipv4Addr::new(239, 255, high, low)
}

/// A header field out of the range of the standard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderError {
    Cid(String),
    SourceName(String),
    Priority(u8),
    Universe(u16),
    TooManyValues(usize),
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderError::Cid(text) => write!(f, "a CID needs 32 hexadecimal digits: {text:?}"),
            HeaderError::SourceName(name) => write!(
                f,
                "the source name is longer than {MAX_SOURCE_NAME} bytes: {name:?}"
            ),
            HeaderError::Priority(priority) => {
                write!(f, "priority {priority} is above {MAX_PRIORITY}")
            }
            HeaderError::Universe(universe) => write!(
                f,
                "universe {universe} is outside {}..={}",
                UNIVERSES.start(),
                UNIVERSES.end()
            ),
            HeaderError::TooManyValues(count) => {
                write!(
                    f,
                    "{count} values do not fit in a universe ({MAX_CHANNELS} at most)"
                )
            }
        }
    }
}

impl std::error::Error for HeaderError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        let cid = Cid([
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10,
        ]);
        Source::new(cid, "pixel-cast", 150).unwrap()
    }

    fn be16(packet: &[u8], at: usize) -> u16 {
        u16::from_be_bytes([packet[at], packet[at + 1]])
    }

    fn be32(packet: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(packet[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn fields_sit_at_the_offsets_of_the_standard() {
        let values: Vec<u8> = (0..510).map(|i| i as u8).collect();
        let packet =
            data_packet(&source(), 0x1234, 7, options::STREAM_TERMINATED, &values).unwrap();
        assert_eq!(packet.len(), 636); // 126 header bytes + 510 values
        // Root layer
        assert_eq!(be16(&packet, 0), 0x0010);
        assert_eq!(be16(&packet, 2), 0x0000);
        assert_eq!(&packet[4..16], b"ASC-E1.17\0\0\0");
        assert_eq!(be16(&packet, 16), 0x7000 | 620);
        assert_eq!(be32(&packet, 18), 0x0000_0004);
        assert_eq!(&packet[22..38], &source().cid().0);
        // Framing layer
        assert_eq!(be16(&packet, 38), 0x7000 | 598);
        assert_eq!(be32(&packet, 40), 0x0000_0002);
        assert_eq!(&packet[44..54], b"pixel-cast");
        assert!(packet[54..108].iter().all(|&b| b == 0));
        assert_eq!(packet[108], 150);
        assert_eq!(be16(&packet, 109), 0);
        assert_eq!(packet[111], 7);
        assert_eq!(packet[112], 0x40);
        assert_eq!(be16(&packet, 113), 0x1234);
        // DMP layer
        assert_eq!(be16(&packet, 115), 0x7000 | 521);
        assert_eq!(packet[117], 0x02);
        assert_eq!(packet[118], 0xa1);
        assert_eq!(be16(&packet, 119), 0);
        assert_eq!(be16(&packet, 121), 1);
        assert_eq!(be16(&packet, 123), 511);
        assert_eq!(packet[125], 0x00);
        assert_eq!(&packet[126..], &values[..]);
    }

    #[test]
    fn short_universes_shrink_every_length() {
        let packet = data_packet(&source(), 1, 0, 0, &[9; 144]).unwrap();
        assert_eq!(packet.len(), 126 + 144);
        assert_eq!(be16(&packet, 16) & 0x0fff, (packet.len() - 16) as u16);
        assert_eq!(be16(&packet, 38) & 0x0fff, (packet.len() - 38) as u16);
        assert_eq!(be16(&packet, 115) & 0x0fff, (packet.len() - 115) as u16);
        assert_eq!(be16(&packet, 123), 145);
    }

    #[test]
    fn header_fields_out_of_range_are_refused() {
        let cid = Cid([0; 16]);
        assert_eq!(Source::new(cid, "p", 201), Err(HeaderError::Priority(201)));
        assert!(Source::new(cid, &"x".repeat(63), 0).is_ok());
        assert!(matches!(
            Source::new(cid, &"x".repeat(64), 0),
            Err(HeaderError::SourceName(_))
        ));
        // 21 three-byte characters: 63 bytes fit, one more does not
        assert!(Source::new(cid, &"€".repeat(21), 0).is_ok());
        assert!(Source::new(cid, &"€".repeat(22), 0).is_err());
        for universe in [0, 64000, 65535] {
            assert_eq!(
                data_packet(&source(), universe, 0, 0, &[]),
                Err(HeaderError::Universe(universe))
            );
        }
        assert_eq!(
            data_packet(&source(), 1, 0, 0, &[0; 513]),
            Err(HeaderError::TooManyValues(513))
        );
        assert!(data_packet(&source(), 63999, 0, 0, &[0; 512]).is_ok());
    }

    #[test]
    fn cids_read_and_print_as_uuids() {
        let cid: Cid = "01020304-0506-0708-090a-0b0c0d0e0f10".parse().unwrap();
        assert_eq!(cid, source().cid());
        assert_eq!(cid.to_string(), "01020304-0506-0708-090a-0b0c0d0e0f10");
        assert_eq!(
            "0102030405060708090A0B0C0D0E0F10".parse::<Cid>().unwrap(),
            cid
        );
        assert!("0102".parse::<Cid>().is_err());
        assert!(
            "zz020304-0506-0708-090a-0b0c0d0e0f10"
                .parse::<Cid>()
                .is_err()
        );
    }

    #[test]
    fn random_cids_are_version_4_and_differ() {
        let a = Cid::random();
        let b = Cid::random();
        assert_ne!(a, b);
        assert_eq!(a.0[6] >> 4, 4);
        assert_eq!(a.0[8] >> 6, 0b10);
    }

    #[test]
    fn multicast_groups_follow_the_universe() {
        assert_eq!(multicast_group(1), std::net::Ipv4Addr::new(239, 255, 0, 1));
        assert_eq!(
            multicast_group(0x1234),
            std::net::Ipv4Addr::new(239, 255, 0x12, 0x34)
        );
    }
}
