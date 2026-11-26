//! Packets byte for byte against reference packets from an independent implementation (`tests/vectors/`).

use pixel_cast::sacn::{self, Cid, Source, options};

fn reference(hex: &str) -> Vec<u8> {
    let digits: Vec<u8> = hex.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn cid() -> Cid {
    "01020304-0506-0708-090a-0b0c0d0e0f10".parse().unwrap()
}

#[test]
fn a_full_universe() {
    let source = Source::new(Cid([0; 16]), "pixel-cast", 100).unwrap();
    let values: Vec<u8> = (0..510).map(|i| (i & 255) as u8).collect();
    let packet = sacn::data_packet(&source, 1, 0, 0, &values).unwrap();
    assert_eq!(packet, reference(include_str!("vectors/full-universe.hex")));
}

#[test]
fn a_short_terminated_universe() {
    let source = Source::new(cid(), "pixel-cast test", 150).unwrap();
    let values: Vec<u8> = (0..144).map(|i| ((i * 7) & 255) as u8).collect();
    let packet = sacn::data_packet(&source, 258, 200, options::STREAM_TERMINATED, &values).unwrap();
    assert_eq!(
        packet,
        reference(include_str!("vectors/last-universe-terminated.hex"))
    );
}

#[test]
fn a_utf8_name_on_the_last_universe() {
    let source = Source::new(cid(), "Écran № 1", 0).unwrap();
    let packet = sacn::data_packet(&source, 63999, 255, options::PREVIEW_DATA, &[]).unwrap();
    assert_eq!(
        packet,
        reference(include_str!("vectors/utf8-name-max-universe.hex"))
    );
}
